use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::rules::{expand_pattern, load_rules, Rule};
use crate::scan::allocated_size;

#[derive(Default)]
pub struct CleanState {
    pub running: AtomicBool,
    pub cancel: AtomicBool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CleanableItem {
    rule_id: String,
    display_name: String,
    explain: String,
    /// safe | cost | caution
    risk: String,
    needs_admin: bool,
    /// 引导型:工具不代删,explain 教手动操作;前端默认不勾且禁用勾选
    guide_only: bool,
    path: String,
    size_bytes: u64,
    file_count: u64,
    /// 正在锁定该项文件的软件友好名(Restart Manager 检出),非空则前端置灰并提示退出
    locked_by: Vec<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CleanablesReport {
    items: Vec<CleanableItem>,
    /// 当前进程是否已提权,决定 needsAdmin 项能否执行
    is_elevated: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CleanProgress {
    rule_id: String,
    freed_bytes: u64,
    deleted_files: u64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SkippedRule {
    rule_id: String,
    reason: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CleanResult {
    freed_bytes: u64,
    deleted_files: u64,
    /// 被占用等原因删不掉而跳过的文件数(容错设计,不算失败)
    failed_files: u64,
    skipped: Vec<SkippedRule>,
    log_path: Option<String>,
}

fn to_wide(p: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    p.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
}

/// 当前进程是否以管理员运行(TokenElevation),决定权限矩阵中系统级清理项的可用性
pub fn is_elevated() -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elev = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut ret_len: u32 = 0;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut elev as *mut _ as *mut _,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret_len,
        );
        CloseHandle(token);
        ok != 0 && elev.TokenIsElevated != 0
    }
}

/// 极简 glob:仅支持 `*` 通配,name/pattern 需已小写(Windows 文件名不区分大小写)
fn glob_match(name: &str, pattern: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return name == pattern;
    }
    let mut rest = name;
    if !rest.starts_with(parts[0]) {
        return false;
    }
    rest = &rest[parts[0].len()..];
    let last = parts[parts.len() - 1];
    if rest.len() < last.len() || !rest.ends_with(last) {
        return false;
    }
    rest = &rest[..rest.len() - last.len()];
    for mid in &parts[1..parts.len() - 1] {
        if mid.is_empty() {
            continue;
        }
        match rest.find(mid) {
            Some(i) => rest = &rest[i + mid.len()..],
            None => return false,
        }
    }
    true
}

fn matches_patterns(name: &std::ffi::OsStr, patterns: Option<&[String]>) -> bool {
    match patterns {
        None => true,
        Some(pats) => {
            let lower = name.to_string_lossy().to_lowercase();
            pats.iter().any(|p| glob_match(&lower, &p.to_lowercase()))
        }
    }
}

/// 统计一个清理项的当前大小(实占口径,与删除统计一致)。
/// 遇 reparse point 不跟入(红线);读不了的目录静默跳过(needsAdmin 项普通权限下 size=0)。
pub(crate) fn measure_dir(dir: &Path, patterns: Option<&[String]>) -> (u64, u64) {
    let (mut bytes, mut files) = (0u64, 0u64);
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&d) else { continue };
        for entry in read.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            } else if ft.is_dir() {
                stack.push(entry.path());
            } else if matches_patterns(&entry.file_name(), patterns) {
                let Ok(meta) = entry.metadata() else { continue };
                use std::os::windows::fs::MetadataExt;
                bytes += allocated_size(&entry.path(), meta.len(), meta.file_attributes());
                files += 1;
            }
        }
    }
    (bytes, files)
}

/// FileRepository 子目录名 → 分组 key:`nv_dispi.inf_amd64_<hash>` → `nv_dispi.inf_amd64`。
/// 同 inf 同架构的多个 hash 目录 = 同一驱动的多个版本共存
fn driver_group_key(dir_name: &str) -> Option<String> {
    let lower = dir_name.to_lowercase();
    let pos = lower.find(".inf_")?;
    let rest = &lower[pos + 5..];
    let arch_end = rest.find('_')?;
    Some(lower[..pos + 5 + arch_end].to_string())
}

