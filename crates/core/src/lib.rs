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

mod action;
pub mod agent;
mod cancel;
pub mod capability;
mod compact_summary;
mod composer;
pub mod config;
pub mod env;
mod error;
pub mod event;
mod execution_control;
mod execution_result;
pub mod home;
pub mod hook;
pub mod ids;
pub mod local_server;
pub mod mode;
pub mod observability;
pub mod plugin;
pub mod policy;
pub mod ports;
pub mod project;
pub mod projection;
pub mod registry;
pub mod runtime;
pub mod session;
mod session_catalog;
mod shell;
pub mod store;
mod time;
// test_support 通过 feature gate "test-support" 守卫。
// 其他 crate 在 dev-dependencies 中启用此 feature：astrcode-core = { features = ["test-support"]
// }。 发布构建默认不启用，tempfile 不会被编译进产物。
pub mod support;
#[cfg(feature = "test-support")]
pub mod test_support;
mod tool;
pub mod tool_result_persist;

pub use action::{
    AssistantContentParts, LlmMessage, ReasoningContent, ToolCallRequest, ToolDefinition,
    ToolExecutionResult, ToolOutputDelta, ToolOutputStream, UserMessageOrigin,
    split_assistant_content,
};
pub use agent::{
    AgentCollaborationActionKind, AgentCollaborationFact, AgentCollaborationOutcomeKind,
    AgentCollaborationPolicyContext, AgentEventContext, AgentInboxEnvelope, AgentMode,
    AgentProfile, AgentProfileCatalog, ArtifactRef, ChildAgentRef, ChildExecutionIdentity,
    ChildSessionLineageKind, ChildSessionNode, ChildSessionNotification,
    ChildSessionNotificationKind, ChildSessionStatusSource, CloseAgentParams,
    CloseRequestParentDeliveryPayload, CollaborationResult, CompletedParentDeliveryPayload,
    CompletedSubRunOutcome, DelegationMetadata, FailedParentDeliveryPayload, FailedSubRunOutcome,
    ForkMode, InboxEnvelopeKind, InvocationKind, LineageSnapshot, ParentDelivery,
    ParentDeliveryKind, ParentDeliveryOrigin, ParentDeliveryPayload,
    ParentDeliveryTerminalSemantics, ParentExecutionRef, ProgressParentDeliveryPayload,
    ResolvedExecutionLimitsSnapshot, ResolvedSubagentContextOverrides, SendAgentParams,
    SendToChildParams, SendToParentParams, SpawnAgentParams, SpawnCapabilityGrant, SubRunFailure,
    SubRunFailureCode, SubRunHandle, SubRunHandoff, SubRunResult, SubRunStatus, SubRunStorageMode,
    SubagentContextOverrides,
    executor::{CollaborationExecutor, SubAgentExecutor},
    input_queue::{
        BatchId, CloseParams, DeliveryId, InputBatchAckedPayload, InputBatchStartedPayload,
        InputDiscardedPayload, InputQueueProjection, InputQueuedPayload, ObserveParams,
        ObserveSnapshot, QueuedInputEnvelope, SendParams,
    },
    lifecycle::{AgentLifecycleStatus, AgentTurnOutcome},
    normalize_non_empty_unique_string_list,
};
pub use cancel::CancelToken;
pub use capability::{
    CapabilityKind, CapabilitySpec, CapabilitySpecBuildError, CapabilitySpecBuilder,
    InvocationMode, PermissionSpec, SideEffect, Stability,
};
pub use compact_summary::{
    COMPACT_SUMMARY_CONTINUATION, COMPACT_SUMMARY_PREFIX, CompactSummaryEnvelope,
    format_compact_summary, parse_compact_summary_message,
};
pub use composer::{ComposerOption, ComposerOptionActionKind, ComposerOptionKind};
pub use config::{
    ActiveSelection, AgentConfig, Config, ConfigOverlay, CurrentModelSelection, ModelConfig,
    ModelOption, ModelSelection, Profile, ResolvedAgentConfig, ResolvedRuntimeConfig,
    RuntimeConfig, TestConnectionResult, max_tool_concurrency, resolve_agent_config,
    resolve_runtime_config,
};
pub use error::{AstrError, Result, ResultExt};
pub use event::{
    AgentEvent, CompactAppliedMeta, CompactMode, CompactTrigger, EventTranslator, Phase,
    PromptMetricsPayload, StorageEvent, StorageEventPayload, StoredEvent, generate_session_id,
    generate_turn_id, normalize_recovered_phase, phase_of_storage_event, replay_records,
};
pub use execution_control::ExecutionControl;
pub use execution_result::ExecutionResultCommon;
pub use hook::{
    CompactionHookContext, CompactionHookResultContext, HookEvent, HookHandler, HookInput,
    HookOutcome, ToolHookContext, ToolHookResultContext,
};
pub use ids::{AgentId, CapabilityName, SessionId, SubRunId, TurnId};
pub use local_server::{LOCAL_SERVER_READY_PREFIX, LocalServerInfo};
pub use mode::{
    ActionPolicies, ActionPolicyEffect, ActionPolicyRule, BUILTIN_MODE_CODE_ID,
    BUILTIN_MODE_PLAN_ID, BUILTIN_MODE_REVIEW_ID, CapabilitySelector, ChildPolicySpec,
    GovernanceModeSpec, ModeExecutionPolicySpec, ModeId, PromptProgramEntry, ResolvedChildPolicy,
    ResolvedTurnEnvelope, SubmitBusyPolicy, TransitionPolicySpec,
};
pub use observability::{
    AgentCollaborationScorecardSnapshot, ExecutionDiagnosticsSnapshot, OperationMetricsSnapshot,
    ReplayMetricsSnapshot, ReplayPath, RuntimeMetricsRecorder, RuntimeObservabilitySnapshot,
    SubRunExecutionMetricsSnapshot,
};
pub use plugin::{PluginHealth, PluginManifest, PluginRegistry, PluginState, PluginType};
pub use policy::{
    AllowAllPolicyEngine, ApprovalDefault, ApprovalPending, ApprovalRequest, ApprovalResolution,
    CapabilityCall, ModelRequest, PolicyContext, PolicyEngine, PolicyVerdict, SystemPromptBlock,
    SystemPromptLayer,
};
pub use ports::{
    EventStore, LlmEvent, LlmEventSink, LlmFinishReason, LlmOutput, LlmProvider, LlmRequest,
    LlmUsage, ModelLimits, PromptAgentProfileSummary, PromptBuildCacheMetrics, PromptBuildOutput,
    PromptBuildRequest, PromptDeclaration, PromptDeclarationKind, PromptDeclarationRenderTarget,
    PromptDeclarationSource, PromptEntrySummary, PromptFacts, PromptFactsProvider,
    PromptFactsRequest, PromptGovernanceContext, PromptProvider, PromptSkillSummary,
    ResourceProvider, ResourceReadResult, ResourceRequestContext,
};
pub use projection::{AgentState, AgentStateProjector, project};
pub use registry::{CapabilityContext, CapabilityExecutionResult, CapabilityInvoker};
pub use runtime::{
    ExecutionAccepted, ExecutionOrchestrationBoundary, LiveSubRunControlBoundary,
    LoopRunnerBoundary, ManagedRuntimeComponent, RuntimeCoordinator, RuntimeHandle,
    SessionTruthBoundary,
};
pub use session::{DeleteProjectResult, SessionEventRecord, SessionMeta};
pub use session_catalog::SessionCatalogEvent;
pub use shell::{
    ResolvedShell, ShellFamily, default_shell_label, detect_shell_family, resolve_shell,
};
pub use store::{
    EventLogWriter, SessionManager, SessionTurnAcquireResult, SessionTurnBusy, SessionTurnLease,
    StoreError, StoreResult,
};
pub use time::{
    format_local_rfc3339, format_local_rfc3339_opt, local_rfc3339, local_rfc3339_option,
};
pub use tool::{
    DEFAULT_MAX_OUTPUT_SIZE, ExecutionOwner, Tool, ToolCapabilityMetadata, ToolContext,
    ToolEventSink, ToolPromptMetadata,
};
pub use tool_result_persist::{
    DEFAULT_TOOL_RESULT_INLINE_LIMIT, PersistedToolOutput, PersistedToolResult,
    TOOL_RESULT_PREVIEW_LIMIT, TOOL_RESULTS_DIR, is_persisted_output, maybe_persist_tool_result,
    persist_tool_result, persisted_output_absolute_path,
};
