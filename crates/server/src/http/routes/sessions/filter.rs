use astrcode_application::{
    AgentEvent, AgentEventContext, SessionEventFilterSpec, SessionEventRecord, SubRunEventScope,
};
use serde::Deserialize;

use super::validate_path_id;
use crate::{ApiError, mapper::parse_event_id};

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SessionEventScopeQuery {
    #[serde(rename = "self")]
    SelfOnly,
    #[default]
    Subtree,
    DirectChildren,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionEventFilterQuery {
    pub(crate) sub_run_id: Option<String>,
    pub(crate) scope: Option<SessionEventScopeQuery>,
}

impl SessionEventFilterQuery {
    pub(crate) fn into_runtime_filter_spec(
        self,
    ) -> Result<Option<SessionEventFilterSpec>, ApiError> {
        match (self.sub_run_id, self.scope) {
            (None, None) => Ok(None),
            (None, Some(_)) => Err(ApiError::bad_request("scope requires subRunId".to_string())),
            (Some(sub_run_id), scope) => Ok(Some(SessionEventFilterSpec {
                target_sub_run_id: validate_subrun_query_id(&sub_run_id)?,
                scope: map_scope(scope.unwrap_or_default()),
            })),
        }
    }
}

pub(crate) fn record_is_after_cursor(record: &SessionEventRecord, cursor: Option<&str>) -> bool {
    let Some(cursor) = cursor else {
        return true;
    };
    match (parse_event_id(&record.event_id), parse_event_id(cursor)) {
        (Some(current), Some(after)) => current > after,
        (Some(_), None) => true,
        _ => false,
    }
}

pub(crate) fn record_matches_filter(
    record: &SessionEventRecord,
    filter_spec: &SessionEventFilterSpec,
) -> bool {
    event_matches_filter(&record.event, filter_spec)
}

pub(crate) fn event_matches_filter(
    event: &AgentEvent,
    filter_spec: &SessionEventFilterSpec,
) -> bool {
    let Some(context) = event_context(event) else {
        return false;
    };
    let target = filter_spec.target_sub_run_id.as_str();
    let current = context.sub_run_id.as_deref();
    let parent = context.parent_sub_run_id.as_deref();

    match filter_spec.scope {
        SubRunEventScope::SelfOnly => current == Some(target),
        SubRunEventScope::DirectChildren => parent == Some(target),
        // 目前事件上下文只暴露当前节点和父节点，这里按“自己或直接子节点”做安全子集实现。
        SubRunEventScope::Subtree => current == Some(target) || parent == Some(target),
    }
}

fn map_scope(scope: SessionEventScopeQuery) -> SubRunEventScope {
    match scope {
        SessionEventScopeQuery::SelfOnly => SubRunEventScope::SelfOnly,
        SessionEventScopeQuery::Subtree => SubRunEventScope::Subtree,
        SessionEventScopeQuery::DirectChildren => SubRunEventScope::DirectChildren,
    }
}

fn validate_subrun_query_id(raw_sub_run_id: &str) -> Result<String, ApiError> {
    validate_path_id(raw_sub_run_id, None, false, "sub-run")
}

fn event_context(event: &AgentEvent) -> Option<&AgentEventContext> {
    match event {
        AgentEvent::UserMessage { agent, .. }
        | AgentEvent::PhaseChanged { agent, .. }
        | AgentEvent::ModelDelta { agent, .. }
        | AgentEvent::ThinkingDelta { agent, .. }
        | AgentEvent::AssistantMessage { agent, .. }
        | AgentEvent::ToolCallStart { agent, .. }
        | AgentEvent::ToolCallDelta { agent, .. }
        | AgentEvent::ToolCallResult { agent, .. }
        | AgentEvent::CompactApplied { agent, .. }
        | AgentEvent::SubRunStarted { agent, .. }
        | AgentEvent::SubRunFinished { agent, .. }
        | AgentEvent::ChildSessionNotification { agent, .. }
        | AgentEvent::TurnDone { agent, .. }
        | AgentEvent::Error { agent, .. }
        | AgentEvent::PromptMetrics { agent, .. }
        | AgentEvent::AgentMailboxQueued { agent, .. }
        | AgentEvent::AgentMailboxBatchStarted { agent, .. }
        | AgentEvent::AgentMailboxBatchAcked { agent, .. }
        | AgentEvent::AgentMailboxDiscarded { agent, .. } => Some(agent),
        AgentEvent::SessionStarted { .. } => None,
    }
}
