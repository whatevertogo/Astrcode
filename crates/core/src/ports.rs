//! 运行时稳定端口契约。
//!
//! 这些 trait 定义在 `core`，由 adapter 层实现，由 kernel/session-runtime/application
//! 通过依赖倒置消费，避免上层再反向依赖具体实现 crate。

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    CancelToken, CapabilitySpec, Config, ConfigOverlay, DeleteProjectResult, LlmMessage,
    ReasoningContent, Result, SessionId, SessionMeta, SessionTurnAcquireResult, StorageEvent,
    StoredEvent, SystemPromptBlock, SystemPromptLayer, ToolCallRequest, ToolDefinition, TurnId,
};

/// MCP 配置文件作用域。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpConfigFileScope {
    User,
    Project,
    Local,
}

/// EventStore 端口。
#[async_trait]
pub trait EventStore: Send + Sync {
    async fn ensure_session(&self, session_id: &SessionId, working_dir: &Path) -> Result<()>;
    async fn append(&self, session_id: &SessionId, event: &StorageEvent) -> Result<StoredEvent>;
    async fn replay(&self, session_id: &SessionId) -> Result<Vec<StoredEvent>>;
    async fn try_acquire_turn(
        &self,
        session_id: &SessionId,
        turn_id: &str,
    ) -> Result<SessionTurnAcquireResult>;
    async fn list_sessions(&self) -> Result<Vec<SessionId>>;
    async fn list_session_metas(&self) -> Result<Vec<SessionMeta>>;
    async fn delete_session(&self, session_id: &SessionId) -> Result<()>;
    async fn delete_sessions_by_working_dir(
        &self,
        working_dir: &str,
    ) -> Result<DeleteProjectResult>;
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
    pub tools: Arc<[ToolDefinition]>,
    pub cancel: CancelToken,
    pub system_prompt: Option<String>,
    pub system_prompt_blocks: Vec<SystemPromptBlock>,
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PromptEntrySummary {
    pub id: String,
    pub description: String,
}

impl PromptEntrySummary {
    pub fn new(id: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            description: description.into(),
        }
    }
}

pub type PromptSkillSummary = PromptEntrySummary;

/// Prompt 侧的轻量 agent profile 摘要。
pub type PromptAgentProfileSummary = PromptEntrySummary;

/// Prompt 声明来源。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PromptDeclarationSource {
    Builtin,
    #[default]
    Plugin,
    Mcp,
}

/// Prompt 声明语义类型。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PromptDeclarationKind {
    ToolGuide,
    #[default]
    ExtensionInstruction,
}

/// Prompt 声明渲染目标。
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

/// 稳定的 Prompt 声明 DTO。
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

/// Prompt 事实查询请求。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PromptGovernanceContext {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_capability_names: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode_id: Option<crate::ModeId>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub approval_mode: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub policy_revision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_subrun_depth: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_spawn_per_turn: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PromptFactsRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    pub working_dir: PathBuf,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_capability_names: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub governance: Option<PromptGovernanceContext>,
}

/// Prompt 组装前的已解析事实。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptFacts {
    pub profile: String,
    #[serde(default)]
    pub profile_context: Value,
    #[serde(default)]
    pub metadata: Value,
    #[serde(default)]
    pub skills: Vec<PromptSkillSummary>,
    #[serde(default)]
    pub agent_profiles: Vec<PromptAgentProfileSummary>,
    #[serde(default)]
    pub prompt_declarations: Vec<PromptDeclaration>,
}

impl Default for PromptFacts {
    fn default() -> Self {
        Self {
            profile: "coding".to_string(),
            profile_context: Value::Null,
            metadata: Value::Null,
            skills: Vec::new(),
            agent_profiles: Vec::new(),
            prompt_declarations: Vec::new(),
        }
    }
}

fn is_unspecified_prompt_layer(layer: &SystemPromptLayer) -> bool {
    matches!(layer, SystemPromptLayer::Unspecified)
}

/// Prompt facts provider 端口。
#[async_trait]
pub trait PromptFactsProvider: Send + Sync {
    async fn resolve_prompt_facts(&self, request: &PromptFactsRequest) -> Result<PromptFacts>;
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
    pub step_index: usize,
    #[serde(default)]
    pub turn_index: usize,
    #[serde(default)]
    pub profile_context: Value,
    #[serde(default)]
    pub capabilities: Vec<CapabilitySpec>,
    #[serde(default)]
    pub skills: Vec<PromptSkillSummary>,
    #[serde(default)]
    pub agent_profiles: Vec<PromptAgentProfileSummary>,
    #[serde(default)]
    pub prompt_declarations: Vec<PromptDeclaration>,
    #[serde(default)]
    pub metadata: Value,
}

/// Prompt 组装结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PromptBuildCacheMetrics {
    pub reuse_hits: u32,
    pub reuse_misses: u32,
}

/// Prompt 组装结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptBuildOutput {
    pub system_prompt: String,
    #[serde(default)]
    pub system_prompt_blocks: Vec<SystemPromptBlock>,
    #[serde(default)]
    pub cache_metrics: PromptBuildCacheMetrics,
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

/// 配置存储端口。
///
/// 将配置文件 IO 从 application 层剥离，由 adapter 层实现。
pub trait ConfigStore: Send + Sync {
    /// 从磁盘加载配置（文件不存在时创建默认配置）。
    fn load(&self) -> Result<Config>;
    /// 保存配置到磁盘（原子写入）。
    fn save(&self, config: &Config) -> Result<()>;
    /// 返回配置文件路径。
    fn path(&self) -> PathBuf;
    /// 加载项目 overlay（文件存在时）。
    fn load_overlay(&self, working_dir: &std::path::Path) -> Result<Option<ConfigOverlay>>;
    /// 保存项目 overlay；当值为空时允许实现删除文件。
    fn save_overlay(&self, working_dir: &std::path::Path, overlay: &ConfigOverlay) -> Result<()>;
    /// 读取指定作用域的独立 MCP 原始配置。
    fn load_mcp(
        &self,
        scope: McpConfigFileScope,
        working_dir: Option<&std::path::Path>,
    ) -> Result<Option<Value>>;
    /// 保存指定作用域的独立 MCP 原始配置；当值为空时允许实现删除文件。
    fn save_mcp(
        &self,
        scope: McpConfigFileScope,
        working_dir: Option<&std::path::Path>,
        mcp: Option<&Value>,
    ) -> Result<()>;
}
