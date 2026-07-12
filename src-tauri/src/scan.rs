use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::rules::{expand_pattern, load_rules, Rule};

/// arena 树节点:只为目录建节点,文件只累加到父目录。
/// 父节点必先于子节点创建,故 child.id > parent.id 恒成立,
/// 汇总阶段倒序遍历一次即完成自底向上聚合。
struct DirNode {
    name: String,
    parent: Option<u32>,
    /// 扫描期存本目录直接文件大小,汇总后为子树总大小(实占口径)
    total_bytes: u64,
    file_count: u64,
    children: Vec<u32>,
    is_reparse: bool,
    reparse_target: Option<String>,
    rule: Option<u16>,
}

struct BigFileEntry {
    dir: u32,
    name: String,
    size: u64,
    mtime_ms: u64,
}

pub struct ScanResult {
    nodes: Vec<DirNode>,
    rules: Vec<Rule>,
    root_path: PathBuf,
    big_files: Vec<BigFileEntry>,
}

#[derive(Default)]
pub struct ScanState {
    pub result: Mutex<Option<ScanResult>>,
    pub running: AtomicBool,
    pub cancel: AtomicBool,
    /// 撤回搬家的目录:小写路径 → 撤回时的精确字节数。
    /// 体检时已迁移目录是联接、快照里大小为 0,撤回后它重新占用 C 盘,
    /// 该表把撤回时统计到的大小补进「可搬家」;重新体检后作废清空。
    pub reverted: Mutex<std::collections::HashMap<String, u64>>,
    /// 本次识别出的孤儿 profile 原始路径(WTF-8 无损)。
    /// 删除接口只认这里面的条目——白名单校验的数据源;
    /// 乱码目录名含无效 UTF-16,给前端的 String 是 lossy 的,
    /// 实际文件操作必须用这里的原始 PathBuf。
    pub orphans: Mutex<Vec<PathBuf>>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    scanned_files: u64,
    scanned_bytes: u64,
    current_path: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ScanSummary {
    pub root_id: u32,
    pub total_bytes: u64,
    pub total_files: u64,
    pub dir_count: u32,
    /// 权限不足等原因跳过的条目数,前端据此标注「部分系统区域未扫描」
    pub denied_entries: u64,
    pub elapsed_ms: u64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RuleTag {
    display_name: String,
    explain: String,
    risk: String,
    action: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeNode {
    id: u32,
    name: String,
    path: String,
    size_bytes: u64,
    file_count: u64,
    has_children: bool,
    is_reparse: bool,
    reparse_target: Option<String>,
    rule: Option<RuleTag>,
}

const FILE_ATTRIBUTE_SPARSE_FILE: u32 = 0x0000_0200;
const FILE_ATTRIBUTE_COMPRESSED: u32 = 0x0000_0800;
const FILE_ATTRIBUTE_OFFLINE: u32 = 0x0000_1000;
const FILE_ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x0004_0000;
const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;

/// OneDrive 按需文件/稀疏/压缩文件的逻辑大小会虚高(占位文件逻辑 10GB 实占 0),
/// 统计口径必须是实际占用——只对带特殊属性的文件多付一次系统调用,普通文件走快路径。
/// 全程只读元数据,绝不打开文件内容,避免触发 OneDrive 全量下载(需求文档 F1 特殊处理②)。
pub(crate) fn allocated_size(path: &Path, logical: u64, attrs: u32) -> u64 {
    const SPECIAL: u32 = FILE_ATTRIBUTE_SPARSE_FILE
        | FILE_ATTRIBUTE_COMPRESSED
        | FILE_ATTRIBUTE_OFFLINE
        | FILE_ATTRIBUTE_RECALL_ON_OPEN
        | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS;
    if attrs & SPECIAL == 0 {
        return logical;
    }
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut high: u32 = 0;
    let low = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetCompressedFileSizeW(wide.as_ptr(), &mut high)
    };
    if low == u32::MAX {
        let err = unsafe { windows_sys::Win32::Foundation::GetLastError() };
        if err != 0 {
            return logical;
        }
    }
    ((high as u64) << 32) | low as u64
}

/// worker 扫完一个目录发回的汇总。文件级信息就地聚合,不跨线程传条目。
struct DirScanMsg {
    node_id: u32,
    path: PathBuf,
    direct_bytes: u64,
    file_count: u64,
    denied: u64,
    subdirs: Vec<SubDir>,
    /// 顺带收集的大文件(F4):扫描本来就枚举每个文件,收集零额外系统调用
    big_files: Vec<BigFileRaw>,
}

struct BigFileRaw {
    name: String,
    size: u64,
    mtime_ms: u64,
}

/// 大文件收录门槛:实占 ≥100MB(需求文档 F4)。用实占而非逻辑大小——
/// OneDrive 占位文件逻辑 10GB 实占 0,删它并不腾出 C 盘,不该进列表
const BIG_FILE_MIN: u64 = 100 * 1024 * 1024;
/// 收集上限:一般 C 盘 ≥100MB 文件几十到几百个,上限只防极端盘
const BIG_FILE_CAP: usize = 5000;

struct SubDir {
    name: String,
    is_reparse: bool,
    reparse_target: Option<String>,
}

/// 单目录扫描:一次 read_dir 枚举。Windows 上 FindNextFile 已带回大小与属性,
/// std 将其缓存于 DirEntry,metadata() 零额外系统调用——这是普通权限下的物理下限。
/// (jwalk 的 metadata() 会按路径重新查询,弃用它正是为了这份免费数据)
fn scan_one_dir(node_id: u32, path: &Path) -> DirScanMsg {
    let mut msg = DirScanMsg {
        node_id,
        path: path.to_path_buf(),
        direct_bytes: 0,
        file_count: 0,
        denied: 0,
        subdirs: Vec::new(),
        big_files: Vec::new(),
    };
    let read = match std::fs::read_dir(path) {
        Ok(r) => r,
        Err(_) => {
            msg.denied = 1;
            return msg;
        }
    };
    for entry in read {
        let Ok(entry) = entry else {
            msg.denied += 1;
            continue;
        };
        let Ok(ft) = entry.file_type() else {
            msg.denied += 1;
            continue;
        };
        if ft.is_symlink() {
            // junction/symlink:标注「已迁移」并显示指向,绝不跟入(产品红线)
            let p = entry.path();
            msg.subdirs.push(SubDir {
                name: entry.file_name().to_string_lossy().into_owned(),
                is_reparse: true,
                reparse_target: std::fs::read_link(&p)
                    .ok()
                    .map(|t| t.to_string_lossy().into_owned()),
            });
        } else if ft.is_dir() {
            msg.subdirs.push(SubDir {
                name: entry.file_name().to_string_lossy().into_owned(),
                is_reparse: false,
                reparse_target: None,
            });
        } else {
            let Ok(meta) = entry.metadata() else {
                msg.denied += 1;
                continue;
            };
            use std::os::windows::fs::MetadataExt;
            let attrs = meta.file_attributes();
            let logical = meta.len();
            // 仅特殊属性文件才拼路径并额外查询实占
            let allocated = allocated_size(&entry.path(), logical, attrs);
            msg.direct_bytes += allocated;
            msg.file_count += 1;
            if allocated >= BIG_FILE_MIN {
                let mtime_ms = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                msg.big_files.push(BigFileRaw {
                    name: entry.file_name().to_string_lossy().into_owned(),
                    size: allocated,
                    mtime_ms,
                });
            }
        }
    }
    msg
}

fn do_scan(app: &AppHandle, state: &ScanState, root: PathBuf) -> Result<ScanSummary, String> {
    let start = Instant::now();
    let rules = load_rules();
    // 规则绝对路径(小写) -> 规则下标,建目录节点时 O(1) 命中
    let rule_paths: HashMap<String, u16> = rules
        .iter()
        .enumerate()
        .filter_map(|(i, r)| {
            expand_pattern(&r.path_pattern)
                .map(|p| (p.to_string_lossy().to_lowercase(), i as u16))
        })
        .collect();

    let mut nodes = vec![DirNode {
        name: root.to_string_lossy().into_owned(),
        parent: None,
        total_bytes: 0,
        file_count: 0,
        children: Vec::new(),
        is_reparse: false,
        reparse_target: None,
        rule: None,
    }];

    let mut files: u64 = 0;
    let mut bytes: u64 = 0;
    let mut denied: u64 = 0;
    let mut big_files: Vec<BigFileEntry> = Vec::new();
    let mut last_emit = Instant::now();
    let mut cancelled = false;

    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(4, 16);
    let (task_tx, task_rx) = crossbeam_channel::unbounded::<(u32, PathBuf)>();
    let (res_tx, res_rx) = crossbeam_channel::unbounded::<DirScanMsg>();

    std::thread::scope(|s| {
        for _ in 0..workers {
            let task_rx = task_rx.clone();
            let res_tx = res_tx.clone();
            let cancel = &state.cancel;
            s.spawn(move || {
                while let Ok((node_id, path)) = task_rx.recv() {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    if res_tx.send(scan_one_dir(node_id, &path)).is_err() {
                        break;
                    }
                }
            });
        }
        // 主线程仅持 worker 手里的克隆,自身的原件立即释放,
        // 这样 drop(task_tx) 后 worker 的 recv 才能断开
        drop(task_rx);
        drop(res_tx);

        let mut pending: u64 = 1;
        let _ = task_tx.send((0, root.clone()));

        while pending > 0 {
            if state.cancel.load(Ordering::Relaxed) {
                cancelled = true;
                break;
            }
            let Ok(msg) = res_rx.recv() else { break };
            pending -= 1;

            files += msg.file_count;
            bytes += msg.direct_bytes;
            denied += msg.denied;
            for bf in msg.big_files {
                if big_files.len() < BIG_FILE_CAP {
                    big_files.push(BigFileEntry {
                        dir: msg.node_id,
                        name: bf.name,
                        size: bf.size,
                        mtime_ms: bf.mtime_ms,
                    });
                }
            }
            {
                let n = &mut nodes[msg.node_id as usize];
                n.total_bytes += msg.direct_bytes;
                n.file_count += msg.file_count;
            }
            for sub in msg.subdirs {
                let child_path = msg.path.join(&sub.name);
                let id = nodes.len() as u32;
                let rule = rule_paths
                    .get(&child_path.to_string_lossy().to_lowercase())
                    .copied();
                nodes.push(DirNode {
                    name: sub.name,
                    parent: Some(msg.node_id),
                    total_bytes: 0,
                    file_count: 0,
                    children: Vec::new(),
                    is_reparse: sub.is_reparse,
                    reparse_target: sub.reparse_target,
                    rule,
                });
                nodes[msg.node_id as usize].children.push(id);
                if !nodes[id as usize].is_reparse {
                    pending += 1;
                    let _ = task_tx.send((id, child_path));
                }
            }

            if last_emit.elapsed().as_millis() >= 100 {
                let _ = app.emit(
                    "scan:progress",
                    ScanProgress {
                        scanned_files: files,
                        scanned_bytes: bytes,
                        current_path: msg.path.to_string_lossy().into_owned(),
                    },
                );
                last_emit = Instant::now();
            }
        }
        drop(task_tx); // 断开任务队列,worker recv 出错退出,scope 随即回收
    });

    if cancelled {
        return Err("cancelled".into());
    }

    // 自底向上汇总:child.id > parent.id,倒序一遍完成
    for i in (1..nodes.len()).rev() {
        let (tb, fc, parent) = {
            let n = &nodes[i];
            (n.total_bytes, n.file_count, n.parent)
        };
        if let Some(p) = parent {
            let pn = &mut nodes[p as usize];
            pn.total_bytes += tb;
            pn.file_count += fc;
        }
    }

    let summary = ScanSummary {
        root_id: 0,
        total_bytes: nodes[0].total_bytes,
        total_files: nodes[0].file_count,
        dir_count: nodes.len() as u32,
        denied_entries: denied,
        elapsed_ms: start.elapsed().as_millis() as u64,
    };
    *state.result.lock().unwrap() = Some(ScanResult {
        nodes,
        rules,
        root_path: root,
        big_files,
    });
    // 新快照已含撤回目录的真实数据,补丁表作废
    state.reverted.lock().unwrap().clear();
    Ok(summary)
}

fn node_path(scan: &ScanResult, id: u32) -> PathBuf {
    let mut names: Vec<&str> = Vec::new();
    let mut cur = &scan.nodes[id as usize];
    loop {
        match cur.parent {
            Some(p) => {
                names.push(&cur.name);
                cur = &scan.nodes[p as usize];
            }
            None => break,
        }
    }
    let mut path = scan.root_path.clone();
    for name in names.iter().rev() {
        path.push(name);
    }
    path
}

#[tauri::command]
pub async fn start_scan(app: AppHandle, root: Option<String>) -> Result<ScanSummary, String> {
    {
        let state = app.state::<ScanState>();
        if state.running.swap(true, Ordering::SeqCst) {
            return Err("已在扫描中".into());
        }
        state.cancel.store(false, Ordering::SeqCst);
    }
    let root = root.map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(format!(
            "{}\\",
            std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into())
        ))
    });
    let app_for_scan = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let state = app_for_scan.state::<ScanState>();
        do_scan(&app_for_scan, &state, root)
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|r| r);
    app.state::<ScanState>().running.store(false, Ordering::SeqCst);
    result
}

