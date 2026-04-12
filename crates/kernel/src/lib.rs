pub mod agent_tree;
pub mod error;
pub mod events;
pub mod execution;
pub mod gateway;
pub mod kernel;
pub mod registry;
pub mod surface;

pub use agent_tree::{
    AgentControl, AgentControlError, AgentControlLimits, AgentProfileSource, LiveSubRunControl,
    PendingParentDelivery, StaticAgentProfileSource,
    subrun::{
        CancelSubRunResolution, ParsedSubRunStatus, ParsedSubRunStatusSource,
        build_child_session_node, build_child_session_notification, build_execution_lineage_index,
        build_subrun_finished_event, build_subrun_started_event, find_subrun_status_in_events,
        live_handle_owned_by_session, overlay_live_snapshot_on_durable,
        resolve_cancel_subrun_resolution, resolve_subrun_status_snapshot,
        snapshot_from_active_handle,
    },
};
pub use error::KernelError;
pub use events::{EventHub, KernelEvent};
pub use execution::{
    AgentExecutionRequest, AgentExecutionSpec, CHILD_INHERITED_COMPACT_SUMMARY_BLOCK_ID,
    CHILD_INHERITED_RECENT_TAIL_BLOCK_ID, ExecutionLineageEntry, ExecutionLineageIndex,
    ExecutionLineageScope, InterruptSessionPlan, LINEAGE_METADATA_UNAVAILABLE_MESSAGE,
    PreparedPromptSubmission, ResolvedContextSnapshot, RootExecutionLaunch,
    build_background_subrun_handoff, build_child_agent_state, build_execution_spec,
    build_resumed_child_agent_state, build_root_spawn_params, build_subrun_failure,
    build_subrun_handoff, derive_child_execution_owner, ensure_root_execution_mode,
    ensure_subagent_mode, latest_compact_summary, prepare_prompt_submission,
    prepare_prompt_submission_with_origin, prepare_root_execution_launch, recent_tail_lines,
    resolve_interrupt_session_plan, resolve_profile_tool_names, resolve_subagent_overrides,
    single_line, summarize_execution_description, validate_root_execution_storage_mode,
};
pub use gateway::KernelGateway;
pub use kernel::{Kernel, KernelBuilder};
pub use registry::{CapabilityRouter, CapabilityRouterBuilder, ToolCapabilityInvoker};
pub use surface::{SurfaceManager, SurfaceSnapshot};
