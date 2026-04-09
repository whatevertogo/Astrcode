//! Sub-run 状态查询：从 live handle 或 durable 事件中解析状态快照。

use std::sync::Arc;

use astrcode_core::{AgentStatus, ChildSessionNotificationKind, SubRunOutcome, SubRunResult};
use astrcode_runtime_execution::{
    ParsedSubRunStatus, ParsedSubRunStatusSource, find_subrun_status_in_events,
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
        Ok(to_service_subrun_snapshot(snapshot))
    }
}

fn to_service_subrun_snapshot(snapshot: ParsedSubRunStatus) -> SubRunStatusSnapshot {
    SubRunStatusSnapshot {
        handle: snapshot.handle,
        descriptor: snapshot.descriptor,
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
        ParsedSubRunStatusSource::LegacyDurable => SubRunStatusSource::LegacyDurable,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ChildTerminalDeliveryProjection {
    pub kind: ChildSessionNotificationKind,
    pub status: AgentStatus,
    pub summary: String,
    pub final_reply_excerpt: Option<String>,
}

pub(super) fn project_child_terminal_delivery(
    result: &SubRunResult,
) -> ChildTerminalDeliveryProjection {
    let (kind, status) = match result.status {
        SubRunOutcome::Running => (
            ChildSessionNotificationKind::ProgressSummary,
            AgentStatus::Running,
        ),
        SubRunOutcome::Completed | SubRunOutcome::TokenExceeded => (
            ChildSessionNotificationKind::Delivered,
            AgentStatus::Completed,
        ),
        SubRunOutcome::Failed => (ChildSessionNotificationKind::Failed, AgentStatus::Failed),
        SubRunOutcome::Aborted => (ChildSessionNotificationKind::Closed, AgentStatus::Cancelled),
    };

    let summary = terminal_summary_or_fallback(result, status);
    let final_reply_excerpt = if matches!(
        result.status,
        SubRunOutcome::Completed | SubRunOutcome::TokenExceeded
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

fn terminal_summary_or_fallback(result: &SubRunResult, status: AgentStatus) -> String {
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

    match status {
        AgentStatus::Completed => "子 Agent 已完成，但没有返回可读总结。".to_string(),
        AgentStatus::Failed => "子 Agent 失败，且没有返回可读错误信息。".to_string(),
        AgentStatus::Cancelled => "子 Agent 已关闭。".to_string(),
        AgentStatus::Running => "子 Agent 正在运行。".to_string(),
        AgentStatus::Pending => "子 Agent 已创建，等待运行。".to_string(),
    }
}
