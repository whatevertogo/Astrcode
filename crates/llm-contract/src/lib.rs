use std::sync::Arc;

use astrcode_core::{
    CancelToken, LlmMessage, ReasoningContent, Result, ToolCallRequest, ToolDefinition,
};
use astrcode_governance_contract::SystemPromptBlock;
pub use astrcode_prompt_contract::{
    PromptCacheBreakReason, PromptCacheDiagnostics, PromptCacheGlobalStrategy, PromptCacheHints,
    PromptLayerFingerprints,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// runtime owner 的 provider 能力限制。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelLimits {
    pub context_window: usize,
    pub max_output_tokens: usize,
}

/// 模型 token 使用统计。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmUsage {
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub cache_creation_input_tokens: usize,
    pub cache_read_input_tokens: usize,
}

impl LlmUsage {
    pub fn total_tokens(self) -> usize {
        self.input_tokens.saturating_add(self.output_tokens)
    }
}

/// LLM 输出结束原因。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LlmFinishReason {
    #[default]
    Stop,
    MaxTokens,
    ToolCalls,
    Other(String),
}

impl LlmFinishReason {
    pub fn is_max_tokens(&self) -> bool {
        matches!(self, Self::MaxTokens)
    }

    pub fn from_api_value(value: &str) -> Self {
        match value {
            "stop" => Self::Stop,
            "max_tokens" | "length" => Self::MaxTokens,
            "tool_calls" => Self::ToolCalls,
            other => Self::Other(other.to_string()),
        }
    }
}

/// provider 流式增量事件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LlmEvent {
    TextDelta(String),
    ThinkingDelta(String),
    ThinkingSignature(String),
    StreamRetryStarted {
        attempt: u32,
        max_attempts: u32,
        reason: String,
    },
    ToolCallDelta {
        index: usize,
        id: Option<String>,
        name: Option<String>,
        arguments_delta: String,
    },
}

pub type LlmEventSink = Arc<dyn Fn(LlmEvent) + Send + Sync>;

/// 模型调用请求。
#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub messages: Vec<LlmMessage>,
    pub tools: Arc<[ToolDefinition]>,
    pub cancel: CancelToken,
    pub system_prompt: Option<String>,
    pub system_prompt_blocks: Vec<SystemPromptBlock>,
    pub prompt_cache_hints: Option<PromptCacheHints>,
    pub max_output_tokens_override: Option<usize>,
    pub skip_cache_write: bool,
}

impl LlmRequest {
    pub fn new(
        messages: Vec<LlmMessage>,
        tools: impl Into<Arc<[ToolDefinition]>>,
        cancel: CancelToken,
    ) -> Self {
        Self {
            messages,
            tools: tools.into(),
            cancel,
            system_prompt: None,
            system_prompt_blocks: Vec::new(),
            prompt_cache_hints: None,
            max_output_tokens_override: None,
            skip_cache_write: false,
        }
    }

    pub fn with_system(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    pub fn with_max_output_tokens_override(mut self, max_output_tokens: usize) -> Self {
        self.max_output_tokens_override = Some(max_output_tokens.max(1));
        self
    }

    pub fn with_skip_cache_write(mut self, skip_cache_write: bool) -> Self {
        self.skip_cache_write = skip_cache_write;
        self
    }
}

/// 模型调用输出。
#[derive(Debug, Clone, Default)]
pub struct LlmOutput {
    pub content: String,
    pub tool_calls: Vec<ToolCallRequest>,
    pub reasoning: Option<ReasoningContent>,
    pub usage: Option<LlmUsage>,
    pub finish_reason: LlmFinishReason,
    pub prompt_cache_diagnostics: Option<PromptCacheDiagnostics>,
}

/// agent-runtime 消费的抽象 provider stream surface。
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn generate(&self, request: LlmRequest, sink: Option<LlmEventSink>) -> Result<LlmOutput>;
    fn model_limits(&self) -> ModelLimits;
    fn supports_cache_metrics(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::{LlmFinishReason, LlmUsage};

    #[test]
    fn usage_total_saturates() {
        let usage = LlmUsage {
            input_tokens: usize::MAX,
            output_tokens: 1,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        };

        assert_eq!(usage.total_tokens(), usize::MAX);
    }

    #[test]
    fn finish_reason_accepts_openai_family_values() {
        assert!(LlmFinishReason::from_api_value("length").is_max_tokens());
        assert_eq!(
            LlmFinishReason::from_api_value("tool_calls"),
            LlmFinishReason::ToolCalls
        );
    }
}
