use serde::{Deserialize, Serialize};

use crate::policy::SystemPromptLayer;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptCacheBreakReason {
    SystemPromptChanged,
    ToolSchemasChanged,
    ModelChanged,
    GlobalCacheStrategyChanged,
    CompactedPrompt,
    ToolResultRebudgeted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PromptCacheDiagnostics {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<PromptCacheBreakReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_cache_read_input_tokens: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_cache_read_input_tokens: Option<usize>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub expected_drop: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub cache_break_detected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PromptDeclarationSource {
    Builtin,
    #[default]
    Plugin,
    Mcp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PromptDeclarationKind {
    ToolGuide,
    #[default]
    ExtensionInstruction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PromptDeclarationRenderTarget {
    #[default]
    System,
    PrependUser,
    PrependAssistant,
    AppendUser,
    AppendAssistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PromptDeclaration {
    pub block_id: String,
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub render_target: PromptDeclarationRenderTarget,
    #[serde(default, skip_serializing_if = "is_unspecified_prompt_layer")]
    pub layer: SystemPromptLayer,
    #[serde(default)]
    pub kind: PromptDeclarationKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority_hint: Option<i32>,
    #[serde(default)]
    pub always_include: bool,
    #[serde(default)]
    pub source: PromptDeclarationSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

fn is_unspecified_prompt_layer(layer: &SystemPromptLayer) -> bool {
    matches!(layer, SystemPromptLayer::Unspecified)
}

fn is_false(value: &bool) -> bool {
    !*value
}
