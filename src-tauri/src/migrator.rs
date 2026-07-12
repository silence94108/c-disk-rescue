use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::cleaner::{locking_apps, sample_files, who_locks};
use crate::rules::{expand_pattern, load_rules};

#[derive(Default)]
pub struct MigrateState {
    pub running: AtomicBool,
    pub cancel: AtomicBool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TargetDisk {
    mount_point: String,
    free_bytes: u64,
    is_ntfs: bool,
    recommended: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MigrateProgress {
    copied_bytes: u64,
    total_bytes: u64,
    current_file: String,
}

/// 迁移历史(migrate-history.json 条目),也是「已搬家管理页」的数据源
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MigrateRecord {
    pub rule_id: String,
    pub display_name: String,
    /// 原位置(现为 junction)
    pub src: String,
    /// 数据现在的实际位置
    pub dst: String,
    /// 源目录改名后的备份(确认后删除,置 None)
    pub bak: Option<String>,
    pub bytes: u64,
    pub file_count: u64,
    pub at: String,
}

/// 事务日志(migrate-pending.json):断电/强关后启动时据此回滚(需求文档 §7)
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PendingTx {
    step: String, // copying | renaming | linking
    src: String,
    dst: String,
    bak: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MigrateResult {
    moved_bytes: u64,
    file_count: u64,
    dst: String,
}

/// 迁移期间阻止系统睡眠(笔记本合盖比手动关机常见得多,需求文档 F3)。
/// SetThreadExecutionState 是线程级调用,本 guard 必须建在执行复制的工作线程上;
/// RAII Drop 恢复,任何提前 return 都不会漏。
struct KeepAwake;

impl KeepAwake {
    fn new() -> Self {
        use windows_sys::Win32::System::Power::{
            SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED,
        };
        unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) };
        KeepAwake
    }
}

impl Drop for KeepAwake {
    fn drop(&mut self) {
        use windows_sys::Win32::System::Power::{SetThreadExecutionState, ES_CONTINUOUS};
        unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
    }
}

fn volume_is_ntfs(mount_point: &str) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetVolumeInformationW;
    // 根路径必须带尾反斜杠("D:\")
    let root = if mount_point.ends_with('\\') {
        mount_point.to_string()
    } else {
        format!("{mount_point}\\")
    };
    let wide: Vec<u16> = std::ffi::OsStr::new(&root)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut fs_name = [0u16; 32];
    let ok = unsafe {
        GetVolumeInformationW(
            wide.as_ptr(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            fs_name.as_mut_ptr(),
            fs_name.len() as u32,
        )
    };
    if ok == 0 {
        return false;
    }
    let len = fs_name.iter().position(|&c| c == 0).unwrap_or(0);
    String::from_utf16_lossy(&fs_name[..len]).eq_ignore_ascii_case("NTFS")
}

/// 候选目标盘:排除系统盘与可移动盘(拔盘后软件异常),校验 NTFS
/// (FAT32 单文件 4GB 上限、exFAT 无 ACL,均拒绝——需求文档 F3)
#[tauri::command]
pub fn get_migrate_targets() -> Vec<TargetDisk> {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let sys_drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
    let mut out: Vec<TargetDisk> = disks
        .list()
        .iter()
        .filter_map(|d| {
            let mount = d.mount_point().to_string_lossy().trim_end_matches('\\').to_string();
            if mount.eq_ignore_ascii_case(&sys_drive) || d.is_removable() {
                return None;
            }
            Some(TargetDisk {
                is_ntfs: volume_is_ntfs(&mount),
                mount_point: mount,
                free_bytes: d.available_space(),
                recommended: false,
            })
        })
        .collect();
    // 推荐剩余空间最大的 NTFS 盘
    if let Some(best) = out
        .iter_mut()
        .filter(|t| t.is_ntfs)
        .max_by_key(|t| t.free_bytes)
    {
        best.recommended = true;
    }
    out.sort_by(|a, b| b.free_bytes.cmp(&a.free_bytes));
    out
}

/// 遍历统计(文件数,逻辑字节)。校验口径必须是逻辑大小:实占会因源/目标盘
/// 压缩设置不同而不等,用它校验会出现「复制完美却报失败」的假阳性。
/// 遇 reparse point 直接拒绝——迁移含链接的目录再在里面建链接,行为不可预期。
fn count_tree(root: &Path) -> Result<(u64, u64), String> {
    let (mut files, mut bytes) = (0u64, 0u64);
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let read = std::fs::read_dir(&d).map_err(|e| format!("读取 {} 失败:{e}", d.display()))?;
        for entry in read {
            let entry = entry.map_err(|e| format!("枚举 {} 失败:{e}", d.display()))?;
            let ft = entry.file_type().map_err(|e| e.to_string())?;
            if ft.is_symlink() {
                return Err("这个目录里有特殊链接,暂不支持搬家".into());
            } else if ft.is_dir() {
                stack.push(entry.path());
            } else {
                files += 1;
                bytes += entry.metadata().map_err(|e| e.to_string())?.len();
            }
        }
    }
    Ok((files, bytes))
}

