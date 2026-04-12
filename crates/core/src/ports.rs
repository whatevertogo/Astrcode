//! 运行时稳定端口契约。
//!
//! 这些 trait 定义在 `core`，由 adapter 层实现，由 kernel/session-runtime/application
//! 通过依赖倒置消费，避免上层再反向依赖具体实现 crate。

use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    CancelToken, CapabilitySpec, LlmMessage, ReasoningContent, Result, SessionId, StorageEvent,
    StoredEvent, SystemPromptBlock, ToolCallRequest, ToolDefinition, TurnId,
};

/// EventStore 端口。
#[async_trait]
pub trait EventStore: Send + Sync {
    async fn append(&self, session_id: &SessionId, event: &StorageEvent) -> Result<StoredEvent>;
    async fn replay(&self, session_id: &SessionId) -> Result<Vec<StoredEvent>>;
    async fn list_sessions(&self) -> Result<Vec<SessionId>>;
    async fn delete_session(&self, session_id: &SessionId) -> Result<()>;
}

/// 模型能力限制。
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

/// 流式增量事件。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LlmEvent {
    TextDelta(String),
    ThinkingDelta(String),
    ThinkingSignature(String),
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
    pub tools: Vec<ToolDefinition>,
    pub cancel: CancelToken,
    pub system_prompt: Option<String>,
    pub system_prompt_blocks: Vec<SystemPromptBlock>,
}

impl LlmRequest {
    pub fn new(messages: Vec<LlmMessage>, tools: Vec<ToolDefinition>, cancel: CancelToken) -> Self {
        Self {
            messages,
            tools,
            cancel,
            system_prompt: None,
            system_prompt_blocks: Vec::new(),
        }
    }

    pub fn with_system(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
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
}

/// LLM provider 端口。
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn generate(&self, request: LlmRequest, sink: Option<LlmEventSink>) -> Result<LlmOutput>;
    fn model_limits(&self) -> ModelLimits;
    fn supports_cache_metrics(&self) -> bool {
        false
    }
}

/// Prompt 组装请求。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptBuildRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    pub working_dir: PathBuf,
    pub profile: String,
    #[serde(default)]
    pub profile_context: Value,
    #[serde(default)]
    pub capabilities: Vec<CapabilitySpec>,
    #[serde(default)]
    pub metadata: Value,
}

/// Prompt 组装结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptBuildOutput {
    pub system_prompt: String,
    #[serde(default)]
    pub system_prompt_blocks: Vec<SystemPromptBlock>,
    #[serde(default)]
    pub metadata: Value,
}

/// Prompt provider 端口。
#[async_trait]
pub trait PromptProvider: Send + Sync {
    async fn build_prompt(&self, request: PromptBuildRequest) -> Result<PromptBuildOutput>;
}

/// 资源读取请求上下文。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ResourceRequestContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub metadata: Value,
}

/// 资源读取结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceReadResult {
    pub uri: String,
    pub content: Value,
    #[serde(default)]
    pub metadata: Value,
}

/// Resource provider 端口。
#[async_trait]
pub trait ResourceProvider: Send + Sync {
    async fn read_resource(
        &self,
        uri: &str,
        context: &ResourceRequestContext,
    ) -> Result<ResourceReadResult>;
}
