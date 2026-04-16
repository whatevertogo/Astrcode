use serde::{Deserialize, Serialize};

/// 输入候选项的来源类别。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ComposerOptionKind {
    Command,
    Skill,
    Capability,
}

/// 输入候选项被选择后的动作类型。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ComposerOptionActionKind {
    InsertText,
    ExecuteCommand,
}

/// 单个输入候选项。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ComposerOption {
    pub kind: ComposerOptionKind,
    pub id: String,
    pub title: String,
    pub description: String,
    pub insert_text: String,
    pub action_kind: ComposerOptionActionKind,
    pub action_value: String,
    #[serde(default)]
    pub badges: Vec<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
}