struct CopyCtx<'a> {
    app: &'a AppHandle,
    cancel: &'a AtomicBool,
    copied: u64,
    total: u64,
    last_emit: Instant,
}

/// 递归复制。fs::copy 底层是 CopyFileExW,内容、时间戳与 NTFS 备用数据流(ADS)
/// 一并保留(需求文档 F3 执行流程③)。
fn copy_tree(src: &Path, dst: &Path, ctx: &mut CopyCtx) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| format!("创建 {} 失败:{e}", dst.display()))?;
    let read = std::fs::read_dir(src).map_err(|e| format!("读取 {} 失败:{e}", src.display()))?;
    for entry in read {
        if ctx.cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        let entry = entry.map_err(|e| e.to_string())?;
        let ft = entry.file_type().map_err(|e| e.to_string())?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if ft.is_symlink() {
            // count_tree 已挡住;此处为防御性第二道闸
            return Err("这个目录里有特殊链接,暂不支持搬家".into());
        } else if ft.is_dir() {
            copy_tree(&from, &to, ctx)?;
        } else {
            let n = std::fs::copy(&from, &to)
                .map_err(|e| format!("复制 {} 失败:{e}", from.display()))?;
            ctx.copied += n;
            if ctx.last_emit.elapsed().as_millis() >= 100 {
                let _ = ctx.app.emit(
                    "migrate:progress",
                    MigrateProgress {
                        copied_bytes: ctx.copied,
                        total_bytes: ctx.total,
                        current_file: from.to_string_lossy().into_owned(),
                    },
                );
                ctx.last_emit = Instant::now();
            }
        }
    }
    Ok(())
}

fn data_file(app: &AppHandle, name: &str) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join(name))
}

