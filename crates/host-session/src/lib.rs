//! session owner 骨架。
//!
//! 这个 crate 后续承接 durable truth、恢复、query/read model、branch/fork、
//! compaction 与多 agent 协作真相。

pub mod branch;
pub mod catalog;
mod child_sessions;
pub mod collaboration;
pub mod compaction;
pub mod composer;
mod event_cache;
pub mod event_log;
mod event_translate;
pub mod execution_surface;
pub mod fork;
pub mod input_hooks;
pub mod input_queue;
pub mod model_selection;
pub mod ports;
pub mod projection;
mod projection_registry;
pub mod query;
pub mod session_catalog;
pub mod session_plan;
pub mod state;
mod tasks;
pub mod turn_mutation;
mod turn_projection;
pub mod workflow;

pub use branch::SubmitTarget;
pub use catalog::{LoadedSession, SessionCatalog, SessionModeState};
pub use collaboration::{
    CollaborationExecutor, DeliveryState, ResultDeliveryState, SubAgentExecutor, SubRunFinishStats,
    SubRunHandle, SubRunStatus, agent_event_context_from_subrun, subrun_finished_event,
    subrun_started_event,
};
pub use compaction::CompactPersistResult;
pub use composer::{ComposerOption, ComposerOptionActionKind, ComposerOptionKind};
pub use event_log::SessionWriter;
pub use event_translate::{EventTranslator, replay_records};
pub use execution_surface::HostSessionSnapshot;
pub use fork::{ForkPoint, ForkResult};
pub use input_hooks::{InputHookApplyRequest, InputHookDecision, apply_input_hooks};
pub use input_queue::{InputKind, InputQueueProjection};
pub use model_selection::{ModelSelectionDecision, apply_model_select_hooks};
pub use ports::{
    EventStore, HookDispatch, ProjectionRegistrySnapshot, PromptAgentProfileSummary,
    PromptBuildCacheMetrics, PromptBuildOutput, PromptBuildRequest, PromptEntrySummary,
    PromptFacts, PromptFactsProvider, PromptFactsRequest, PromptProvider, PromptSkillSummary,
    RecoveredSessionState, SessionRecoveryCheckpoint, TurnProjectionSnapshot,
};
pub use projection::{AgentState, AgentStateProjector, project};
pub use query::{
    LastCompactMetaSnapshot, ProjectedTurnOutcome, SessionControlStateSnapshot,
    SessionObserveSnapshot, SessionReadModelReplay, TurnTerminalSnapshot,
};
pub use session_catalog::SessionCatalogEvent;
pub use session_plan::{
    SESSION_PLAN_DRAFT_APPROVAL_GUARD_MARKER, SessionPlanState, SessionPlanStatus,
    session_plan_content_digest,
};
pub use state::{
    SESSION_BROADCAST_CAPACITY, SESSION_LIVE_BROADCAST_CAPACITY, SessionSnapshot, SessionState,
    append_and_broadcast, checkpoint_if_compacted,
};
pub use turn_mutation::{
    AcceptedSubmitPrompt, BegunAcceptedTurn, CompactSessionMutationInput, CompactSessionSummary,
    InterruptSessionMutationInput, InterruptSessionSummary, PendingManualCompactRequest,
    PromptAcceptedSummary, RuntimeTurnEventPersistenceInput, RuntimeTurnPersistenceInput,
    SubmitPromptMutationInput, SubmitTurnBusyPolicy, TurnMutationFacade, TurnMutationPreparation,
    TurnMutationPreparationOwner,
};
pub use workflow::{
    WorkflowArtifactRef, WorkflowBridgeState, WorkflowDef, WorkflowInstanceState, WorkflowPhaseDef,
    WorkflowSignal, WorkflowTransitionDef, WorkflowTransitionTrigger,
};