/// inf 是 UTF-16LE(带 BOM)或 ANSI,两种都要认
fn read_inf_text(p: &Path) -> Option<String> {
    let bytes = std::fs::read(p).ok()?;
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        let utf16: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        Some(String::from_utf16_lossy(&utf16))
    } else {
        Some(String::from_utf8_lossy(&bytes).into_owned())
    }
}

/// 版本比较 key:4 段版本号优先,同版本再比日期(y,m,d)
type DriverVerKey = (u64, u64, u64, u64, u32, u32, u32);

/// 解析 inf 的 `DriverVer = MM/DD/YYYY,a.b.c.d` 行
fn parse_driver_ver(content: &str) -> Option<DriverVerKey> {
    for line in content.lines() {
        let t = line.trim();
        if t.len() < 9 || !t[..9].eq_ignore_ascii_case("driverver") {
            continue;
        }
        let rest = t[9..].trim_start().strip_prefix('=')?.trim();
        let rest = rest.split(';').next().unwrap_or("").trim();
        let mut parts = rest.splitn(2, ',');
        let date = parts.next().unwrap_or("").trim();
        let ver = parts.next().unwrap_or("").trim();
        let mut dmy = date.split('/');
        let m: u32 = dmy.next()?.trim().parse().ok()?;
        let d: u32 = dmy.next()?.trim().parse().ok()?;
        let y: u32 = dmy.next()?.trim().parse().ok()?;
        let mut v = [0u64; 4];
        for (i, seg) in ver.split('.').take(4).enumerate() {
            v[i] = seg.trim().parse().unwrap_or(0);
        }
        return Some((v[0], v[1], v[2], v[3], y, m, d));
    }
    None
}

/// 「设备驱动程序包」的展示口径:同 inf 同架构多版本共存时,严格旧于最新版
/// 的目录才计入(cleanmgr「保留每驱动最新版」思路的近似)。仅用于预估展示,
/// 删除仍引导系统磁盘清理;组内任一版本读不出 DriverVer 则整组保守跳过
pub(crate) fn measure_old_driver_packages(repo: &Path) -> (u64, u64) {
    let mut groups: HashMap<String, Vec<PathBuf>> = HashMap::new();
    let Ok(read) = std::fs::read_dir(repo) else {
        return (0, 0);
    };
    for entry in read.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_dir() || ft.is_symlink() {
            continue;
        }
        if let Some(key) = driver_group_key(&entry.file_name().to_string_lossy()) {
            groups.entry(key).or_default().push(entry.path());
        }
    }
    let (mut bytes, mut files) = (0u64, 0u64);
    for (key, dirs) in groups {
        if dirs.len() < 2 {
            continue;
        }
        let Some(stem) = key.split(".inf_").next() else { continue };
        let inf_name = format!("{stem}.inf");
        let mut vers: Vec<(DriverVerKey, &PathBuf)> = Vec::new();
        for d in &dirs {
            match read_inf_text(&d.join(&inf_name)).as_deref().and_then(parse_driver_ver) {
                Some(v) => vers.push((v, d)),
                None => {
                    vers.clear();
                    break;
                }
            }
        }
        if vers.is_empty() {
            continue;
        }
        let newest = vers.iter().map(|(v, _)| *v).max().unwrap();
        for (v, d) in &vers {
            if *v < newest {
                let (b, f) = measure_dir(d, None);
                bytes += b;
                files += f;
            }
        }
    }
    (bytes, files)
}

/// 递归取样至多 limit 个文件,供 Restart Manager 注册。/// 浏览器进程锁定的是缓存子目录里的具体文件,故必须深入取样而非只看顶层。
pub(crate) fn sample_files(dir: &Path, limit: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        if out.len() >= limit {
            break;
        }
        let Ok(read) = std::fs::read_dir(&d) else { continue };
        for entry in read.flatten() {
            if out.len() >= limit {
                break;
            }
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            } else if ft.is_dir() {
                stack.push(entry.path());
            } else {
                out.push(entry.path());
            }
        }
    }
    out
}