fn load_history(app: &AppHandle) -> Vec<MigrateRecord> {
    let Ok(path) = data_file(app, "migrate-history.json") else { return Vec::new() };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_history(app: &AppHandle, records: &[MigrateRecord]) -> Result<(), String> {
    let path = data_file(app, "migrate-history.json")?;
    let json = serde_json::to_string_pretty(records).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

/// 本工具已记录的搬家源路径(小写),供 scan 排除「已在工具记录里」的 junction
pub(crate) fn history_srcs(app: &AppHandle) -> Vec<String> {
    load_history(app)
        .into_iter()
        .map(|r| r.src.to_lowercase())
        .collect()
}

fn write_pending(app: &AppHandle, tx: &PendingTx) -> Result<(), String> {
    let path = data_file(app, "migrate-pending.json")?;
    let json = serde_json::to_string(tx).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

fn clear_pending(app: &AppHandle) {
    if let Ok(path) = data_file(app, "migrate-pending.json") {
        let _ = std::fs::remove_file(path);
    }
}

/// 容错递归删除(回滚副本/清理 .bak 用):失败文件跳过,返回是否全部删净
fn best_effort_remove(root: &Path) -> bool {
    fn walk(dir: &Path, clean: &mut bool) {
        let Ok(read) = std::fs::read_dir(dir) else {
            *clean = false;
            return;
        };
        for entry in read.flatten() {
            let Ok(ft) = entry.file_type() else {
                *clean = false;
                continue;
            };
            let p = entry.path();
            if ft.is_symlink() {
                // 红线:只摘链接本身,永不跟入
                if std::fs::remove_dir(&p).or_else(|_| std::fs::remove_file(&p)).is_err() {
                    *clean = false;
                }
            } else if ft.is_dir() {
                walk(&p, clean);
                if std::fs::remove_dir(&p).is_err() {
                    *clean = false;
                }
            } else if std::fs::remove_file(&p).is_err() {
                *clean = false;
            }
        }
    }
    let mut clean = true;
    walk(root, &mut clean);
    clean && std::fs::remove_dir(root).is_ok()
}

#[tauri::command]
pub async fn start_migrate(
    app: AppHandle,
    rule_id: String,
    target_root: String,
) -> Result<MigrateResult, String> {
    {
        let state = app.state::<MigrateState>();
        if state.running.swap(true, Ordering::SeqCst) {
            return Err("已有搬家任务在进行".into());
        }
        state.cancel.store(false, Ordering::SeqCst);
    }
    let app2 = app.clone();
    let result =
        tauri::async_runtime::spawn_blocking(move || do_migrate(&app2, &rule_id, &target_root))
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r);
    app.state::<MigrateState>().running.store(false, Ordering::SeqCst);
    result
}

/// 解析搬家源:pick: 前缀走候选白名单(优化方案 §3.4),否则走知识库(现状)。
/// 返回(源路径、展示名、写入历史记录的 rule_id)。二者都不接受前端直接传路径。
fn resolve_migrate_source(
    app: &AppHandle,
    rule_id: &str,
) -> Result<(PathBuf, String, String), String> {
    if rule_id.starts_with("pick:") {
        // 缓存态自愈:内存白名单空(重启后)时从盘加载再解析(优化方案)
        let path = crate::scan::resolve_candidate_path(app, rule_id)
            .ok_or("这个文件夹不在本次识别的可搬列表里,重新体检一下再试")?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "所选文件夹".into());
        Ok((path, name, rule_id.to_string()))
    } else {
        let rule = load_rules()
            .into_iter()
            .find(|r| r.id == rule_id && r.action == "migrate")
            .ok_or("未知的搬家项")?;
        let src = expand_pattern(&rule.path_pattern).ok_or("路径解析失败")?;
        Ok((src, rule.display_name, rule.id))
    }
}

fn do_migrate(app: &AppHandle, rule_id: &str, target_root: &str) -> Result<MigrateResult, String> {
    let state = app.state::<MigrateState>();

    // 白名单原则:路径只从知识库或候选缓存取,不接受前端传路径(接口形状即安全边界)。
    // pick: 前缀 = 自选搬家候选,从 ScanState.candidates 白名单查真实路径(优化方案 §3.4)。
    let (src, display_name, rec_rule_id) = resolve_migrate_source(app, rule_id)?;
    if !src.is_dir() {
        return Err("这个目录不存在,可能软件已卸载".into());
    }
    let meta = std::fs::symlink_metadata(&src).map_err(|e| e.to_string())?;
    if meta.file_type().is_symlink() {
        return Err("这个目录已经搬过家了".into());
    }

    // 目标盘三重校验:在候选集内(非系统/非可移动)、NTFS、空间充足
    let targets = get_migrate_targets();
    let target = targets
        .iter()
        .find(|t| t.mount_point.eq_ignore_ascii_case(target_root))
        .ok_or("目标盘不可用(不能是 C 盘、U 盘或移动硬盘)")?;
    if !target.is_ntfs {
        return Err("目标盘不是 NTFS 格式,搬过去软件会出问题,换一个盘吧".into());
    }

    let (files, bytes) = count_tree(&src)?;
    if bytes == 0 {
        return Err("这个目录是空的,不需要搬家".into());
    }
    if target.free_bytes < bytes + bytes / 20 {
        return Err("目标盘剩余空间不够,换一个盘或先清理目标盘".into());
    }

    // 占用检查:迁移期间源目录必须无人写入
    let locked = who_locks(&sample_files(&src, 64));
    if !locked.is_empty() {
        return Err(format!("请先退出 {}", locked.join("、")));
    }

    let name = src.file_name().ok_or("路径异常")?.to_string_lossy().into_owned();
    let dst = PathBuf::from(format!("{target_root}\\AppDataMove")).join(&name);
    let bak = src.with_file_name(format!("{name}.bak"));
    if dst.exists() {
        return Err(format!("目标位置已有同名目录({}),先处理它再搬", dst.display()));
    }
    if bak.exists() {
        return Err("发现上次搬家留下的备份还没清理,先在「已搬家」页处理".into());
    }

    // 迁移期间阻止睡眠;guard 建在本工作线程(SetThreadExecutionState 线程级)
    let _awake = KeepAwake::new();

    // ① 复制(源数据全程未动,失败只需删副本)
    let tx = PendingTx {
        step: "copying".into(),
        src: src.to_string_lossy().into_owned(),
        dst: dst.to_string_lossy().into_owned(),
        bak: bak.to_string_lossy().into_owned(),
    };
    write_pending(app, &tx)?;
    let mut ctx = CopyCtx {
        app,
        cancel: &state.cancel,
        copied: 0,
        total: bytes,
        last_emit: Instant::now(),
    };
    if let Err(e) = copy_tree(&src, &dst, &mut ctx) {
        best_effort_remove(&dst);
        clear_pending(app);
        return Err(if e == "cancelled" {
            "已取消,你的数据没有任何变化".into()
        } else {
            e
        });
    }

    // ② 双校验:文件数 + 总字节数(需求文档 F3 执行流程④)
    let (dst_files, dst_bytes) = count_tree(&dst)?;
    if dst_files != files || dst_bytes != bytes {
        best_effort_remove(&dst);
        clear_pending(app);
        return Err("复制结果校验没通过,已撤销,你的数据没有任何变化。可能有软件正在写入,退干净后再试".into());
    }

    // ③ 源目录改名为 .bak(同盘 rename 原子)
    write_pending(app, &PendingTx { step: "renaming".into(), ..tx.clone() })?;
    if let Err(e) = std::fs::rename(&src, &bak) {
        best_effort_remove(&dst);
        clear_pending(app);
        return Err(format!("源目录改名失败({e}),已撤销,你的数据没有任何变化。可能有软件占用,退干净后再试"));
    }

    // ④ 原位置建 junction
    write_pending(app, &PendingTx { step: "linking".into(), ..tx.clone() })?;
    if let Err(e) = junction::create(&dst, &src) {
        // 逆序回退:名字改回去、副本删掉,回到原状
        let _ = std::fs::rename(&bak, &src);
        best_effort_remove(&dst);
        clear_pending(app);
        return Err(format!("建立联接失败({e}),已恢复原状,你的数据没有任何变化"));
    }

    // ⑤ 落历史,事务完结
    let mut history = load_history(app);
    history.push(MigrateRecord {
        rule_id: rec_rule_id,
        display_name,
        src: tx.src,
        dst: tx.dst.clone(),
        bak: Some(tx.bak),
        bytes,
        file_count: files,
        at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    });
    save_history(app, &history)?;
    clear_pending(app);

    Ok(MigrateResult { moved_bytes: bytes, file_count: files, dst: tx.dst })
}

#[tauri::command]
pub fn cancel_migrate(state: State<'_, MigrateState>) {
    if state.running.load(Ordering::SeqCst) {
        state.cancel.store(true, Ordering::SeqCst);
    }
}

#[tauri::command]
pub fn get_migrations(app: AppHandle) -> Vec<MigrateRecord> {
    load_history(&app)
}

/// 用户启动软件确认正常后,删除 .bak 释放 C 盘空间(需求文档 F3 执行流程⑧)
#[tauri::command]
pub async fn confirm_migration(app: AppHandle, rule_id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut history = load_history(&app);
        let rec = history
            .iter_mut()
            .find(|r| r.rule_id == rule_id && r.bak.is_some())
            .ok_or("没有找到待确认的搬家记录")?;
        let bak = PathBuf::from(rec.bak.as_ref().unwrap());
        // 只删历史记录过的 .bak 路径,且路径名必须以 .bak 结尾(双重防呆)
        if !bak.to_string_lossy().ends_with(".bak") {
            return Err("备份路径异常,拒绝删除".into());
        }
        if bak.exists() && !best_effort_remove(&bak) {
            return Err("备份删除未完成(部分文件被占用),稍后再试".into());
        }
        rec.bak = None;
        save_history(&app, &history)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 启动时检查未完成事务:断电/强关后据事务日志逆序回滚,永远回到「原状」。
/// 因源数据在 copy 阶段全程未动,回滚只是删副本/改回名(需求文档 F3)。
#[tauri::command]
pub fn recover_pending_migration(app: AppHandle) -> Result<Option<String>, String> {
    let path = data_file(&app, "migrate-pending.json")?;
    let Ok(raw) = std::fs::read_to_string(&path) else { return Ok(None) };
    let Ok(tx) = serde_json::from_str::<PendingTx>(&raw) else {
        let _ = std::fs::remove_file(&path);
        return Ok(None);
    };
    let src = PathBuf::from(&tx.src);
    let dst = PathBuf::from(&tx.dst);
    let bak = PathBuf::from(&tx.bak);

    // ---- 还原事务的恢复(bak 字段此时存放 .restoring 临时目录) ----
    if tx.step == "restore-copying" {
        // 链接未动,删掉复制到一半的临时目录即回到原状
        if bak.exists() {
            best_effort_remove(&bak);
        }
        let _ = std::fs::remove_file(&path);
        return Ok(Some("上次还原被打断,已自动清理,你的数据没有任何变化".into()));
    }
    if tx.step == "restore-swapping" {
        let src_is_link = std::fs::symlink_metadata(&src)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false);
        if src_is_link {
            // 摘链前中断:原状,删临时目录
            if bak.exists() {
                best_effort_remove(&bak);
            }
        } else if !src.exists() && bak.is_dir() {
            // 摘链后、就位前中断:把临时目录就位,完成还原
            std::fs::rename(&bak, &src).map_err(|e| format!("恢复目录就位失败:{e}"))?;
            if dst.exists() {
                best_effort_remove(&dst);
            }
            remove_history_by_src(&app, &tx.src);
        } else if src.is_dir() {
            // 已就位:收尾,删数据副本
            if dst.exists() {
                best_effort_remove(&dst);
            }
            remove_history_by_src(&app, &tx.src);
        }
        let _ = std::fs::remove_file(&path);
        return Ok(Some("上次还原被打断,已自动接续完成,数据完好".into()));
    }

    // ---- 迁移事务的恢复 ----
    // linking 阶段中断且链接已建成:事务实际已完成,补写历史即可
    let src_meta = std::fs::symlink_metadata(&src);
    let src_is_link = src_meta.as_ref().map(|m| m.file_type().is_symlink()).unwrap_or(false);
    if tx.step == "linking" && src_is_link {
        let mut history = load_history(&app);
        if !history.iter().any(|r| r.src == tx.src) {
            let (files, bytes) = count_tree(&dst).unwrap_or((0, 0));
            history.push(MigrateRecord {
                rule_id: String::new(),
                display_name: src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                src: tx.src.clone(),
                dst: tx.dst.clone(),
                bak: bak.exists().then(|| tx.bak.clone()),
                bytes,
                file_count: files,
                at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            });
            let _ = save_history(&app, &history);
        }
        let _ = std::fs::remove_file(&path);
        return Ok(Some("上次搬家在最后一步被打断,已确认搬家其实完成了,数据完好".into()));
    }

    // 其余情况逆序回滚:摘链接(若有)→ 名字改回 → 删副本
    if src_is_link {
        let _ = junction::delete(&src);
    }
    if !src.exists() && bak.exists() {
        std::fs::rename(&bak, &src).map_err(|e| format!("恢复原目录名失败:{e}"))?;
    }
    if dst.exists() {
        best_effort_remove(&dst);
    }
    let _ = std::fs::remove_file(&path);
    Ok(Some("上次搬家被打断,已自动恢复原状,你的数据没有任何变化".into()))
}

fn remove_history_by_src(app: &AppHandle, src: &str) {
    let mut history = load_history(app);
    let before = history.len();
    history.retain(|r| r.src != src);
    if history.len() != before {
        let _ = save_history(app, &history);
    }
}

/// 「帮我退出」:向锁定进程的可见主窗口发 WM_CLOSE(等同用户点 ×),
/// 轮询等待文件锁释放。绝不 TerminateProcess——强杀正在写数据库的进程
/// 会损坏用户数据(需求文档 F2 红线)。返回超时后仍在锁定的软件名(空 = 成功)。
#[tauri::command]
pub async fn request_close(app: AppHandle, rule_id: String) -> Result<Vec<String>, String> {
    // pick: 候选路径先在白名单里查好(内存空则从盘自愈),再进阻塞线程(State 不跨线程)
    let pick_path = if rule_id.starts_with("pick:") {
        Some(
            crate::scan::resolve_candidate_path(&app, &rule_id)
                .ok_or("这个文件夹不在本次识别的可搬列表里")?,
        )
    } else {
        None
    };
    tauri::async_runtime::spawn_blocking(move || {
        let path = match pick_path {
            Some(p) => p,
            None => {
                let rule = load_rules()
                    .into_iter()
                    .find(|r| r.id == rule_id)
                    .ok_or("未知的项目")?;
                expand_pattern(&rule.path_pattern).ok_or("路径解析失败")?
            }
        };
        if !path.is_dir() {
            return Ok(Vec::new());
        }
        let samples = sample_files(&path, 64);
        let apps = locking_apps(&samples);
        if apps.is_empty() {
            return Ok(Vec::new());
        }
        let pids: Vec<u32> = apps.iter().map(|a| a.pid).collect();
        post_close_to_windows(&pids);
        // 优雅退出需要时间(保存状态、写库收尾),轮询至多 10 秒
        for _ in 0..12 {
            std::thread::sleep(std::time::Duration::from_millis(800));
            if who_locks(&samples).is_empty() {
                return Ok(Vec::new());
            }
        }
        Ok(who_locks(&samples))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 向属于目标进程的可见顶层窗口投递 WM_CLOSE
fn post_close_to_windows(pids: &[u32]) {
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_CLOSE,
    };
    struct Ctx<'a> {
        pids: &'a [u32],
        hwnds: Vec<isize>,
    }
    unsafe extern "system" fn cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let ctx = &mut *(lparam as *mut Ctx);
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid != 0 && ctx.pids.contains(&pid) && IsWindowVisible(hwnd) != 0 {
            ctx.hwnds.push(hwnd as isize);
        }
        1
    }
    let mut ctx = Ctx { pids, hwnds: Vec::new() };
    unsafe {
        EnumWindows(Some(cb), &mut ctx as *mut Ctx as LPARAM);
        for h in &ctx.hwnds {
            PostMessageW(*h as HWND, WM_CLOSE, 0, 0);
        }
    }
}

/// 还原(搬回 C 盘):软件更新异常时的自救通道(需求文档 F3)。
/// 复用迁移的事务化结构(copy→校验→摘链→就位),动手前校验 C 盘剩余空间——
/// 典型事故:迁走 20GB 后 C 盘又被填满,还原中途空间不足把数据劈成两半。
#[tauri::command]
pub async fn revert_migration(app: AppHandle, rule_id: String) -> Result<(), String> {
    {
        let state = app.state::<MigrateState>();
        if state.running.swap(true, Ordering::SeqCst) {
            return Err("已有搬家任务在进行".into());
        }
        state.cancel.store(false, Ordering::SeqCst);
    }
    let app2 = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || do_revert(&app2, &rule_id))
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r);
    app.state::<MigrateState>().running.store(false, Ordering::SeqCst);
    result
}

