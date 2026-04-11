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
    build_resumed_child_agent_state, build_root_spawn_params, build_subrun_failure,
    build_subrun_handoff, derive_child_execution_owner, ensure_root_execution_mode,
    ensure_subagent_mode, prepare_prompt_submission, prepare_prompt_submission_with_origin,
    prepare_root_execution_launch, prepare_scoped_agent_execution, resolve_interrupt_session_plan,
    resolve_profile_tool_names, summarize_execution_description,
    validate_root_execution_storage_mode,
};
pub use subrun::{
    CancelSubRunResolution, ParsedSubRunStatus, ParsedSubRunStatusSource, build_child_session_node,
    build_child_session_notification, build_execution_lineage_index, build_subrun_finished_event,
    build_subrun_started_event, find_subrun_status_in_events, live_handle_owned_by_session,
    overlay_live_snapshot_on_durable, resolve_cancel_subrun_resolution,
    resolve_subrun_status_snapshot, snapshot_from_active_handle,
};

/// 子会话生命周期观测阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildLifecycleStage {
    Spawned,
    StartedPersisted,
    TerminalPersisted,
    ReactivationRequested,
    ReactivationSucceeded,
    ReactivationFailed,
}

impl ChildLifecycleStage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Spawned => "spawned",
            Self::StartedPersisted => "started_persisted",
            Self::TerminalPersisted => "terminal_persisted",
            Self::ReactivationRequested => "reactivation_requested",
            Self::ReactivationSucceeded => "reactivation_succeeded",
            Self::ReactivationFailed => "reactivation_failed",
        }
    }
}

/// 谱系不一致的稳定分类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineageMismatchKind {
    ParentAgent,
    ParentSession,
    ChildSession,
    DescriptorMissing,
}

impl LineageMismatchKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ParentAgent => "parent_agent",
            Self::ParentSession => "parent_session",
            Self::ChildSession => "child_session",
            Self::DescriptorMissing => "descriptor_missing",
        }
    }
}

/// 交付缓冲/唤醒链路的稳定动作标签。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryBufferStage {
    Queued,
    Dequeued,
    WakeRequested,
    WakeSucceeded,
    WakeFailed,
}

impl DeliveryBufferStage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Dequeued => "dequeued",
            Self::WakeRequested => "wake_requested",
            Self::WakeSucceeded => "wake_succeeded",
            Self::WakeFailed => "wake_failed",
        }
    }
}

/// legacy cutover 的稳定拒绝原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyRejectionKind {
    SharedHistoryUnsupported,
}

impl LegacyRejectionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SharedHistoryUnsupported => "unsupported_legacy_shared_history",
        }
    }
}

/// 统一 legacy shared-history 拒绝文案，避免 runtime/server 各自拼接出不一致错误。
pub fn legacy_shared_history_rejection_message(
    session_id: &str,
    sub_run_id: Option<&str>,
) -> String {
    match sub_run_id.filter(|value| !value.is_empty()) {
        Some(sub_run_id) => format!(
            "{}: session '{}' contains legacy shared-history data for sub-run '{}'; open the \
             migrated child session durable history before continuing",
            LegacyRejectionKind::SharedHistoryUnsupported.as_str(),
            session_id,
            sub_run_id
        ),
        None => format!(
            "{}: session '{}' contains legacy shared-history data; open the migrated child \
             session durable history before continuing",
            LegacyRejectionKind::SharedHistoryUnsupported.as_str(),
            session_id
        ),
    }
}

/// Child terminal delivery 的统一结果标签。
///
/// 用于 observability 日志，避免不同调用方各自拼接不一致字符串。
pub fn child_delivery_outcome_label(result: &astrcode_core::SubRunResult) -> &'static str {
    match result.status {
        astrcode_core::AgentStatus::Running => "running",
        astrcode_core::AgentStatus::Completed => "completed",
        astrcode_core::AgentStatus::Failed => "failed",
        astrcode_core::AgentStatus::Cancelled => "cancelled",
        astrcode_core::AgentStatus::TokenExceeded => "token_exceeded",
        _ => "unknown",
    }
}

/// 协作工具（send/close）结果的统一标签。
///
/// 用于 observability 日志，记录协作操作类型与接受状态。
pub fn collaboration_action_label(result: &astrcode_core::CollaborationResult) -> &'static str {
    match result.kind {
        astrcode_core::CollaborationResultKind::Sent => "sent",
        astrcode_core::CollaborationResultKind::Observed => "observed",
        astrcode_core::CollaborationResultKind::Closed => "closed",
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
