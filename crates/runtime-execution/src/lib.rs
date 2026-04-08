//! Agent execution 装配与上下文构建的共享逻辑。
//!
//! 这里保留纯函数与无状态装配器，让 runtime façade 不再同时承担
//! profile 裁剪、context snapshot 拼装与结果摘要等细节。

mod context;
mod policy;
mod prep;
mod subrun;

pub use context::{
    ExecutionLineageEntry, ExecutionLineageIndex, ExecutionLineageScope,
    LINEAGE_METADATA_UNAVAILABLE_MESSAGE, ResolvedContextSnapshot, latest_compact_summary,
    recent_tail_lines, resolve_context_snapshot, single_line,
};
pub use policy::resolve_subagent_overrides;
pub use prep::{
    AgentExecutionRequest, AgentExecutionSpec, InterruptSessionPlan, PreparedAgentExecution,
    PreparedPromptSubmission, RootExecutionLaunch, ScopedExecutionSurface,
    build_background_subrun_handoff, build_child_agent_state, build_result_artifacts,
    build_root_spawn_params, build_subrun_failure, build_subrun_handoff,
    derive_child_execution_owner, ensure_root_execution_mode, ensure_subagent_mode,
    prepare_prompt_submission, prepare_root_execution_launch, prepare_scoped_agent_execution,
    resolve_interrupt_session_plan, resolve_profile_tool_names, summarize_execution_description,
    validate_root_execution_storage_mode,
};
pub use subrun::{
    CancelSubRunResolution, ParsedSubRunStatus, ParsedSubRunStatusSource, build_child_session_node,
    build_child_session_notification, build_execution_lineage_index, build_subrun_descriptor,
    build_subrun_finished_event, build_subrun_started_event, find_subrun_status_in_events,
    live_handle_owned_by_session, overlay_live_snapshot_on_durable,
    resolve_cancel_subrun_resolution, resolve_subrun_status_snapshot, snapshot_from_active_handle,
};

/// Child terminal delivery 的统一结果标签。
///
/// 用于 observability 日志，避免不同调用方各自拼接不一致字符串。
pub fn child_delivery_outcome_label(result: &astrcode_core::SubRunResult) -> &'static str {
    match result.status {
        astrcode_core::SubRunOutcome::Running => "running",
        astrcode_core::SubRunOutcome::Completed => "completed",
        astrcode_core::SubRunOutcome::Failed => "failed",
        astrcode_core::SubRunOutcome::Aborted => "aborted",
        astrcode_core::SubRunOutcome::TokenExceeded => "token_exceeded",
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn runtime_execution_boundary_metadata_declares_expected_public_surface() {
        let manifest = include_str!("../Cargo.toml");

        assert!(manifest.contains("owner = \"execution-orchestration\""));
        assert!(manifest.contains("public_surface = ["));
        assert!(manifest.contains("\"submit\""));
        assert!(manifest.contains("\"interrupt\""));
        assert!(manifest.contains("\"root-execute\""));
        assert!(manifest.contains("\"subrun-status\""));
        assert!(manifest.contains("\"subrun-cancel\""));
    }

    #[test]
    fn runtime_execution_boundary_manifest_keeps_forbidden_cross_boundary_dependencies() {
        let manifest = include_str!("../Cargo.toml");

        assert!(manifest.contains("forbidden_depends_on = ["));
        assert!(manifest.contains("\"astrcode-runtime-session\""));
        assert!(manifest.contains("\"astrcode-runtime-agent-control\""));
        assert!(manifest.contains("\"astrcode-runtime-agent-loop\""));
    }
}