#[tauri::command]
pub fn cancel_scan(state: State<'_, ScanState>) {
    if state.running.load(Ordering::SeqCst) {
        state.cancel.store(true, Ordering::SeqCst);
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BigFileInfo {
    path: String,
    name: String,
    size_bytes: u64,
    modified_ms: u64,
    /// video | archive | installer | image | other
    category: String,
    deletable: bool,
    /// 不可删时的白话解释(需求文档 F4:系统关键文件只展示不可删)
    reason: Option<String>,
}

fn file_category(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "mp4" | "mkv" | "avi" | "mov" | "flv" | "wmv" | "ts" | "webm" | "m4v" | "rmvb" => "video",
        "zip" | "rar" | "7z" | "tar" | "gz" | "xz" | "bz2" | "cab" => "archive",
        "exe" | "msi" | "msix" | "appx" => "installer",
        "iso" | "img" | "vhd" | "vhdx" | "wim" | "esd" | "gho" => "image",
        _ => "other",
    }
}

/// 系统关键文件判定:(不可删, 白话原因)
fn deletability(name_lower: &str, path_lower: &str, windir_lower: &str) -> (bool, Option<String>) {
    if matches!(name_lower, "pagefile.sys" | "hiberfil.sys" | "swapfile.sys") {
        return (
            false,
            Some("这是系统的虚拟内存/休眠文件,不能直接删。想减小它需要在系统设置里调整".into()),
        );
    }
    if path_lower.starts_with(&format!("{windir_lower}\\")) {
        return (false, Some("这是 Windows 系统文件,删了系统可能出问题,只看不动".into()));
    }
    (true, None)
}

/// 大文件列表(F4)。对 F2/F3 已覆盖的路径去重——那些空间已计入
/// 「垃圾清理」或「可搬家」,再列一遍会双重计数(需求文档 §3.5)。
#[tauri::command]
pub fn get_big_files(state: State<'_, ScanState>) -> Result<Vec<BigFileInfo>, String> {
    let guard = state.result.lock().map_err(|e| e.to_string())?;
    let scan = guard.as_ref().ok_or("尚未完成扫描")?;
    let covered: Vec<String> = scan
        .rules
        .iter()
        .filter(|r| r.action == "clean" || r.action == "migrate")
        .filter_map(|r| expand_pattern(&r.path_pattern))
        .map(|p| format!("{}\\", p.to_string_lossy().to_lowercase()))
        .collect();
    let windir = std::env::var("WINDIR")
        .unwrap_or_else(|_| "C:\\Windows".into())
        .to_lowercase();
    let mut out: Vec<BigFileInfo> = Vec::new();
    for bf in &scan.big_files {
        let path = node_path(scan, bf.dir).join(&bf.name);
        let lower = path.to_string_lossy().to_lowercase();
        if covered.iter().any(|c| lower.starts_with(c.as_str())) {
            continue;
        }
        let name_lower = bf.name.to_lowercase();
        let (deletable, reason) = deletability(&name_lower, &lower, &windir);
        out.push(BigFileInfo {
            path: path.to_string_lossy().into_owned(),
            name: bf.name.clone(),
            size_bytes: bf.size,
            modified_ms: bf.mtime_ms,
            category: file_category(&bf.name).to_string(),
            deletable,
            reason,
        });
    }
    out.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    Ok(out)
}

/// 进回收站删除(FOF_ALLOWUNDO),保留反悔余地(需求文档 F4 安全)
fn recycle_delete(p: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::{
        SHFileOperationW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT,
        FO_DELETE, SHFILEOPSTRUCTW,
    };
    // pFrom 要求双 \0 结尾的路径列表
    let mut from: Vec<u16> = p.as_os_str().encode_wide().collect();
    from.push(0);
    from.push(0);
    let mut op = SHFILEOPSTRUCTW {
        hwnd: std::ptr::null_mut(),
        wFunc: FO_DELETE,
        pFrom: from.as_ptr(),
        pTo: std::ptr::null(),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI) as u16,
        fAnyOperationsAborted: 0,
        hNameMappings: std::ptr::null_mut(),
        lpszProgressTitle: std::ptr::null(),
    };
    let rc = unsafe { SHFileOperationW(&mut op) };
    if rc != 0 {
        return Err(format!("删除没有成功(代码 {rc}),文件可能正被使用"));
    }
    if op.fAnyOperationsAborted != 0 {
        return Err("删除被中止,文件没有变化".into());
    }
    Ok(())
}

