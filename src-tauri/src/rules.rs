use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 知识库规则,schema 见 docs/设计规范.md §7
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    pub id: String,
    pub path_pattern: String,
    pub display_name: String,
    pub explain: String,
    /// safe | cost | caution
    pub risk: String,
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
pub fn expand_pattern(pattern: &str) -> Option<PathBuf> {
    let mut out = pattern.to_string();
    for var in ["USERPROFILE", "APPDATA", "LOCALAPPDATA"] {
        let holder = format!("%{var}%");
        if out.contains(&holder) {
            let val = std::env::var(var).ok()?;
            out = out.replace(&holder, &val);
        }
    }
    Some(PathBuf::from(out))
}
