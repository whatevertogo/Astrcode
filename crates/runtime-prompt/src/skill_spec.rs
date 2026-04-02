//! Skill 规格定义与名称校验。
//!
//! 本模块定义了 skill 的核心数据结构 [`SkillSpec`]，以及 skill 名称的校验和规范化逻辑。
//!
//! # Skill 名称规则
//!
//! Skill 名称必须为 kebab-case（小写字母、数字、连字符），且必须与文件夹名一致。
//! 这是 Claude-style skill 的约定，确保名称的一致性和可预测性。

use serde::{Deserialize, Serialize};

/// Skill 的来源。
///
/// 用于追踪 skill 是从哪里加载的，影响覆盖优先级和诊断标签。
/// 优先级顺序：Builtin < User < Project < Plugin < Mcp（后者覆盖前者）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SkillSource {
    #[default]
    Builtin,
    User,
    Project,
    Plugin,
    Mcp,
}

impl SkillSource {
    pub fn as_tag(&self) -> &'static str {
        match self {
            Self::Builtin => "source:builtin",
            Self::User => "source:user",
            Self::Project => "source:project",
            Self::Plugin => "source:plugin",
            Self::Mcp => "source:mcp",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillSpec {
    /// Skill 的唯一标识符（kebab-case，与文件夹名一致）。
    pub id: String,
    /// Skill 的显示名称（与 `id` 相同）。
    pub name: String,
    /// Skill 的简短描述，用于 system prompt 中的索引展示。
    ///
    /// 这是两阶段 skill 模型的第一阶段：模型通过描述判断何时调用 `Skill` tool。
    pub description: String,
    /// Skill 的完整指南正文。
    ///
    /// 在两阶段模型中，只有当模型调用 `Skill` tool 时才加载此内容。
    pub guide: String,
    /// Skill 目录的根路径（运行时填充）。
    pub skill_root: Option<String>,
    /// Skill 目录中的资产文件列表（如 `references/`、`scripts/` 下的文件）。
    pub asset_files: Vec<String>,
    /// 此 skill 允许调用的工具列表。
    ///
    /// 用于限制 skill 执行时的能力边界，builtin skill 由 `build.rs` 配置。
    pub allowed_tools: Vec<String>,
    /// Skill 的来源，影响覆盖优先级和诊断标签。
    pub source: SkillSource,
}

impl SkillSpec {
    /// 检查此 skill 是否匹配请求的名称。
    ///
    /// 比较时进行大小写不敏感和斜杠容忍处理，
    /// 使得 `/repo-search` 和 `REPO SEARCH` 都能匹配 `repo-search`。
    pub fn matches_requested_name(&self, requested_name: &str) -> bool {
        let requested_name = normalize_skill_name(requested_name);
        // `id` is already validated as kebab-case at parse time, so normalize
        // is strictly for the caller-provided side — both sides land in the
        // same canonical form for comparison.
        requested_name == normalize_skill_name(&self.id)
    }
}

/// 检查名称是否为合法的 skill 名称。
///
/// 合法名称仅允许小写 ASCII 字母、数字和连字符，且不能为空。
/// 这是 Claude-style skill 的强制要求。
pub fn is_valid_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
}

/// 规范化 skill 名称。
///
/// 将输入转换为小写、去除首尾空白和前导斜杠、将非字母数字字符替换为空格后合并。
/// 用于用户输入与 skill id 的模糊匹配。
pub fn normalize_skill_name(value: &str) -> String {
    value
        .trim()
        .trim_start_matches('/')
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || (!ch.is_ascii() && ch.is_alphanumeric()) {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project_skill(id: &str, name: &str, description: &str) -> SkillSpec {
        SkillSpec {
            id: id.to_string(),
            name: name.to_string(),
            description: description.to_string(),
            guide: "guide".to_string(),
            skill_root: None,
            asset_files: Vec::new(),
            allowed_tools: Vec::new(),
            source: SkillSource::Project,
        }
    }

    #[test]
    fn skill_name_matching_is_case_insensitive_and_slash_tolerant() {
        let skill = project_skill("repo-search", "repo-search", "Search the repo");

        assert!(skill.matches_requested_name("repo-search"));
        assert!(skill.matches_requested_name("/repo-search"));
        assert!(skill.matches_requested_name("REPO SEARCH"));
        assert!(!skill.matches_requested_name("edit-file"));
    }

    #[test]
    fn validates_claude_style_skill_names() {
        assert!(is_valid_skill_name("git-commit"));
        assert!(is_valid_skill_name("pdf2"));
        assert!(!is_valid_skill_name("Git-Commit"));
        assert!(!is_valid_skill_name("git_commit"));
        assert!(!is_valid_skill_name(""));
    }
}
