//! Astrcode 共享语义层。
//!
//! 承载跨 crate 共享的类型、trait、事件数据模型和工具/LLM 契约。
//! runtime 边界合同由 `runtime-contract` crate 持有。

pub mod action;
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
pub mod llm;
pub mod local_server;
mod mcp;
pub mod mode;
pub mod observability;
pub mod policy;
pub mod ports;
pub mod project;
pub mod prompt;
pub mod registry;
pub mod session;
mod shell;
pub mod skill;
pub mod store;
pub mod support;
#[cfg(feature = "test-support")]
pub mod test_support;
mod time;
pub mod tool;
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
    SendToChildParams, SendToParentParams, SpawnAgentParams, SubRunFailure, SubRunFailureCode,
    SubRunHandoff, SubRunResult, SubRunStatus, SubRunStorageMode, SubagentContextOverrides,
    input_queue::{
        BatchId, CloseParams, DeliveryId, InputBatchAckedPayload, InputBatchStartedPayload,
        InputDiscardedPayload, InputQueuedPayload, ObserveParams, ObserveSnapshot,
        QueuedInputEnvelope, SendParams,
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
pub use config::{
    ActiveSelection, AgentConfig, Config, ConfigOverlay, CurrentModelSelection, ModelConfig,
    ModelOption, ModelSelection, OpenAiApiMode, Profile, ResolvedAgentConfig,
    ResolvedRuntimeConfig, RuntimeConfig, TestConnectionResult, resolve_agent_config,
    resolve_runtime_config,
};
pub use error::{AstrError, Result, ResultExt};
pub use event::{
    AgentEvent, CompactAppliedMeta, CompactMode, CompactTrigger, Phase, PromptMetricsPayload,
    StorageEvent, StorageEventPayload, StoredEvent, TurnTerminalKind, generate_session_id,
    generate_turn_id, normalize_recovered_phase, phase_of_storage_event,
};
pub use execution_control::ExecutionControl;
pub use execution_result::{ExecutionContinuation, ExecutionResultCommon};
pub use execution_task::{
    EXECUTION_TASK_SNAPSHOT_SCHEMA, ExecutionTaskItem, ExecutionTaskSnapshotMetadata,
    ExecutionTaskStatus, TaskSnapshot,
};
pub use hook::HookEventKey;
pub use ids::{AgentId, CapabilityName, SessionId, SubRunId, TurnId};
pub use local_server::{LOCAL_SERVER_READY_PREFIX, LocalServerInfo};
pub use mcp::{McpApprovalData, McpApprovalStatus};
pub use observability::{
    AgentCollaborationScorecardSnapshot, ExecutionDiagnosticsSnapshot, OperationMetricsSnapshot,
    ReplayMetricsSnapshot, ReplayPath, RuntimeMetricsRecorder, RuntimeObservabilitySnapshot,
    SubRunExecutionMetricsSnapshot,
};
pub use policy::SystemPromptLayer;
pub use ports::{McpSettingsStore, SkillCatalog};
pub use prompt::{
    PromptCacheBreakReason, PromptCacheDiagnostics, PromptCacheGlobalStrategy, PromptCacheHints,
    PromptDeclaration, PromptDeclarationKind, PromptDeclarationRenderTarget,
    PromptDeclarationSource, PromptLayerFingerprints, SystemPromptBlock,
};
pub use registry::{CapabilityContext, CapabilityExecutionResult, CapabilityInvoker};
pub use session::{DeleteProjectResult, SessionEventRecord, SessionMeta};
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
