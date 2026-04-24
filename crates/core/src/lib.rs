//! Astrcode 最小共享语义层。
//!
//! `core` 只作为跨 owner 共享的值对象和稳定语义入口。session durable
//! truth、plugin descriptor、运行时执行面、projection、workflow、mode 等
//! owner 专属模型不再作为顶层默认导出；仍保留的历史模块路径仅供 owner
//! bridge 内部复用，不应被新调用方继续视为正式入口。

mod action;
pub mod agent;
mod cancel;
pub mod capability;
mod compact_summary;
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
pub mod policy;
pub mod ports;
pub mod project;
mod prompt;
pub mod registry;
pub mod runtime;
pub mod session;
mod shell;
mod skill;
pub mod store;
pub mod support;
#[cfg(feature = "test-support")]
pub mod test_support;
mod time;
mod tool;
pub mod tool_result_persist;

pub use action::{
    AssistantContentParts, LlmMessage, ReasoningContent, ToolCallRequest, ToolDefinition,
    ToolExecutionResult, ToolOutputDelta, ToolOutputStream, UserMessageOrigin,
    split_assistant_content,
};
#[doc(hidden)]
pub use agent::{
    AgentCollaborationActionKind, AgentCollaborationFact as PreviousAgentCollaborationFact,
    AgentCollaborationOutcomeKind, AgentCollaborationPolicyContext,
    AgentEventContext as PreviousAgentEventContext, AgentInboxEnvelope, AgentMode,
    AgentProfile as PreviousAgentProfile, AgentProfileCatalog, ArtifactRef, ChildAgentRef,
    ChildExecutionIdentity, ChildSessionLineageKind, ChildSessionNode as PreviousChildSessionNode,
    ChildSessionNotification as PreviousChildSessionNotification,
    ChildSessionNotificationKind as PreviousChildSessionNotificationKind, ChildSessionStatusSource,
    CloseAgentParams as PreviousCloseAgentParams, CloseRequestParentDeliveryPayload,
    CollaborationResult as PreviousCollaborationResult, CompletedParentDeliveryPayload,
    CompletedSubRunOutcome, DelegationMetadata, FailedParentDeliveryPayload, FailedSubRunOutcome,
    ForkMode as PreviousForkMode, InboxEnvelopeKind, InvocationKind as PreviousInvocationKind,
    LineageSnapshot, ParentDelivery, ParentDeliveryKind, ParentDeliveryOrigin,
    ParentDeliveryPayload, ParentDeliveryTerminalSemantics, ParentExecutionRef,
    ProgressParentDeliveryPayload,
    ResolvedExecutionLimitsSnapshot as PreviousResolvedExecutionLimitsSnapshot,
    ResolvedSubagentContextOverrides as PreviousResolvedSubagentContextOverrides,
    SendAgentParams as PreviousSendAgentParams, SendToChildParams, SendToParentParams,
    SpawnAgentParams as PreviousSpawnAgentParams, SubRunFailure, SubRunFailureCode, SubRunHandoff,
    SubRunResult as PreviousSubRunResult, SubRunStatus,
    SubRunStorageMode as PreviousSubRunStorageMode,
    SubagentContextOverrides as PreviousSubagentContextOverrides,
    input_queue::{
        BatchId, CloseParams, DeliveryId as PreviousDeliveryId,
        InputBatchAckedPayload as PreviousInputBatchAckedPayload,
        InputBatchStartedPayload as PreviousInputBatchStartedPayload,
        InputDiscardedPayload as PreviousInputDiscardedPayload,
        InputQueuedPayload as PreviousInputQueuedPayload, ObserveParams as PreviousObserveParams,
        ObserveSnapshot, QueuedInputEnvelope, SendParams,
    },
    lifecycle::{AgentLifecycleStatus, AgentTurnOutcome as PreviousAgentTurnOutcome},
};
#[doc(hidden)]
pub use agent::{
    AgentCollaborationFact, AgentEventContext, AgentProfile, AgentTurnOutcome, ChildSessionNode,
    ChildSessionNotification, ChildSessionNotificationKind, CloseAgentParams, CollaborationResult,
    ForkMode, InvocationKind, ResolvedExecutionLimitsSnapshot, ResolvedSubagentContextOverrides,
    SendAgentParams, SpawnAgentParams, SubRunResult, SubRunStorageMode, SubagentContextOverrides,
    input_queue::{
        DeliveryId, InputBatchAckedPayload, InputBatchStartedPayload, InputDiscardedPayload,
        InputQueuedPayload, ObserveParams,
    },
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
#[doc(hidden)]
pub use config::{
    ActiveSelection, AgentConfig, Config, ConfigOverlay, CurrentModelSelection, ModelConfig,
    ModelOption, ModelSelection, OpenAiApiMode, Profile, ResolvedAgentConfig,
    ResolvedRuntimeConfig, RuntimeConfig, TestConnectionResult, max_tool_concurrency,
    resolve_agent_config, resolve_runtime_config,
};
pub use error::{AstrError, Result, ResultExt};
pub use event::{
    AgentEvent, CompactAppliedMeta, CompactMode, CompactTrigger, EventTranslator, Phase,
    PromptMetricsPayload, StorageEvent, StorageEventPayload, StoredEvent, TurnTerminalKind,
    generate_session_id, generate_turn_id, normalize_recovered_phase, phase_of_storage_event,
    replay_records,
};
#[doc(hidden)]
pub use execution_control::ExecutionControl;
#[doc(hidden)]
pub use execution_result::{ExecutionContinuation, ExecutionResultCommon};
#[doc(hidden)]
pub use execution_task::{
    EXECUTION_TASK_SNAPSHOT_SCHEMA, ExecutionTaskItem, ExecutionTaskSnapshotMetadata,
    ExecutionTaskStatus, TaskSnapshot,
};
#[doc(hidden)]
pub use hook::{
    CompactionHookContext, CompactionHookResultContext, ToolHookContext, ToolHookResultContext,
};
pub use hook::{HookEvent, HookEventKey};
pub use ids::{AgentId, CapabilityName, SessionId, SubRunId, TurnId};
pub use local_server::{LOCAL_SERVER_READY_PREFIX, LocalServerInfo};
pub use mcp::{McpApprovalData, McpApprovalStatus};
#[doc(hidden)]
pub use mode::{
    ActionPolicies, ActionPolicyEffect, ActionPolicyRule, BUILTIN_MODE_CODE_ID,
    BUILTIN_MODE_PLAN_ID, BUILTIN_MODE_REVIEW_ID, BoundModeToolContractSnapshot,
    CapabilitySelector, ChildPolicySpec, CompiledModeContracts, GovernanceModeSpec,
    ModeArtifactDef, ModeExecutionPolicySpec, ModeExitGateDef, ModeId, ModePromptHooks,
    PromptProgramEntry, ResolvedChildPolicy, ResolvedTurnEnvelope, SubmitBusyPolicy,
    TransitionPolicySpec,
};
#[doc(hidden)]
pub use observability::{
    AgentCollaborationScorecardSnapshot, ExecutionDiagnosticsSnapshot, OperationMetricsSnapshot,
    ReplayMetricsSnapshot, ReplayPath, RuntimeMetricsRecorder, RuntimeObservabilitySnapshot,
    SubRunExecutionMetricsSnapshot,
};
#[doc(hidden)]
pub use policy::{
    AllowAllPolicyEngine, ApprovalDefault, ApprovalPending, ApprovalRequest, ApprovalResolution,
    CapabilityCall, ModelRequest, PolicyContext, PolicyEngine, PolicyVerdict, SystemPromptBlock,
    SystemPromptLayer,
};
#[doc(hidden)]
pub use ports::{McpSettingsStore, SkillCatalog};
pub use prompt::{
    PromptCacheBreakReason, PromptCacheDiagnostics, PromptCacheGlobalStrategy, PromptCacheHints,
    PromptDeclaration, PromptDeclarationKind, PromptDeclarationRenderTarget,
    PromptDeclarationSource, PromptLayerFingerprints,
};
pub use registry::{CapabilityContext, CapabilityExecutionResult, CapabilityInvoker};
#[doc(hidden)]
pub use runtime::{
    ExecutionAccepted, ExecutionOrchestrationBoundary, LiveSubRunControlBoundary,
    LoopRunnerBoundary, ManagedRuntimeComponent, RuntimeHandle, SessionTruthBoundary,
};
pub use session::{DeleteProjectResult, SessionEventRecord, SessionMeta};
pub use shell::{ResolvedShell, ShellFamily};
pub use skill::{SkillSource, SkillSpec, is_valid_skill_name, normalize_skill_name};
#[doc(hidden)]
pub use store::{
    EventLogWriter, SessionManager, SessionTurnAcquireResult, SessionTurnBusy, SessionTurnLease,
    StoreError, StoreResult,
};
#[doc(hidden)]
pub use time::{
    format_local_rfc3339, format_local_rfc3339_opt, local_rfc3339, local_rfc3339_option,
};
pub use tool::{
    DEFAULT_MAX_OUTPUT_SIZE, ExecutionOwner, Tool, ToolCapabilityMetadata, ToolContext,
    ToolEventSink, ToolPromptMetadata,
};
#[doc(hidden)]
pub use tool_result_persist::{
    DEFAULT_TOOL_RESULT_INLINE_LIMIT, PersistedToolOutput, PersistedToolResult,
    TOOL_RESULT_PREVIEW_LIMIT, TOOL_RESULTS_DIR, is_persisted_output,
    persisted_output_absolute_path,
};
