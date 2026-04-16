//! 输入候选（composer options）相关 DTO。
//!
//! 这些 DTO 服务于前端输入框的候选面板，而不是运行时内部的 prompt 组装。
//! 单独建模的原因是：UI 需要一个稳定、轻量的“可选项投影视图”，
//! 不能直接复用 `SkillSpec` / `CapabilityWireDescriptor` 这类内部结构。

use serde::{Deserialize, Serialize};

/// 输入候选项的来源类别。
///
/// `skill` 表示具体的 skill 条目，而不是 `Skill` 加载器 capability 本身。
/// 保留独立枚举可以明确区分“prompt 资源”与“可调用 capability”两个层次。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ComposerOptionKindDto {
    Command,
    Skill,
    Capability,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ComposerOptionActionKindDto {
    InsertText,
    ExecuteCommand,
}

/// 单个输入候选项。
///
/// `insert_text` 是前端选择该项后建议写回输入框的文本。
/// `badges` / `keywords` 让 UI 和本地搜索可以使用统一载荷，
/// 避免前端再去推断来源、标签或 profile。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ComposerOptionDto {
    pub kind: ComposerOptionKindDto,
    pub id: String,
    pub title: String,
    pub description: String,
    pub insert_text: String,
    pub action_kind: ComposerOptionActionKindDto,
    pub action_value: String,
    #[serde(default)]
    pub badges: Vec<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
}

/// 输入候选列表响应。
///
/// 预留响应外层对象而非直接返回数组，是为了后续在不破坏协议的前提下
/// 增加服务端元数据（例如 query 规范化结果或分页信息）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ComposerOptionsResponseDto {
    pub items: Vec<ComposerOptionDto>,
}
