use astrcode_core::{AgentEvent, SessionEventRecord};
use astrcode_runtime_execution::{ExecutionLineageIndex, ExecutionLineageScope};
use serde::Deserialize;

use super::validate_path_id;
use crate::{ApiError, mapper::parse_event_id};

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SubRunEventScope {
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
    pub(crate) scope: Option<SubRunEventScope>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionEventFilterSpec {
    target_sub_run_id: String,
    scope: SubRunEventScope,
}

impl SessionEventFilterSpec {
    pub(crate) fn from_query(query: SessionEventFilterQuery) -> Result<Option<Self>, ApiError> {
        match (query.sub_run_id, query.scope) {
            (None, None) => Ok(None),
            (None, Some(_)) => Err(ApiError::bad_request("scope requires subRunId".to_string())),
            (Some(sub_run_id), scope) => Ok(Some(Self {
                target_sub_run_id: validate_subrun_query_id(&sub_run_id)?,
                scope: scope.unwrap_or_default(),
            })),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SessionEventFilter {
    spec: SessionEventFilterSpec,
    lineage: ExecutionLineageIndex,
}

impl SessionEventFilter {
    pub(crate) fn new(
        spec: SessionEventFilterSpec,
        history: &[SessionEventRecord],
    ) -> Result<Self, ApiError> {
        let lineage = ExecutionLineageIndex::from_session_history(history);
        lineage
            .require_scope(&spec.target_sub_run_id, map_lineage_scope(spec.scope))
            .map_err(|message| ApiError {
                status: axum::http::StatusCode::CONFLICT,
                message,
            })?;
        Ok(Self { spec, lineage })
    }

    pub(crate) fn matches(&mut self, record: &SessionEventRecord) -> bool {
        self.lineage.observe_session_record(record);
        let Some(event_sub_run_id) = event_sub_run_id(&record.event) else {
            return false;
        };

        match self.spec.scope {
            SubRunEventScope::SelfOnly => event_sub_run_id == self.spec.target_sub_run_id,
            SubRunEventScope::DirectChildren => self
                .lineage
                .is_direct_child_of(event_sub_run_id, &self.spec.target_sub_run_id),
            SubRunEventScope::Subtree => self
                .lineage
                .is_in_subtree(event_sub_run_id, &self.spec.target_sub_run_id),
        }
    }

    pub(crate) fn spec(&self) -> SessionEventFilterSpec {
        self.spec.clone()
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

fn map_lineage_scope(scope: SubRunEventScope) -> ExecutionLineageScope {
    match scope {
        SubRunEventScope::SelfOnly => ExecutionLineageScope::SelfOnly,
        SubRunEventScope::DirectChildren => ExecutionLineageScope::DirectChildren,
        SubRunEventScope::Subtree => ExecutionLineageScope::Subtree,
    }
}

fn validate_subrun_query_id(raw_sub_run_id: &str) -> Result<String, ApiError> {
    validate_path_id(raw_sub_run_id, None, false, "sub-run")
}

fn event_sub_run_id(event: &AgentEvent) -> Option<&str> {
    event_agent_context(event)?
        .sub_run_id
        .as_deref()
        .filter(|sub_run_id| !sub_run_id.is_empty())
}

fn event_agent_context(event: &AgentEvent) -> Option<&astrcode_core::AgentEventContext> {
    match event {
        AgentEvent::SessionStarted { .. } => None,
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
        | AgentEvent::PromptMetrics { agent, .. }
        | AgentEvent::TurnDone { agent, .. }
        | AgentEvent::Error { agent, .. } => Some(agent),
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{
        AgentEventContext, Phase, ResolvedExecutionLimitsSnapshot,
        ResolvedSubagentContextOverrides, SessionEventRecord, SubRunDescriptor, SubRunStorageMode,
    };
    use astrcode_runtime_execution::LINEAGE_METADATA_UNAVAILABLE_MESSAGE;

    use super::*;

    fn record(event_id: &str, event: AgentEvent) -> SessionEventRecord {
        SessionEventRecord {
            event_id: event_id.to_string(),
            event,
        }
    }

    fn root_context() -> AgentEventContext {
        AgentEventContext::root_execution("root-agent", "primary")
    }

    fn sub_context(sub_run_id: &str, parent_turn_id: &str, agent_id: &str) -> AgentEventContext {
        AgentEventContext::sub_run(
            agent_id.to_string(),
            parent_turn_id.to_string(),
            "review",
            sub_run_id.to_string(),
            SubRunStorageMode::SharedSession,
            None,
        )
    }

    #[test]
    fn direct_children_scope_excludes_self_and_grandchildren() {
        let spec = SessionEventFilterSpec {
            target_sub_run_id: "sub-a".to_string(),
            scope: SubRunEventScope::DirectChildren,
        };
        let events = vec![
            record(
                "1.0",
                AgentEvent::UserMessage {
                    turn_id: "turn-root".to_string(),
                    agent: root_context(),
                    content: "root".to_string(),
                },
            ),
            record(
                "2.0",
                AgentEvent::SubRunStarted {
                    turn_id: Some("turn-root".to_string()),
                    agent: sub_context("sub-a", "turn-root", "agent-a"),
                    descriptor: Some(SubRunDescriptor {
                        sub_run_id: "sub-a".to_string(),
                        parent_turn_id: "turn-root".to_string(),
                        parent_agent_id: None,
                        depth: 1,
                    }),
                    tool_call_id: None,
                    resolved_overrides: ResolvedSubagentContextOverrides::default(),
                    resolved_limits: ResolvedExecutionLimitsSnapshot::default(),
                },
            ),
            record(
                "3.0",
                AgentEvent::UserMessage {
                    turn_id: "turn-a".to_string(),
                    agent: sub_context("sub-a", "turn-root", "agent-a"),
                    content: "child".to_string(),
                },
            ),
            record(
                "4.0",
                AgentEvent::SubRunStarted {
                    turn_id: Some("turn-a".to_string()),
                    agent: sub_context("sub-b", "turn-a", "agent-b"),
                    descriptor: Some(SubRunDescriptor {
                        sub_run_id: "sub-b".to_string(),
                        parent_turn_id: "turn-a".to_string(),
                        parent_agent_id: Some("agent-a".to_string()),
                        depth: 2,
                    }),
                    tool_call_id: None,
                    resolved_overrides: ResolvedSubagentContextOverrides::default(),
                    resolved_limits: ResolvedExecutionLimitsSnapshot::default(),
                },
            ),
            record(
                "5.0",
                AgentEvent::UserMessage {
                    turn_id: "turn-b".to_string(),
                    agent: sub_context("sub-b", "turn-a", "agent-b"),
                    content: "grandchild".to_string(),
                },
            ),
            record(
                "6.0",
                AgentEvent::SubRunStarted {
                    turn_id: Some("turn-b".to_string()),
                    agent: sub_context("sub-c", "turn-b", "agent-c"),
                    descriptor: Some(SubRunDescriptor {
                        sub_run_id: "sub-c".to_string(),
                        parent_turn_id: "turn-b".to_string(),
                        parent_agent_id: Some("agent-b".to_string()),
                        depth: 3,
                    }),
                    tool_call_id: None,
                    resolved_overrides: ResolvedSubagentContextOverrides::default(),
                    resolved_limits: ResolvedExecutionLimitsSnapshot::default(),
                },
            ),
        ];

        let mut filter = SessionEventFilter::new(spec, &events).expect("filter should build");
        let matched = events
            .iter()
            .filter(|event| filter.matches(event))
            .map(|event| event.event_id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(matched, vec!["4.0", "5.0"]);
    }

    #[test]
    fn non_self_scope_rejects_legacy_lineage_gap() {
        let spec = SessionEventFilterSpec {
            target_sub_run_id: "sub-legacy".to_string(),
            scope: SubRunEventScope::Subtree,
        };
        let events = vec![record(
            "1.0",
            AgentEvent::SubRunStarted {
                turn_id: Some("turn-legacy".to_string()),
                agent: sub_context("sub-legacy", "turn-legacy", "agent-legacy"),
                descriptor: None,
                tool_call_id: None,
                resolved_overrides: ResolvedSubagentContextOverrides::default(),
                resolved_limits: ResolvedExecutionLimitsSnapshot::default(),
            },
        )];

        let error = SessionEventFilter::new(spec, &events).expect_err("legacy scope should fail");

        assert_eq!(error.status, axum::http::StatusCode::CONFLICT);
        assert_eq!(error.message, LINEAGE_METADATA_UNAVAILABLE_MESSAGE);
    }

    #[test]
    fn record_is_after_cursor_uses_monotonic_event_ids() {
        let record = record(
            "9.1",
            AgentEvent::PhaseChanged {
                turn_id: None,
                agent: AgentEventContext::default(),
                phase: Phase::Idle,
            },
        );

        assert!(record_is_after_cursor(&record, Some("9.0")));
        assert!(!record_is_after_cursor(&record, Some("9.1")));
        assert!(record_is_after_cursor(&record, Some("oops")));
    }
}
