//! Composer 输入补全用例。
//!
//! 提供 composer 输入候选列表的查询和过滤用例。
//! 候选来源包括：命令、技能、能力（通过 `KernelGateway` 查询）。

use astrcode_kernel::KernelGateway;

// ============================================================
// 业务模型
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComposerOptionKind {
    Command,
    Skill,
    Capability,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComposerOptionActionKind {
    InsertText,
    ExecuteCommand,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerOptionsRequest {
    pub query: Option<String>,
    pub kinds: Vec<ComposerOptionKind>,
    pub limit: usize,
}

impl Default for ComposerOptionsRequest {
    fn default() -> Self {
        Self {
            query: None,
            kinds: Vec::new(),
            limit: 50,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerOption {
    pub kind: ComposerOptionKind,
    pub id: String,
    pub title: String,
    pub description: String,
    pub insert_text: String,
    pub action_kind: ComposerOptionActionKind,
    pub action_value: String,
    pub badges: Vec<String>,
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerSkillSummary {
    pub id: String,
    pub description: String,
}

impl ComposerSkillSummary {
    pub fn new(id: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            description: description.into(),
        }
    }
}

// ============================================================
// Composer 用例服务
// ============================================================

/// Composer 输入补全用例服务。
pub struct ComposerService {
    builtin_commands: Vec<ComposerOption>,
}

impl Default for ComposerService {
    fn default() -> Self {
        Self::new()
    }
}

impl ComposerService {
    pub fn new() -> Self {
        Self {
            builtin_commands: vec![ComposerOption {
                kind: ComposerOptionKind::Command,
                id: "compact".to_string(),
                title: "压缩上下文".to_string(),
                description: "压缩当前会话上下文".to_string(),
                insert_text: "/compact".to_string(),
                action_kind: ComposerOptionActionKind::ExecuteCommand,
                action_value: "/compact".to_string(),
                badges: vec!["built-in".to_string()],
                keywords: vec!["compact".to_string(), "compress".to_string()],
            }],
        }
    }

    /// 用例：列出可用的 composer 选项。
    ///
    /// 合并内置命令和通过 kernel gateway 查询到的能力选项，
    /// 然后按 kind 和 query 过滤。
    pub fn list_options(
        &self,
        request: ComposerOptionsRequest,
        skill_summaries: Vec<ComposerSkillSummary>,
        gateway: Option<&KernelGateway>,
    ) -> Vec<ComposerOption> {
        let mut items = self.builtin_commands.clone();
        items.extend(skill_summaries.into_iter().map(skill_summary_to_option));

        if let Some(gateway) = gateway {
            for spec in gateway.capabilities().capability_specs() {
                let name_str = spec.name.to_string();
                items.push(ComposerOption {
                    kind: ComposerOptionKind::Capability,
                    id: name_str.clone(),
                    title: name_str.clone(),
                    description: spec.description.clone(),
                    insert_text: name_str.clone(),
                    action_kind: ComposerOptionActionKind::InsertText,
                    action_value: name_str.clone(),
                    badges: vec!["capability".to_string()],
                    keywords: vec![name_str.to_lowercase()],
                });
            }
        }

        if !request.kinds.is_empty() {
            items.retain(|item| request.kinds.contains(&item.kind));
        }

        if let Some(query) = request.query {
            let query = query.to_lowercase();
            items.retain(|item| {
                item.id.to_lowercase().contains(&query)
                    || item.title.to_lowercase().contains(&query)
                    || item.description.to_lowercase().contains(&query)
                    || item
                        .keywords
                        .iter()
                        .any(|kw| kw.to_lowercase().contains(&query))
            });
        }

        items.truncate(request.limit);
        items
    }
}

fn skill_summary_to_option(skill: ComposerSkillSummary) -> ComposerOption {
    let keywords = skill
        .id
        .split('-')
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    ComposerOption {
        kind: ComposerOptionKind::Skill,
        title: humanize_skill_title(&skill.id),
        insert_text: format!("/{}", skill.id),
        action_kind: ComposerOptionActionKind::InsertText,
        action_value: format!("/{}", skill.id),
        badges: vec!["skill".to_string()],
        keywords,
        id: skill.id,
        description: skill.description,
    }
}

fn humanize_skill_title(skill_id: &str) -> String {
    let words = skill_id
        .split('-')
        .filter(|segment| !segment.is_empty())
        .map(title_case_token)
        .collect::<Vec<_>>();
    if words.is_empty() {
        skill_id.to_string()
    } else {
        words.join(" ")
    }
}

fn title_case_token(token: &str) -> String {
    let mut chars = token.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let rest = chars.collect::<String>();
    if first.is_ascii_alphabetic() {
        format!("{}{}", first.to_ascii_uppercase(), rest)
    } else {
        format!("{first}{rest}")
    }
}

impl std::fmt::Debug for ComposerService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComposerService")
            .field("builtin_commands", &self.builtin_commands.len())
            .finish()
    }
}