pub(crate) struct LockingApp {
    pub pid: u32,
    pub name: String,
}

/// 用 Restart Manager 查询哪些进程锁定了这些文件(pid + 软件友好名)。
/// 按文件锁判定而非进程名匹配——软件跨版本会改进程名(需求文档 F2 前置检查)。
pub(crate) fn locking_apps(files: &[PathBuf]) -> Vec<LockingApp> {
    use windows_sys::Win32::Foundation::ERROR_MORE_DATA;
    use windows_sys::Win32::System::RestartManager::{
        RmEndSession, RmGetList, RmRegisterResources, RmStartSession, RM_PROCESS_INFO,
    };
    if files.is_empty() {
        return Vec::new();
    }
    let wides: Vec<Vec<u16>> = files.iter().map(|p| to_wide(p)).collect();
    let ptrs: Vec<*const u16> = wides.iter().map(|w| w.as_ptr()).collect();
    let mut out: Vec<LockingApp> = Vec::new();
    unsafe {
        let mut session: u32 = 0;
        // CCH_RM_SESSION_KEY(32) + 终止符
        let mut key = [0u16; 33];
        if RmStartSession(&mut session, 0, key.as_mut_ptr()) != 0 {
            return out;
        }
        if RmRegisterResources(
            session,
            ptrs.len() as u32,
            ptrs.as_ptr(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
        ) == 0
        {
            let mut needed: u32 = 0;
            let mut count: u32 = 16;
            let mut infos: [RM_PROCESS_INFO; 16] = std::mem::zeroed();
            let mut reasons: u32 = 0;
            let rc = RmGetList(session, &mut needed, &mut count, infos.as_mut_ptr(), &mut reasons);
            // ERROR_MORE_DATA:锁定进程超过 16 个,已写入的前 16 个足够展示
            if rc == 0 || rc == ERROR_MORE_DATA {
                for info in infos.iter().take(count.min(16) as usize) {
                    let len = info.strAppName.iter().position(|&c| c == 0).unwrap_or(0);
                    let name = String::from_utf16_lossy(&info.strAppName[..len]);
                    if !name.is_empty() {
                        out.push(LockingApp {
                            pid: info.Process.dwProcessId,
                            name,
                        });
                    }
                }
            }
        }
        RmEndSession(session);
    }
    out
}

/// 锁定软件的友好名去重列表(如「Google Chrome」),UI 展示用
pub(crate) fn who_locks(files: &[PathBuf]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for app in locking_apps(files) {
        if !names.contains(&app.name) {
            names.push(app.name);
        }
    }
    names
}

fn recycle_bin_status() -> (u64, u64) {
    use windows_sys::Win32::UI::Shell::{SHQueryRecycleBinW, SHQUERYRBINFO};
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into()) + "\\";
    let wide = to_wide(Path::new(&drive));
    unsafe {
        let mut info = SHQUERYRBINFO {
            cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32,
            i64Size: 0,
            i64NumItems: 0,
        };
        if SHQueryRecycleBinW(wide.as_ptr(), &mut info) == 0 {
            (info.i64Size.max(0) as u64, info.i64NumItems.max(0) as u64)
        } else {
            (0, 0)
        }
    }
}

fn empty_recycle_bin() -> Result<(), String> {
    use windows_sys::Win32::UI::Shell::SHEmptyRecycleBinW;
    const SHERB_NOCONFIRMATION: u32 = 0x1;
    const SHERB_NOPROGRESSUI: u32 = 0x2;
    const SHERB_NOSOUND: u32 = 0x4;
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into()) + "\\";
    let wide = to_wide(Path::new(&drive));
    let hr = unsafe {
        SHEmptyRecycleBinW(
            std::ptr::null_mut(),
            wide.as_ptr(),
            SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND,
        )
    };
    if hr == 0 {
        Ok(())
    } else {
        Err(format!("清空回收站失败(0x{hr:08X})"))
    }
}