fn do_revert(app: &AppHandle, rule_id: &str) -> Result<(), String> {
    let state = app.state::<MigrateState>();
    let history = load_history(app);
    let rec = history
        .iter()
        .find(|r| r.rule_id == rule_id)
        .ok_or("没有找到这条搬家记录")?
        .clone();
    let src = PathBuf::from(&rec.src);
    let dst = PathBuf::from(&rec.dst);

    // 现场校验:原位置必须仍是联接,数据目录必须还在
    let src_is_link = std::fs::symlink_metadata(&src)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);
    if !src_is_link {
        return Err("原位置已经不是联接,可能已经手动还原过了".into());
    }
    if !dst.is_dir() {
        return Err("搬过去的数据目录找不到了,无法还原".into());
    }

    let (files, bytes) = count_tree(&dst)?;

    // C 盘剩余空间校验(需求文档 F3 还原)
    let sys_drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let c_free = disks
        .list()
        .iter()
        .find(|d| {
            d.mount_point()
                .to_string_lossy()
                .trim_end_matches('\\')
                .eq_ignore_ascii_case(&sys_drive)
        })
        .map(|d| d.available_space())
        .unwrap_or(0);
    if c_free < bytes + bytes / 20 {
        return Err(format!(
            "C 盘剩余空间不够装回这 {:.1}GB 数据,先清理 C 盘再还原",
            bytes as f64 / 1073741824.0
        ));
    }

    // 占用检查:还原期间数据目录必须无人写入
    let locked = who_locks(&sample_files(&dst, 64));
    if !locked.is_empty() {
        return Err(format!("请先退出 {}", locked.join("、")));
    }

    let _awake = KeepAwake::new();
    let name = src.file_name().ok_or("路径异常")?.to_string_lossy().into_owned();
    let tmp = src.with_file_name(format!("{name}.restoring"));
    if tmp.exists() {
        best_effort_remove(&tmp);
    }

    // ① 数据复制回 C 盘临时目录(此阶段联接与数据均未动)
    // PendingTx.bak 字段在还原事务中存放 .restoring 临时目录
    let tx = PendingTx {
        step: "restore-copying".into(),
        src: rec.src.clone(),
        dst: rec.dst.clone(),
        bak: tmp.to_string_lossy().into_owned(),
    };
    write_pending(app, &tx)?;
    let mut ctx = CopyCtx {
        app,
        cancel: &state.cancel,
        copied: 0,
        total: bytes,
        last_emit: Instant::now(),
    };
    if let Err(e) = copy_tree(&dst, &tmp, &mut ctx) {
        best_effort_remove(&tmp);
        clear_pending(app);
        return Err(if e == "cancelled" {
            "已取消,你的数据没有任何变化".into()
        } else {
            e
        });
    }

    // ② 双校验
    let (tmp_files, tmp_bytes) = count_tree(&tmp)?;
    if tmp_files != files || tmp_bytes != bytes {
        best_effort_remove(&tmp);
        clear_pending(app);
        return Err("复制结果校验没通过,已撤销,你的数据没有任何变化。可能有软件正在写入,退干净后再试".into());
    }

    // ③ 摘联接 → 临时目录就位(两个毫秒级操作,事务日志护住中断窗口)
    write_pending(app, &PendingTx { step: "restore-swapping".into(), ..tx.clone() })?;
    if let Err(e) = junction::delete(&src) {
        best_effort_remove(&tmp);
        clear_pending(app);
        return Err(format!("摘除联接失败({e}),已撤销,你的数据没有任何变化"));
    }
    // junction::delete 只摘 reparse 数据,残留空目录须移除后才能 rename 就位
    let _ = std::fs::remove_dir(&src);
    if let Err(e) = std::fs::rename(&tmp, &src) {
        // 回滚:重建联接,恢复「已搬家」状态
        let _ = junction::create(&dst, &src);
        best_effort_remove(&tmp);
        clear_pending(app);
        return Err(format!("数据就位失败({e}),已恢复原状,软件仍可正常使用"));
    }

    // ④ 收尾:删 D 盘数据副本与残留 .bak,移除历史记录
    best_effort_remove(&dst);
    if let Some(bak) = &rec.bak {
        let b = PathBuf::from(bak);
        if b.exists() {
            best_effort_remove(&b);
        }
    }
    remove_history_by_src(app, &rec.src);
    clear_pending(app);
    // 撤回的目录重新占用 C 盘:把刚统计到的精确大小补进扫描快照的
    // 撤回补丁表,「可搬家」列表即刻恢复它(重新体检后此表作废)
    if let Ok(mut map) = app.state::<crate::scan::ScanState>().reverted.lock() {
        map.insert(rec.src.to_lowercase(), bytes);
    }
    Ok(())
}