/// 删除大文件。双防线:①路径必须是本次扫描收录的大文件(接口层面杜绝
/// 任意路径删除);②系统关键文件复核拒绝——前端置灰只是第一道。
#[tauri::command]
pub async fn delete_big_file(app: AppHandle, path: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ScanState>();
        let guard = state.result.lock().map_err(|e| e.to_string())?;
        let scan = guard.as_ref().ok_or("尚未完成扫描")?;
        let lower = path.to_lowercase();
        let known = scan.big_files.iter().any(|bf| {
            node_path(scan, bf.dir)
                .join(&bf.name)
                .to_string_lossy()
                .to_lowercase()
                == lower
        });
        if !known {
            return Err("这个文件不在本次体检的大文件列表里,拒绝删除".into());
        }
        let windir = std::env::var("WINDIR")
            .unwrap_or_else(|_| "C:\\Windows".into())
            .to_lowercase();
        let name_lower = Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let (deletable, _) = deletability(&name_lower, &lower, &windir);
        if !deletable {
            return Err("系统文件不能删除".into());
        }
        drop(guard);
        recycle_delete(Path::new(&path))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigratableItem {
    rule_id: String,
    display_name: String,
    explain: String,
    path: String,
    size_bytes: u64,
}

/// 从扫描树汇总可搬家(migrate 规则)目录:报告页「可搬家 X GB」卡片数据源。
/// 扫描树是体检时刻的快照,两处与现实的偏差都在此校正:
/// ① 刚搬完家的目录树里还是原样 → 实时复查联接状态,已迁移的不再列出;
/// ② 刚撤回的目录树里大小为 0(体检时是联接) → 用撤回补丁表里的精确大小补回。
#[tauri::command]
pub fn get_migratables(state: State<'_, ScanState>) -> Result<Vec<MigratableItem>, String> {
    let guard = state.result.lock().map_err(|e| e.to_string())?;
    let scan = guard.as_ref().ok_or("尚未完成扫描")?;
    let reverted = state.reverted.lock().map_err(|e| e.to_string())?;
    let mut out: Vec<MigratableItem> = Vec::new();
    for (i, node) in scan.nodes.iter().enumerate() {
        let Some(ri) = node.rule else { continue };
        let r = &scan.rules[ri as usize];
        if r.action != "migrate" {
            continue;
        }
        let path = node_path(scan, i as u32);
        let migrated_or_gone = std::fs::symlink_metadata(&path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(true);
        if migrated_or_gone {
            continue;
        }
        let size_bytes = if node.total_bytes > 0 {
            node.total_bytes
        } else {
            reverted
                .get(&path.to_string_lossy().to_lowercase())
                .copied()
                .unwrap_or(0)
        };
        if size_bytes == 0 {
            continue;
        }
        out.push(MigratableItem {
            rule_id: r.id.clone(),
            display_name: r.display_name.clone(),
            explain: r.explain.clone(),
            path: path.to_string_lossy().into_owned(),
            size_bytes,
        });
    }
    out.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    Ok(out)
}

/// 懒加载:前端每展开一层才取一层,整棵树留在 Rust 侧
#[tauri::command]
pub fn get_children(state: State<'_, ScanState>, node_id: u32) -> Result<Vec<TreeNode>, String> {
    let guard = state.result.lock().map_err(|e| e.to_string())?;
    let scan = guard.as_ref().ok_or("尚未完成扫描")?;
    let node = scan
        .nodes
        .get(node_id as usize)
        .ok_or("节点不存在")?;
    let base = node_path(scan, node_id);
    let mut out: Vec<TreeNode> = node
        .children
        .iter()
        .map(|&cid| {
            let c = &scan.nodes[cid as usize];
            TreeNode {
                id: cid,
                name: c.name.clone(),
                path: base.join(&c.name).to_string_lossy().into_owned(),
                size_bytes: c.total_bytes,
                file_count: c.file_count,
                has_children: !c.children.is_empty(),
                is_reparse: c.is_reparse,
                reparse_target: c.reparse_target.clone(),
                rule: c.rule.map(|ri| {
                    let r = &scan.rules[ri as usize];
                    RuleTag {
                        display_name: r.display_name.clone(),
                        explain: r.explain.clone(),
                        risk: r.risk.clone(),
                        action: r.action.clone(),
                    }
                }),
            }
        })
        .collect();
    out.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    Ok(out)
}

// ─── 孤儿 profile 检测(融合方案 §2):C:\Users 下被软件以异常用户名创建、
// 只剩 AppData 缓存的废弃 profile 残骸。三条件全满足才判定(宁可漏报不误伤)。

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OrphanProfile {
    /// 显示名(可能是乱码,前端原样呈现)
    name: String,
    path: String,
    size_bytes: u64,
    file_count: u64,
    /// 据 AppData 内部目录推断的来源软件线索,降低用户删除时的恐惧
    hints: Vec<String>,
}

/// 读取 HKLM ProfileList 中系统登记的全部 profile 目录(小写)。
/// 任一步失败返回 None——调用方必须降级为「不判定」,绝不因取数失败误伤。
fn registered_profile_paths() -> Option<Vec<String>> {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_LOCAL_MACHINE,
        KEY_READ, RRF_RT_REG_SZ,
    };
    let root: Vec<u16> = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\ProfileList"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let value_name: Vec<u16> = "ProfileImagePath"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let mut hkey: HKEY = std::ptr::null_mut();
        if RegOpenKeyExW(HKEY_LOCAL_MACHINE, root.as_ptr(), 0, KEY_READ, &mut hkey) != 0 {
            return None;
        }
        let mut out = Vec::new();
        let mut index = 0u32;
        loop {
            let mut name = [0u16; 256];
            let mut name_len = name.len() as u32;
            if RegEnumKeyExW(
                hkey,
                index,
                name.as_mut_ptr(),
                &mut name_len,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            ) != 0
            {
                break; // ERROR_NO_MORE_ITEMS,枚举完毕
            }
            index += 1;
            let mut buf = [0u16; 1024];
            let mut size = (buf.len() * 2) as u32;
            // RRF_RT_REG_SZ:REG_EXPAND_SZ 值会被自动展开后按 SZ 返回
            if RegGetValueW(
                hkey,
                name.as_ptr(),
                value_name.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buf.as_mut_ptr() as *mut _,
                &mut size,
            ) == 0
            {
                // 按第一个 NUL 截断——REG_EXPAND_SZ 展开后 size 可能大于
                // 实际字符串,NUL 之后是上轮循环的缓冲区残留,不能信 size
                let end = buf
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or_else(|| (size as usize / 2).min(buf.len()));
                out.push(String::from_utf16_lossy(&buf[..end]).to_lowercase());
            }
        }
        RegCloseKey(hkey);
        // 正常系统至少登记 systemprofile/LocalService/NetworkService + 真实用户,
        // 读出来是空说明取数异常,同样降级
        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    }
}

