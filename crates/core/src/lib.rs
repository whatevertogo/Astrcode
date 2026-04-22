//! # Astrcode 核心库
//!
//! 本库定义了 Astrcode 系统的核心领域模型和接口，与具体的运行时实现解耦。
//!
//! ## 主要模块
//!
//! ### 领域模型
//!
//! - [`agent`][]: Agent 协作模型、子运行管理、输入队列
//! - [`capability`][]: 能力规格定义（CapabilitySpec 等）
//! - [`ids`][]: 核心标识符类型（AgentId, SessionId, TurnId 等）
//! - [`action`][]: LLM 消息与工具调用相关的数据结构
//!
//! ### 事件与会话
//!
//! - [`event`][]: 事件存储与回放系统（JSONL append-only 日志）
//! - [`session`][]: 会话元数据
//! - [`store`][]: 会话存储与事件日志写入
//! - [`projection`][]: Agent 状态投影（从事件流推导状态）
//!
//! ### 治理与策略
//!
//! - [`mode`][]: 治理模式（Code/Plan/Review 模式与策略规则）
//! - [`policy`][]: 策略引擎 trait（审批与模型/工具请求检查）
//!
//! ### 扩展点
//!
//! - [`ports`][]: 核心 port trait 定义（LlmProvider, PromptProvider, EventStore 等）
//! - [`tool`][]: Tool trait 定义（插件系统的基础抽象）
//! - [`plugin`][]: 插件清单与注册表
//! - [`registry`][]: 能力路由器（将能力调用分派到具体的 invoker）
//! - [`hook`][]: 钩子系统（工具/压缩钩子）
//!
//! ### 运行时与配置
//!
//! - [`runtime`][]: 运行时协调器接口
//! - [`config`][]: 配置模型（Agent/Model/Runtime 配置）
//! - [`observability`][]: 运行时可观测性指标
//!
//! ### 基础设施
//!
//! - [`env`][]: 环境变量解析
//! - [`local_server`][]: 本地服务器信息
//! - [`project`][]: 项目标识与目录名算法
//! - [`shell`][]: Shell 检测与解析
//! - [`tool_result_persist`][]: 工具结果持久化

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
mod execution_task;
pub mod hook;
pub mod ids;
pub mod local_server;
mod mcp;
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
mod session_plan;
mod shell;
mod skill;
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
mod workflow;

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
    PromptMetricsPayload, StorageEvent, StorageEventPayload, StoredEvent, TurnTerminalKind,
    generate_session_id, generate_turn_id, normalize_recovered_phase, phase_of_storage_event,
    replay_records,
};
pub use execution_control::ExecutionControl;
pub use execution_result::{ExecutionContinuation, ExecutionResultCommon};
pub use execution_task::{
    EXECUTION_TASK_SNAPSHOT_SCHEMA, ExecutionTaskItem, ExecutionTaskSnapshotMetadata,
    ExecutionTaskStatus, TaskSnapshot,
};
pub use hook::{
    CompactionHookContext, CompactionHookResultContext, HookEvent, HookHandler, HookInput,
    HookOutcome, ToolHookContext, ToolHookResultContext,
};
pub use ids::{AgentId, CapabilityName, SessionId, SubRunId, TurnId};
pub use local_server::{LOCAL_SERVER_READY_PREFIX, LocalServerInfo};
pub use mcp::{McpApprovalData, McpApprovalStatus};
pub use mode::{
    ActionPolicies, ActionPolicyEffect, ActionPolicyRule, BUILTIN_MODE_CODE_ID,
    BUILTIN_MODE_PLAN_ID, BUILTIN_MODE_REVIEW_ID, BoundModeToolContractSnapshot,
    CapabilitySelector, ChildPolicySpec, CompiledModeContracts, GovernanceModeSpec,
    ModeArtifactDef, ModeExecutionPolicySpec, ModeExitGateDef, ModeId, ModePromptHooks,
    PromptProgramEntry, ResolvedChildPolicy, ResolvedTurnEnvelope, SubmitBusyPolicy,
    TransitionPolicySpec,
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
    LlmUsage, McpSettingsStore, ModelLimits, ProjectionRegistrySnapshot, PromptAgentProfileSummary,
    PromptBuildCacheMetrics, PromptBuildOutput, PromptBuildRequest, PromptCacheBreakReason,
    PromptCacheDiagnostics, PromptCacheGlobalStrategy, PromptCacheHints, PromptDeclaration,
    PromptDeclarationKind, PromptDeclarationRenderTarget, PromptDeclarationSource,
    PromptEntrySummary, PromptFacts, PromptFactsProvider, PromptFactsRequest,
    PromptGovernanceContext, PromptLayerFingerprints, PromptProvider, PromptSkillSummary,
    RecoveredSessionState, ResourceProvider, ResourceReadResult, ResourceRequestContext,
    SessionRecoveryCheckpoint, SkillCatalog, TurnProjectionSnapshot,
};
pub use projection::{AgentState, AgentStateProjector, project};
pub use registry::{CapabilityContext, CapabilityExecutionResult, CapabilityInvoker};
pub use runtime::{
    ExecutionAccepted, ExecutionOrchestrationBoundary, LiveSubRunControlBoundary,
    LoopRunnerBoundary, ManagedRuntimeComponent, RuntimeHandle, SessionTruthBoundary,
};
pub use session::{DeleteProjectResult, SessionEventRecord, SessionMeta};
pub use session_catalog::SessionCatalogEvent;
pub use session_plan::{
    SESSION_PLAN_DRAFT_APPROVAL_GUARD_MARKER, SessionPlanState, SessionPlanStatus,
    session_plan_content_digest,
};
pub use shell::{ResolvedShell, ShellFamily};
pub use skill::{SkillSource, SkillSpec, is_valid_skill_name, normalize_skill_name};
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
    TOOL_RESULT_PREVIEW_LIMIT, TOOL_RESULTS_DIR, is_persisted_output,
    persisted_output_absolute_path,
};
pub use workflow::{
    WorkflowArtifactRef, WorkflowBridgeState, WorkflowDef, WorkflowInstanceState, WorkflowPhaseDef,
    WorkflowSignal, WorkflowTransitionDef, WorkflowTransitionTrigger,
};
