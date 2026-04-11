//! Sub-run 状态查询：从 live handle 或 durable 事件中解析状态快照。

use std::sync::Arc;

use astrcode_core::{
    AgentLifecycleStatus, AgentTurnOutcome, ChildSessionNotificationKind, SubRunResult,
    SubRunStorageMode,
};
use astrcode_runtime_execution::{
    LegacyRejectionKind, ParsedSubRunStatus, ParsedSubRunStatusSource,
    find_subrun_status_in_events, legacy_shared_history_rejection_message,
    resolve_subrun_status_snapshot,
};
use astrcode_runtime_session::normalize_session_id;

use super::root::AgentExecutionServiceHandle;
use crate::service::{ServiceError, ServiceResult, SubRunStatusSnapshot, SubRunStatusSource};

impl AgentExecutionServiceHandle {
    /// 查询指定 sub-run 的状态快照。
    pub async fn get_subrun_status(
        &self,
        session_id: &str,
        sub_run_id: &str,
    ) -> ServiceResult<SubRunStatusSnapshot> {
        let session_id = normalize_session_id(session_id);
        let events = crate::service::session::load_events(
            Arc::clone(&self.runtime.session_manager),
            &session_id,
        )
        .await?;
        let durable_snapshot = find_subrun_status_in_events(&events, &session_id, sub_run_id);
        let live_handle = self.runtime.agent_control.get(sub_run_id).await;
        let Some(snapshot) = resolve_subrun_status_snapshot(
            &session_id,
            live_handle,
            durable_snapshot,
            normalize_session_id,
        ) else {
            return Err(ServiceError::NotFound(format!(
                "sub-run '{}' was not found in session '{}'",
                sub_run_id, session_id
            )));
        };
        if is_legacy_shared_history_snapshot(&snapshot) {
            self.runtime
                .observability
                .record_legacy_rejection(LegacyRejectionKind::SharedHistoryUnsupported);
            return Err(ServiceError::Conflict(
                legacy_shared_history_rejection_message(&session_id, Some(sub_run_id)),
            ));
        }
        Ok(to_service_subrun_snapshot(snapshot))
    }
}

fn is_legacy_shared_history_snapshot(snapshot: &ParsedSubRunStatus) -> bool {
    if snapshot.source != ParsedSubRunStatusSource::Durable {
        return false;
    }
    if snapshot.handle.storage_mode != SubRunStorageMode::SharedSession {
        return false;
    }
    if snapshot.handle.child_session_id.is_some() {
        return false;
    }
    let Some(result) = snapshot.result.as_ref() else {
        return false;
    };
    // 旧 shared-history 场景：lifecycle=Idle + outcome=Completed/TokenExceeded + 无 handoff/failure
    let is_legacy_completed = matches!(
        result.last_turn_outcome,
        Some(AgentTurnOutcome::Completed | AgentTurnOutcome::TokenExceeded)
    );
    is_legacy_completed && result.handoff.is_none() && result.failure.is_none()
}

fn to_service_subrun_snapshot(snapshot: ParsedSubRunStatus) -> SubRunStatusSnapshot {
    SubRunStatusSnapshot {
        handle: snapshot.handle,
        tool_call_id: snapshot.tool_call_id,
        source: map_subrun_status_source(snapshot.source),
        result: snapshot.result,
        step_count: snapshot.step_count,
        estimated_tokens: snapshot.estimated_tokens,
        resolved_overrides: snapshot.resolved_overrides,
        resolved_limits: snapshot.resolved_limits,
    }
}

fn map_subrun_status_source(source: ParsedSubRunStatusSource) -> SubRunStatusSource {
    match source {
        ParsedSubRunStatusSource::Live => SubRunStatusSource::Live,
        ParsedSubRunStatusSource::Durable => SubRunStatusSource::Durable,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ChildTerminalDeliveryProjection {
    pub kind: ChildSessionNotificationKind,
    pub status: AgentLifecycleStatus,
    pub summary: String,
    pub final_reply_excerpt: Option<String>,
}

pub(super) fn project_child_terminal_delivery(
    result: &SubRunResult,
) -> ChildTerminalDeliveryProjection {
    let (kind, status) = match result.last_turn_outcome {
        None => match result.lifecycle {
            AgentLifecycleStatus::Pending => (
                ChildSessionNotificationKind::ProgressSummary,
                AgentLifecycleStatus::Pending,
            ),
            AgentLifecycleStatus::Running => (
                ChildSessionNotificationKind::ProgressSummary,
                AgentLifecycleStatus::Running,
            ),
            _ => (
                ChildSessionNotificationKind::ProgressSummary,
                result.lifecycle,
            ),
        },
        Some(AgentTurnOutcome::Completed) => (
            ChildSessionNotificationKind::Delivered,
            AgentLifecycleStatus::Idle,
        ),
        Some(AgentTurnOutcome::TokenExceeded) => (
            ChildSessionNotificationKind::Delivered,
            AgentLifecycleStatus::Idle,
        ),
        Some(AgentTurnOutcome::Failed) => (
            ChildSessionNotificationKind::Failed,
            AgentLifecycleStatus::Idle,
        ),
        Some(AgentTurnOutcome::Cancelled) => (
            ChildSessionNotificationKind::Closed,
            AgentLifecycleStatus::Idle,
        ),
    };

    let summary = terminal_summary_or_fallback(result, status);
    let final_reply_excerpt = if matches!(
        result.last_turn_outcome,
        Some(AgentTurnOutcome::Completed | AgentTurnOutcome::TokenExceeded)
    ) {
        result
            .handoff
            .as_ref()
            .map(|handoff| handoff.summary.trim().to_string())
            .filter(|summary| !summary.is_empty())
    } else {
        None
    };

    ChildTerminalDeliveryProjection {
        kind,
        status,
        summary,
        final_reply_excerpt,
    }
}

fn terminal_summary_or_fallback(result: &SubRunResult, _status: AgentLifecycleStatus) -> String {
    if let Some(summary) = result
        .handoff
        .as_ref()
        .map(|handoff| handoff.summary.trim())
        .filter(|summary| !summary.is_empty())
    {
        return summary.to_string();
    }

    if let Some(display_message) = result
        .failure
        .as_ref()
        .map(|failure| failure.display_message.trim())
        .filter(|message| !message.is_empty())
    {
        return display_message.to_string();
    }

    match result.last_turn_outcome {
        Some(AgentTurnOutcome::Completed) => "子 Agent 已完成，但没有返回可读总结。".to_string(),
        Some(AgentTurnOutcome::TokenExceeded) => {
            "子 Agent 因 token 限额结束，但没有返回可读总结。".to_string()
        },
        Some(AgentTurnOutcome::Failed) => "子 Agent 失败，且没有返回可读错误信息。".to_string(),
        Some(AgentTurnOutcome::Cancelled) => "子 Agent 已关闭。".to_string(),
        None => match result.lifecycle {
            AgentLifecycleStatus::Running => "子 Agent 正在运行。".to_string(),
            AgentLifecycleStatus::Pending => "子 Agent 已创建，等待运行。".to_string(),
            _ => "子 Agent 状态未知。".to_string(),
        },
    }
}
