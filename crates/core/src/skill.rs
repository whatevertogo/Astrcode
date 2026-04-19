//! Skill 领域模型与查询端口共享的稳定类型。

use serde::{Deserialize, Serialize};

/// Skill 的来源。
///
/// 用于追踪 skill 是从哪里加载的，影响覆盖优先级和诊断标签。
/// 优先级顺序：Builtin < Mcp < Plugin < User < Project（后者覆盖前者）。
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
    pub id: String,
    pub name: String,
    pub description: String,
    pub guide: String,
    pub skill_root: Option<String>,
    pub asset_files: Vec<String>,
    pub allowed_tools: Vec<String>,
    pub source: SkillSource,
}

impl SkillSpec {
    pub fn matches_requested_name(&self, requested_name: &str) -> bool {
        let requested_name = normalize_skill_name(requested_name);
        requested_name == normalize_skill_name(&self.id)
    }
}

pub fn is_valid_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
}

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
