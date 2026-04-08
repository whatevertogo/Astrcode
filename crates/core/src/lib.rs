//! # Astrcode 核心库
//!
//! 本库定义了 Astrcode 系统的核心领域模型和接口，与具体的运行时实现解耦。
//!
//! ## 主要模块
//!
//! - [`event`][]: 事件存储与回放系统（JSONL append-only 日志）
//! - [`session`][]: 会话管理与持久化
//! - [`tool`][]: Tool trait 定义（插件系统的基础抽象）
//! - [`policy`][]: 策略引擎 trait（审批与模型/工具请求检查）
//! - [`plugin`][]: 插件清单与注册表
//! - [`registry`][]: 能力路由器（将能力调用分派到具体的 invoker）
//! - [`runtime`][]: 运行时协调器接口
//! - [`projection`][]: Agent 状态投影（从事件流推导状态）
//! - `action`: LLM 消息与工具调用相关的数据结构
//!
//! 注意：能力描述符（capability）定义在 `astrcode-protocol` crate 中。

mod action;
pub mod agent;
mod cancel;
mod compact_summary;
pub mod env;
mod error;
pub mod event;
pub mod home;
pub mod hook;
pub mod local_server;
pub mod plugin;
pub mod policy;
pub mod project;
pub mod projection;
pub mod registry;
pub mod runtime;
pub mod session;
pub mod store;
mod time;
// test_support 通过 feature gate "test-support" 守卫。
// 其他 crate 在 dev-dependencies 中启用此 feature：astrcode-core = { features = ["test-support"]
// }。 发布构建默认不启用，tempfile 不会被编译进产物。
pub mod support;
#[cfg(feature = "test-support")]
pub mod test_support;
mod tool;

pub use action::{
    AssistantContentParts, LlmMessage, ReasoningContent, ToolCallRequest, ToolDefinition,
    ToolExecutionResult, ToolOutputDelta, ToolOutputStream, UserMessageOrigin,
    split_assistant_content,
};
pub use agent::{
    AgentEventContext, AgentMode, AgentProfile, AgentProfileCatalog, AgentStatus, ArtifactRef,
    ChildAgentRef, ChildSessionLineageKind, ChildSessionNode, ChildSessionStatusSource, ForkMode,
    InvocationKind, ResolvedExecutionLimitsSnapshot, ResolvedSubagentContextOverrides,
    SpawnAgentParams, SubRunDescriptor, SubRunFailure, SubRunFailureCode, SubRunHandle,
    SubRunHandoff, SubRunOutcome, SubRunResult, SubRunStorageMode, SubagentContextOverrides,
};
pub use cancel::CancelToken;
pub use compact_summary::{
    COMPACT_SUMMARY_CONTINUATION, COMPACT_SUMMARY_PREFIX, CompactSummaryEnvelope,
    format_compact_summary, parse_compact_summary_message,
};
pub use error::{AstrError, Result, ResultExt};
pub use event::{
    AgentEvent, CompactTrigger, EventTranslator, Phase, StorageEvent, StoredEvent, StoredEventLine,
    generate_session_id, phase_of_storage_event, replay_records,
};
pub use hook::{
    CompactionHookContext, CompactionHookResultContext, HookCompactionReason, HookEvent,
    HookHandler, HookInput, HookOutcome, ToolHookContext, ToolHookResultContext,
};
pub use local_server::{LOCAL_SERVER_READY_PREFIX, LocalServerInfo};
pub use plugin::{PluginHealth, PluginManifest, PluginRegistry, PluginState, PluginType};
pub use policy::{
    AllowAllPolicyEngine, ApprovalDefault, ApprovalPending, ApprovalRequest, ApprovalResolution,
    CapabilityCall, ContextDecisionInput, ContextStrategy, ModelRequest, PolicyContext,
    PolicyEngine, PolicyVerdict, SystemPromptBlock, SystemPromptLayer,
};
pub use projection::{AgentState, AgentStateProjector, project};
pub use registry::{CapabilityContext, CapabilityExecutionResult, CapabilityInvoker};
pub use runtime::{
    ExecutionOrchestrationBoundary, LiveSubRunControlBoundary, LoopRunnerBoundary,
    ManagedRuntimeComponent, PromptAccepted, RootExecutionAccepted, RuntimeCoordinator,
    RuntimeHandle, SessionTruthBoundary,
};
pub use session::{DeleteProjectResult, SessionEventRecord, SessionMeta};
pub use store::{
    EventLogWriter, SessionManager, SessionTurnAcquireResult, SessionTurnBusy, SessionTurnLease,
    StoreError, StoreResult,
};
pub use time::{
    format_local_rfc3339, format_local_rfc3339_opt, local_rfc3339, local_rfc3339_option,
};
pub use tool::{
    DEFAULT_MAX_OUTPUT_SIZE, ExecutionOwner, SessionId, Tool, ToolCapabilityMetadata, ToolContext,
    ToolEventSink, ToolPromptMetadata,
};
