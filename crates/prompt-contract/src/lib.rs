//! Prompt 合约类型。
//!
//! `SystemPromptLayer`、`PromptDeclaration` 系列类型重导出自 `astrcode_core`，
//! 消除双重定义。本 crate 保留 `PromptCacheHints`、`PromptLayerFingerprints`、
//! `PromptCacheGlobalStrategy`、`PromptCacheBreakReason`、`PromptCacheDiagnostics`
//! 供缓存优化使用。这些类型未来可能合并到 `astrcode-llm-contract`。

pub use astrcode_core::{
    policy::SystemPromptLayer,
    prompt::{
        PromptCacheBreakReason, PromptCacheDiagnostics, PromptDeclaration, PromptDeclarationKind,
        PromptDeclarationRenderTarget, PromptDeclarationSource,
    },
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PromptLayerFingerprints {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stable: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semi_stable: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherited: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PromptCacheHints {
    #[serde(default)]
    pub layer_fingerprints: PromptLayerFingerprints,
    #[serde(default)]
    pub global_cache_strategy: PromptCacheGlobalStrategy,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unchanged_layers: Vec<SystemPromptLayer>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub compacted: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub tool_result_rebudgeted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PromptCacheGlobalStrategy {
    #[default]
    SystemPrompt,
    ToolBased,
}

fn is_false(value: &bool) -> bool {
    !*value
}
