use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 知识库规则,schema 见 docs/设计规范.md §7
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    pub id: String,
    pub path_pattern: String,
    /// 只清理目录内匹配这些模式的文件(如缩略图缓存目录混有日志,只删 thumbcache_*.db);
    /// 缺省 = 清理目录全部内容
    #[serde(default)]
    pub file_patterns: Option<Vec<String>>,
    pub display_name: String,
    pub explain: String,
    /// safe | cost | caution
    pub risk: String,
    /// 需要管理员权限的清理项,普通权限下置灰展示(需求文档 §3.5 权限矩阵)
    #[serde(default)]
    pub needs_admin: bool,
    /// 引导型清理项:本工具不代删,explain 教用户手动操作。
    /// 前端默认不勾且禁用勾选(数字承诺才真实);后端执行时无条件跳过(最后防线)
    #[serde(default)]
    pub guide_only: bool,
    #[serde(default)]
    pub related_processes: Vec<String>,
    /// clean | migrate | guide
    pub action: String,
}

/// 规则内置于二进制,随版本更新(需求文档 §4:首版不做在线热更新)
pub fn load_rules() -> Vec<Rule> {
    serde_json::from_str(include_str!("../rules/rules.json"))
        .expect("rules.json 内置规则解析失败,属打包错误")
}

/// 把 %USERPROFILE% 等占位符展开为当前用户的绝对路径。
/// 环境变量缺失(如无用户配置的服务会话)时该规则不参与匹配。
/// `special:` 前缀不是文件路径(如回收站走 Shell API),由 cleaner 按 id 特判。
pub fn expand_pattern(pattern: &str) -> Option<PathBuf> {
    if pattern.starts_with("special:") {
        return None;
    }
    let mut out = pattern.to_string();
    for var in ["USERPROFILE", "APPDATA", "LOCALAPPDATA", "WINDIR", "SystemDrive"] {
        let holder = format!("%{var}%");
        if out.contains(&holder) {
            let val = std::env::var(var).ok()?;
            out = out.replace(&holder, &val);
        }
    }
    Some(PathBuf::from(out))
}