#[tauri::command]
pub async fn scan_cleanables(app: AppHandle) -> Result<CleanablesReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _ = app; // 保持签名一致,统计不需要 app
        let elevated = is_elevated();
        let mut items = Vec::new();
        for rule in load_rules().into_iter().filter(|r| r.action == "clean") {
            let item = if rule.path_pattern == "special:recycle-bin" {
                let (bytes, count) = recycle_bin_status();
                CleanableItem {
                    rule_id: rule.id,
                    display_name: rule.display_name,
                    explain: rule.explain,
                    risk: rule.risk,
                    needs_admin: rule.needs_admin,
                    guide_only: rule.guide_only,
                    path: "回收站".into(),
                    size_bytes: bytes,
                    file_count: count,
                    locked_by: Vec::new(),
                }
            } else {
                let Some(path) = expand_pattern(&rule.path_pattern) else { continue };
                if !path.is_dir() {
                    continue;
                }
                let patterns = rule.file_patterns.as_deref();
                // 驱动仓库专用口径:只算旧版本部分,不算在用驱动
                let (bytes, files) = if rule.id == "driver-packages" {
                    measure_old_driver_packages(&path)
                } else {
                    measure_dir(&path, patterns)
                };
                // 占用检测只对有关联进程的项做(浏览器等);Temp 类靠删除时逐文件容错
                let locked_by = if !rule.related_processes.is_empty() && bytes > 0 {
                    who_locks(&sample_files(&path, 64))
                } else {
                    Vec::new()
                };
                CleanableItem {
                    rule_id: rule.id,
                    display_name: rule.display_name,
                    explain: rule.explain,
                    risk: rule.risk,
                    needs_admin: rule.needs_admin,
                    guide_only: rule.guide_only,
                    path: path.to_string_lossy().into_owned(),
                    size_bytes: bytes,
                    file_count: files,
                    locked_by,
                }
            };
            if item.size_bytes > 0 {
                items.push(item);
            }
        }
        items.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
        Ok(CleanablesReport { items, is_elevated: elevated })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LockStatus {
    rule_id: String,
    locked_by: Vec<String>,
}

/// 轻量复查:只做 Restart Manager 文件锁检测,不重新统计大小。
/// 报告页与搬家向导轮询用——用户退出软件后自动解锁/亮起下一步
/// (设计规范 §3.4「检测到进程退出后自动亮起」)。
#[tauri::command]
pub async fn check_locks(rule_ids: Vec<String>) -> Result<Vec<LockStatus>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut out = Vec::new();
        for rule in load_rules()
            .into_iter()
            .filter(|r| rule_ids.iter().any(|id| id == &r.id))
        {
            // 无关联进程的项(Temp 类)不做锁检测,与 scan_cleanables 口径一致
            if rule.related_processes.is_empty() {
                continue;
            }
            let Some(path) = expand_pattern(&rule.path_pattern) else { continue };
            if !path.is_dir() {
                continue;
            }
            out.push(LockStatus {
                rule_id: rule.id,
                locked_by: who_locks(&sample_files(&path, 64)),
            });
        }
        Ok(out)
    })
    .await
    .map_err(|e| e.to_string())?
}

struct CleanCtx<'a> {
    app: &'a AppHandle,
    cancel: &'a AtomicBool,
    rule_id: String,
    bytes: u64,
    files: u64,
    failed: u64,
    last_emit: Instant,
}

impl CleanCtx<'_> {
    fn tick(&mut self) {
        if self.last_emit.elapsed().as_millis() >= 100 {
            let _ = self.app.emit(
                "clean:progress",
                CleanProgress {
                    rule_id: self.rule_id.clone(),
                    freed_bytes: self.bytes,
                    deleted_files: self.files,
                },
            );
            self.last_emit = Instant::now();
        }
    }
}