/// 条件③:废弃 profile 的指纹——子目录仅 AppData,顶层文件仅系统噪音。
/// 出现任何其他内容(哪怕一个文档)立即否决,宁可漏报。
fn looks_like_abandoned_profile(dir: &Path) -> bool {
    let Ok(read) = std::fs::read_dir(dir) else {
        return false;
    };
    let mut has_appdata = false;
    for entry in read.flatten() {
        let Ok(ft) = entry.file_type() else {
            return false;
        };
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if ft.is_symlink() {
            return false;
        } else if ft.is_dir() {
            if name == "appdata" {
                has_appdata = true;
            } else {
                return false;
            }
        } else if name != "desktop.ini" && !name.starts_with("ntuser") {
            return false;
        }
    }
    has_appdata
}

/// 扫 AppData 内部推断残留来自哪个软件。精确条目在前,泛条目兜底。
fn orphan_hints(dir: &Path) -> Vec<String> {
    const KNOWN: &[(&str, &str)] = &[
        ("tencent\\qqpcmgr", "腾讯电脑管家"),
        ("tencent\\wemeet", "腾讯会议"),
        ("tencent\\wxwork", "企业微信"),
        ("iqiyi", "爱奇艺"),
        ("adobe", "Adobe"),
        ("nvidia", "NVIDIA 驱动组件"),
        ("tencent", "腾讯系软件"),
    ];
    fn push(name: &str, out: &mut Vec<String>) {
        if !out.iter().any(|n| n == name) {
            out.push(name.to_string());
        }
    }
    let mut out: Vec<String> = Vec::new();
    for sub in ["AppData\\Roaming", "AppData\\Local", "AppData\\LocalLow"] {
        let Ok(read) = std::fs::read_dir(dir.join(sub)) else {
            continue;
        };
        for entry in read.flatten() {
            let vendor = entry.file_name().to_string_lossy().to_lowercase();
            let mut matched = false;
            // vendor\app 二级 key 优先(区分腾讯全家桶里的具体软件)
            if let Ok(inner) = std::fs::read_dir(entry.path()) {
                for e2 in inner.flatten() {
                    let key =
                        format!("{vendor}\\{}", e2.file_name().to_string_lossy().to_lowercase());
                    if let Some((_, name)) = KNOWN.iter().find(|(k, _)| key.starts_with(k)) {
                        push(name, &mut out);
                        matched = true;
                    }
                }
            }
            if !matched {
                if let Some((_, name)) = KNOWN.iter().find(|(k, _)| vendor.starts_with(k)) {
                    push(name, &mut out);
                }
            }
            if out.len() >= 3 {
                return out;
            }
        }
    }
    out
}

