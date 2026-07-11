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

pub struct ScanResult {
    nodes: Vec<DirNode>,
    rules: Vec<Rule>,
    root_path: PathBuf,
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
}

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
            msg.direct_bytes += allocated_size(&entry.path(), logical, attrs);
            msg.file_count += 1;
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
