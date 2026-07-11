use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
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
fn allocated_size(path: &std::path::Path, logical: u64, attrs: u32) -> u64 {
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
    // 遍历产出的 parent_path 与建节点时的 path 字符串来源一致,可作精确键
    let mut index: HashMap<PathBuf, u32> = HashMap::new();
    index.insert(root.clone(), 0);

    let mut files: u64 = 0;
    let mut bytes: u64 = 0;
    let mut denied: u64 = 0;
    let mut last_emit = Instant::now();

    let walker = jwalk::WalkDir::new(&root)
        .follow_links(false)
        .skip_hidden(false);

    for entry in walker {
        if state.cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        let e = match entry {
            Ok(e) => e,
            Err(_) => {
                denied += 1;
                continue;
            }
        };
        if e.depth == 0 {
            continue;
        }
        let parent_id = match index.get(e.parent_path()) {
            Some(&id) => id,
            None => continue, // 父目录曾读取失败,子项无处挂,跳过
        };

        if e.file_type.is_symlink() {
            // junction/symlink:标注「已迁移」并显示指向,绝不跟入(产品红线)
            let path = e.path();
            let target = std::fs::read_link(&path)
                .ok()
                .map(|p| p.to_string_lossy().into_owned());
            let id = nodes.len() as u32;
            let rule = rule_paths
                .get(&path.to_string_lossy().to_lowercase())
                .copied();
            nodes.push(DirNode {
                name: e.file_name.to_string_lossy().into_owned(),
                parent: Some(parent_id),
                total_bytes: 0,
                file_count: 0,
                children: Vec::new(),
                is_reparse: true,
                reparse_target: target,
                rule,
            });
            nodes[parent_id as usize].children.push(id);
        } else if e.file_type.is_dir() {
            let path = e.path();
            let id = nodes.len() as u32;
            let rule = rule_paths
                .get(&path.to_string_lossy().to_lowercase())
                .copied();
            nodes.push(DirNode {
                name: e.file_name.to_string_lossy().into_owned(),
                parent: Some(parent_id),
                total_bytes: 0,
                file_count: 0,
                children: Vec::new(),
                is_reparse: false,
                reparse_target: None,
                rule,
            });
            nodes[parent_id as usize].children.push(id);
            index.insert(path, id);
        } else {
            let Ok(meta) = e.metadata() else {
                denied += 1;
                continue;
            };
            use std::os::windows::fs::MetadataExt;
            let size = allocated_size(&e.path(), meta.len(), meta.file_attributes());
            let p = &mut nodes[parent_id as usize];
            p.total_bytes += size;
            p.file_count += 1;
            files += 1;
            bytes += size;
            if files % 512 == 0 && last_emit.elapsed().as_millis() >= 100 {
                let _ = app.emit(
                    "scan:progress",
                    ScanProgress {
                        scanned_files: files,
                        scanned_bytes: bytes,
                        current_path: e.parent_path().to_string_lossy().into_owned(),
                    },
                );
                last_emit = Instant::now();
            }
        }
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
    Ok(summary)
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