/// 三条件判定主体(纯逻辑,不碰 tauri State,测试直接调用)
fn detect_orphans() -> Result<Vec<(PathBuf, OrphanProfile)>, String> {
    let users_root = PathBuf::from(format!(
        "{}\\Users",
        std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into())
    ));
    // 条件②数据源取不到 → 全部不判定
    let Some(registered) = registered_profile_paths() else {
        return Ok(Vec::new());
    };
    let current = std::env::var("USERPROFILE").ok().map(|p| p.to_lowercase());
    const RESERVED: &[&str] = &["default", "default user", "public", "all users"];
    let mut found: Vec<(PathBuf, OrphanProfile)> = Vec::new();
    let read = std::fs::read_dir(&users_root).map_err(|e| e.to_string())?;
    for entry in read.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_dir() || ft.is_symlink() {
            continue;
        }
        let path = entry.path();
        let name_lossy = entry.file_name().to_string_lossy().into_owned();
        if RESERVED.contains(&name_lossy.to_lowercase().as_str()) {
            continue;
        }
        let path_lower = path.to_string_lossy().to_lowercase();
        if current.as_deref() == Some(path_lower.as_str()) {
            continue;
        }
        // 条件②:系统登记过的 profile 不是孤儿
        if registered.iter().any(|r| r == &path_lower) {
            continue;
        }
        // 条件①:NTUSER.DAT 存在或无法确认(权限等)都跳过,保守方向
        match path.join("NTUSER.DAT").try_exists() {
            Ok(false) => {}
            _ => continue,
        }
        // 条件③:废弃 profile 指纹
        if !looks_like_abandoned_profile(&path) {
            continue;
        }
        let (bytes, files) = crate::cleaner::measure_dir(&path, None);
        found.push((
            path.clone(),
            OrphanProfile {
                name: name_lossy,
                path: path.to_string_lossy().into_owned(),
                size_bytes: bytes,
                file_count: files,
                hints: orphan_hints(&path),
            },
        ));
    }
    found.sort_by(|a, b| b.1.size_bytes.cmp(&a.1.size_bytes));
    Ok(found)
}