/// 删除目录内容,顶层目录本身保留。
/// 红线:遇 reparse point 只删链接本身、永不跟入目标(需求文档 1.3)。
/// filePatterns 模式:只删匹配文件,不动目录结构与链接(缩略图缓存场景)。
/// 逐文件容错:被占用的跳过计入 failed,绝不因个别文件失败中断整项。
fn delete_contents(dir: &Path, patterns: Option<&[String]>, ctx: &mut CleanCtx) {
    let Ok(read) = std::fs::read_dir(dir) else {
        ctx.failed += 1;
        return;
    };
    for entry in read.flatten() {
        if ctx.cancel.load(Ordering::Relaxed) {
            return;
        }
        let Ok(ft) = entry.file_type() else {
            ctx.failed += 1;
            continue;
        };
        let p = entry.path();
        if ft.is_symlink() {
            if patterns.is_some() {
                continue;
            }
            // 目录型联接用 remove_dir,文件型链接用 remove_file;两者都只摘链接、不碰目标
            if std::fs::remove_dir(&p).or_else(|_| std::fs::remove_file(&p)).is_ok() {
                ctx.files += 1;
            } else {
                ctx.failed += 1;
            }
        } else if ft.is_dir() {
            delete_contents(&p, patterns, ctx);
            if patterns.is_none() {
                // 内容删完后移除子目录;失败说明里面还有占用文件,已计入 failed
                let _ = std::fs::remove_dir(&p);
            }
        } else {
            if !matches_patterns(&entry.file_name(), patterns) {
                continue;
            }
            let size = entry
                .metadata()
                .map(|m| {
                    use std::os::windows::fs::MetadataExt;
                    allocated_size(&p, m.len(), m.file_attributes())
                })
                .unwrap_or(0);
            match std::fs::remove_file(&p) {
                Ok(_) => {
                    ctx.bytes += size;
                    ctx.files += 1;
                }
                Err(_) => ctx.failed += 1,
            }
            ctx.tick();
        }
    }
}

fn append_log(app: &AppHandle, line: &str) -> Option<PathBuf> {
    let dir = app.path().app_log_dir().ok()?;
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("clean.log");
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path).ok()?;
    writeln!(f, "{line}").ok()?;
    Some(path)
}

#[tauri::command]
pub async fn run_clean(app: AppHandle, rule_ids: Vec<String>) -> Result<CleanResult, String> {
    {
        let state = app.state::<CleanState>();
        if state.running.swap(true, Ordering::SeqCst) {
            return Err("已在清理中".into());
        }
        state.cancel.store(false, Ordering::SeqCst);
    }
    let app2 = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || do_clean(&app2, rule_ids))
        .await
        .map_err(|e| e.to_string());
    app.state::<CleanState>().running.store(false, Ordering::SeqCst);
    result
}