/// 搬回一个「外部 junction」(非本工具搬的、但指向别的盘的已搬走目录,用户需求)。
/// 复用与 do_revert 相同的事务结构(copy→校验→摘链→就位),但源来自 scan 的
/// external_junctions 白名单、目标现场从 read_link 取,不涉及 history 与 .bak。
#[tauri::command]
pub async fn revert_external_junction(app: AppHandle, src: String) -> Result<(), String> {
    {
        let state = app.state::<MigrateState>();
        if state.running.swap(true, Ordering::SeqCst) {
            return Err("已有搬家任务在进行".into());
        }
        state.cancel.store(false, Ordering::SeqCst);
    }
    let app2 = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || do_revert_external(&app2, &src))
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r);
    app.state::<MigrateState>().running.store(false, Ordering::SeqCst);
    result
}

fn do_revert_external(app: &AppHandle, src_str: &str) -> Result<(), String> {
    let state = app.state::<MigrateState>();

    // 白名单校验:必须是本次识别的外部 junction(接口形状即安全边界,不接受任意路径)
    let src = {
        let scan_state = app.state::<crate::scan::ScanState>();
        let wl = scan_state
            .external_junctions
            .lock()
            .map_err(|e| e.to_string())?;
        let lower = src_str.to_lowercase();
        wl.iter()
            .find(|p| p.to_string_lossy().to_lowercase() == lower)
            .cloned()
    }
    .ok_or("这个目录不在本次识别的可搬回列表里,刷新一下再试")?;

    // 现场校验:原位置必须仍是联接,目标数据目录必须还在
    let src_is_link = std::fs::symlink_metadata(&src)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);
    if !src_is_link {
        return Err("这个位置已经不是联接了,可能已经处理过".into());
    }
    let raw = std::fs::read_link(&src).map_err(|e| format!("读取联接目标失败:{e}"))?;
    let dst = crate::scan::normalize_junction_target(&raw);
    if !dst.is_dir() {
        return Err("搬过去的数据目录找不到了,无法搬回".into());
    }

    let (files, bytes) = count_tree(&dst)?;

    // C 盘剩余空间校验(同 do_revert:防搬回中途空间不足把数据劈两半)
    let sys_drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let c_free = disks
        .list()
        .iter()
        .find(|d| {
            d.mount_point()
                .to_string_lossy()
                .trim_end_matches('\\')
                .eq_ignore_ascii_case(&sys_drive)
        })
        .map(|d| d.available_space())
        .unwrap_or(0);
    if c_free < bytes + bytes / 20 {
        return Err(format!(
            "C 盘剩余空间不够装回这 {:.1}GB 数据,先清理 C 盘再搬回",
            bytes as f64 / 1073741824.0
        ));
    }

    // 占用检查:搬回期间目标数据目录必须无人写入
    let locked = who_locks(&sample_files(&dst, 64));
    if !locked.is_empty() {
        return Err(format!("请先退出 {}", locked.join("、")));
    }

    let _awake = KeepAwake::new();
    let name = src.file_name().ok_or("路径异常")?.to_string_lossy().into_owned();
    let tmp = src.with_file_name(format!("{name}.restoring"));
    if tmp.exists() {
        best_effort_remove(&tmp);
    }

    // ① 数据复制回 C 盘临时目录(联接与数据均未动;事务日志复用 restore-* 步骤,
    // 断电恢复由 recover_pending_migration 统一兜住)
    let tx = PendingTx {
        step: "restore-copying".into(),
        src: src.to_string_lossy().into_owned(),
        dst: dst.to_string_lossy().into_owned(),
        bak: tmp.to_string_lossy().into_owned(),
    };
    write_pending(app, &tx)?;
    let mut ctx = CopyCtx {
        app,
        cancel: &state.cancel,
        copied: 0,
        total: bytes,
        last_emit: Instant::now(),
    };
    if let Err(e) = copy_tree(&dst, &tmp, &mut ctx) {
        best_effort_remove(&tmp);
        clear_pending(app);
        return Err(if e == "cancelled" {
            "已取消,你的数据没有任何变化".into()
        } else {
            e
        });
    }

    // ② 双校验
    let (tf, tb) = count_tree(&tmp)?;
    if tf != files || tb != bytes {
        best_effort_remove(&tmp);
        clear_pending(app);
        return Err("复制结果校验没通过,已撤销,你的数据没有任何变化。可能有软件正在写入,退干净后再试".into());
    }

    // ③ 摘联接 → 临时目录就位
    write_pending(app, &PendingTx { step: "restore-swapping".into(), ..tx.clone() })?;
    if let Err(e) = junction::delete(&src) {
        best_effort_remove(&tmp);
        clear_pending(app);
        return Err(format!("摘除联接失败({e}),已撤销,你的数据没有任何变化"));
    }
    let _ = std::fs::remove_dir(&src);
    if let Err(e) = std::fs::rename(&tmp, &src) {
        let _ = junction::create(&dst, &src);
        best_effort_remove(&tmp);
        clear_pending(app);
        return Err(format!("数据就位失败({e}),已恢复原状,软件仍可正常使用"));
    }

    // ④ 收尾:删别的盘的数据副本;不碰 history(外部 junction 不在记录里)
    best_effort_remove(&dst);
    clear_pending(app);
    if let Ok(mut wl) = app.state::<crate::scan::ScanState>().external_junctions.lock() {
        wl.retain(|p| p != &src);
    }
    Ok(())
}