/// 识别孤儿 profile。与扫描主流程解耦(C:\Users 只有个位数子目录,现查毫秒级),
/// 结果的原始 PathBuf 存入 state.orphans 作为删除接口的白名单。
#[tauri::command]
pub async fn get_orphan_profiles(app: AppHandle) -> Result<Vec<OrphanProfile>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let found = detect_orphans()?;
        let state = app.state::<ScanState>();
        *state.orphans.lock().map_err(|e| e.to_string())? =
            found.iter().map(|(p, _)| p.clone()).collect();
        Ok(found.into_iter().map(|(_, o)| o).collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 删除孤儿 profile(整个目录进回收站,可反悔)。
/// 双防线:①路径必须命中本次识别集,接口层杜绝任意路径删除;
/// ②实际删除用识别集里的原始 PathBuf——乱码目录名含无效 UTF-16,
/// 前端传回的 lossy 字符串只作匹配 key,当真实路径用会找不到文件。
#[tauri::command]
pub async fn delete_orphan_profile(app: AppHandle, path: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ScanState>();
        let target = {
            let orphans = state.orphans.lock().map_err(|e| e.to_string())?;
            let lower = path.to_lowercase();
            orphans
                .iter()
                .find(|p| p.to_string_lossy().to_lowercase() == lower)
                .cloned()
        };
        let Some(real) = target else {
            return Err("这个目录不在本次识别的残留列表里,拒绝删除".into());
        };
        recycle_delete(&real)?;
        state
            .orphans
            .lock()
            .map_err(|e| e.to_string())?
            .retain(|p| p != &real);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod orphan_tests {
    use super::*;

    /// 真机冒烟:ProfileList 必须可读,且当前用户一定在册——
    /// 这是条件②「取数失败降级」防线依赖的基本事实
    #[test]
    fn profile_list_contains_current_user() {
        let paths = registered_profile_paths().expect("ProfileList 读取失败,降级防线会全量跳过");
        let me = std::env::var("USERPROFILE").unwrap().to_lowercase();
        assert!(
            paths.iter().any(|p| p == &me),
            "当前用户 {me} 应在 ProfileList 中,实际:{paths:?}"
        );
    }

    /// 真机自洽:识别结果必须逐条满足三条件与白名单(环境自适应,
    /// 无孤儿的机器上结果为空也通过;有样本的机器上顺带人工核对输出)
    #[test]
    fn detected_orphans_are_self_consistent() {
        let found = detect_orphans().expect("detect_orphans 不应失败");
        let me = std::env::var("USERPROFILE").unwrap().to_lowercase();
        for (raw, o) in &found {
            eprintln!("识别: name=[{}] size={}B files={} hints={:?}", o.name, o.size_bytes, o.file_count, o.hints);
            let lower = raw.to_string_lossy().to_lowercase();
            assert_ne!(lower, me, "绝不允许把当前用户识别为孤儿");
            for reserved in ["default", "public", "all users", "default user"] {
                assert!(!lower.ends_with(&format!("\\{reserved}")), "系统目录 {reserved} 被误报");
            }
            assert!(
                !raw.join("NTUSER.DAT").try_exists().unwrap_or(true),
                "{lower} 有 NTUSER.DAT 却被判孤儿"
            );
        }
        eprintln!("共识别 {} 个孤儿 profile", found.len());
    }
}

#[cfg(test)]
mod orphan_diag {
    use super::*;

    /// 端到端:合成一个孤儿 profile(仅 AppData + 腾讯管家特征),
    /// 应被识别且 hints 命中;跑完清理现场。无权限创建时跳过。
    #[test]
    fn end_to_end_detects_synthetic_orphan() {
        let dir = PathBuf::from(r"C:\Users\_orphan_e2e_test");
        if std::fs::create_dir_all(dir.join(r"AppData\Roaming\Tencent\QQPCMgr")).is_err() {
            eprintln!("跳过:无权限在 C:\\Users 下创建测试目录");
            return;
        }
        std::fs::write(dir.join(r"AppData\Roaming\Tencent\QQPCMgr\t.log"), b"x").unwrap();
        let found = detect_orphans().unwrap();
        let hit = found
            .iter()
            .find(|(p, _)| p == &dir)
            .map(|(_, o)| o.clone());
        // 先清理现场再断言,失败也不留垃圾
        let _ = std::fs::remove_dir_all(&dir);
        let o = hit.expect("合成孤儿目录应被三条件识别");
        assert!(
            o.hints.iter().any(|h| h == "腾讯电脑管家"),
            "hints 应含腾讯电脑管家,实际 {:?}",
            o.hints
        );
        assert!(o.file_count >= 1 && o.size_bytes >= 1);
    }
}