fn do_clean(app: &AppHandle, rule_ids: Vec<String>) -> CleanResult {
    let state = app.state::<CleanState>();
    let elevated = is_elevated();
    let rules: Vec<Rule> = load_rules()
        .into_iter()
        .filter(|r| r.action == "clean" && rule_ids.iter().any(|id| id == &r.id))
        .collect();

    let mut total = CleanResult {
        freed_bytes: 0,
        deleted_files: 0,
        failed_files: 0,
        skipped: Vec::new(),
        log_path: None,
    };
    let stamp = || chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    for rule in rules {
        if state.cancel.load(Ordering::Relaxed) {
            break;
        }
        // 后端是权限矩阵的最后防线:即使前端置灰失效也不越权执行
        if rule.needs_admin && !elevated {
            total.skipped.push(SkippedRule {
                rule_id: rule.id,
                reason: "需要管理员权限".into(),
            });
            continue;
        }
        // 引导型项(Windows.old/驱动仓库等)不由本工具删除——DriverStore 在用
        // 与废弃驱动同层混放、Windows.old 归 TrustedInstaller,逐文件删会出事。
        // 前端已禁勾,此处是最后防线
        if rule.guide_only {
            total.skipped.push(SkippedRule {
                rule_id: rule.id,
                reason: "此项不由本工具直接删除,请按卡片上的说明手动处理".into(),
            });
            continue;
        }

        if rule.path_pattern == "special:recycle-bin" {
            let (bytes, count) = recycle_bin_status();
            if count == 0 {
                continue;
            }
            match empty_recycle_bin() {
                Ok(()) => {
                    total.freed_bytes += bytes;
                    total.deleted_files += count;
                    let _ = app.emit(
                        "clean:progress",
                        CleanProgress {
                            rule_id: rule.id.clone(),
                            freed_bytes: total.freed_bytes,
                            deleted_files: total.deleted_files,
                        },
                    );
                    total.log_path = append_log(
                        app,
                        &format!("[{}] rule=recycle-bin freed={bytes}B items={count}", stamp()),
                    )
                    .map(|p| p.to_string_lossy().into_owned())
                    .or(total.log_path);
                }
                Err(e) => total.skipped.push(SkippedRule { rule_id: rule.id, reason: e }),
            }
            continue;
        }

        let Some(path) = expand_pattern(&rule.path_pattern) else { continue };
        if !path.is_dir() {
            continue;
        }
        // 计数器以全局累计值起步,进度事件报全局值,前端进度条才单调递增
        let mut ctx = CleanCtx {
            app,
            cancel: &state.cancel,
            rule_id: rule.id.clone(),
            bytes: total.freed_bytes,
            files: total.deleted_files,
            failed: 0,
            last_emit: Instant::now(),
        };
        delete_contents(&path, rule.file_patterns.as_deref(), &mut ctx);
        let rule_bytes = ctx.bytes - total.freed_bytes;
        let rule_files = ctx.files - total.deleted_files;
        total.freed_bytes = ctx.bytes;
        total.deleted_files = ctx.files;
        total.failed_files += ctx.failed;
        total.log_path = append_log(
            app,
            &format!(
                "[{}] rule={} path={} deleted={} freed={}B skipped_locked={}",
                stamp(),
                rule.id,
                path.display(),
                rule_files,
                rule_bytes,
                ctx.failed
            ),
        )
        .map(|p| p.to_string_lossy().into_owned())
        .or(total.log_path);
    }
    total
}

#[tauri::command]
pub fn cancel_clean(state: State<'_, CleanState>) {
    if state.running.load(Ordering::SeqCst) {
        state.cancel.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod driver_tests {
    use super::*;

    /// 真机冒烟:旧版口径必须严格不超过整仓口径;输出预估值供与 cleanmgr 对照
    #[test]
    fn old_driver_size_is_subset_of_repo() {
        let repo = PathBuf::from(std::env::var("WINDIR").unwrap())
            .join(r"System32\DriverStore\FileRepository");
        if !repo.is_dir() {
            return;
        }
        let (old_bytes, old_files) = measure_old_driver_packages(&repo);
        let (total, _) = measure_dir(&repo, None);
        eprintln!(
            "旧版驱动预估 {:.2} GB / {} 文件;整仓 {:.2} GB",
            old_bytes as f64 / 1e9,
            old_files,
            total as f64 / 1e9
        );
        assert!(old_bytes <= total);
    }

    #[test]
    fn driver_ver_parsing() {
        assert_eq!(
            parse_driver_ver("[Version]\r\nDriverVer = 07/01/2021,27.21.14.5671\r\n"),
            Some((27, 21, 14, 5671, 2021, 7, 1))
        );
        assert_eq!(
            parse_driver_ver("driverver=1/2/2020,1.0 ; comment"),
            Some((1, 0, 0, 0, 2020, 1, 2))
        );
        assert_eq!(parse_driver_ver("NoVersionHere=1"), None);
        assert_eq!(driver_group_key("nv_dispi.inf_amd64_abc123"), Some("nv_dispi.inf_amd64".into()));
        assert_eq!(driver_group_key("weird-folder"), None);
    }
}
