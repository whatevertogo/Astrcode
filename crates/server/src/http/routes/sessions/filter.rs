use std::collections::HashMap;

use astrcode_core::{AgentEvent, SessionEventRecord};
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
    turn_owner: HashMap<String, Option<String>>,
    sub_run_parent: HashMap<String, Option<String>>,
}

impl SessionEventFilter {
    pub(crate) fn new(spec: SessionEventFilterSpec) -> Self {
        Self {
            spec,
            turn_owner: HashMap::new(),
            sub_run_parent: HashMap::new(),
        }
    }

    pub(crate) fn matches(&mut self, record: &SessionEventRecord) -> bool {
        self.observe_turn_owner(&record.event);
        self.observe_sub_run_parent(&record.event);

        let Some(event_sub_run_id) = event_sub_run_id(&record.event, &self.turn_owner) else {
            return false;
        };
        self.matches_scope(event_sub_run_id)
    }

    fn matches_scope(&self, event_sub_run_id: &str) -> bool {
        if event_sub_run_id == self.spec.target_sub_run_id {
            return true;
        }

        match self.spec.scope {
            SubRunEventScope::SelfOnly => false,
            SubRunEventScope::DirectChildren => {
                self.sub_run_parent
                    .get(event_sub_run_id)
                    .and_then(|parent| parent.as_deref())
                    == Some(self.spec.target_sub_run_id.as_str())
            },
            SubRunEventScope::Subtree => self.is_descendant_of(event_sub_run_id),
        }
    }

    fn is_descendant_of(&self, event_sub_run_id: &str) -> bool {
        let mut current = self
            .sub_run_parent
            .get(event_sub_run_id)
            .and_then(|id| id.as_deref());
        while let Some(sub_run_id) = current {
            if sub_run_id == self.spec.target_sub_run_id {
                return true;
            }
            current = self
                .sub_run_parent
                .get(sub_run_id)
                .and_then(|id| id.as_deref());
        }
        false
    }

    fn observe_turn_owner(&mut self, event: &AgentEvent) {
        if !event_establishes_turn_owner(event) {
            return;
        }
        let Some(turn_id) = event_turn_id(event) else {
            return;
        };
        self.turn_owner
            .entry(turn_id.to_string())
            .or_insert_with(|| event_agent_sub_run_id(event).map(ToOwned::to_owned));
    }

    fn observe_sub_run_parent(&mut self, event: &AgentEvent) {
        let Some(sub_run_id) = event_agent_sub_run_id(event) else {
            return;
        };
        if self.sub_run_parent.contains_key(sub_run_id) {
            return;
        }
        let parent_sub_run_id = event_parent_turn_id(event)
            .and_then(|parent_turn_id| self.turn_owner.get(parent_turn_id))
            .cloned()
            .flatten();
        self.sub_run_parent
            .insert(sub_run_id.to_string(), parent_sub_run_id);
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

fn validate_subrun_query_id(raw_sub_run_id: &str) -> Result<String, ApiError> {
    validate_path_id(raw_sub_run_id, None, false, "sub-run")
}

fn event_turn_id(event: &AgentEvent) -> Option<&str> {
    match event {
        AgentEvent::SessionStarted { .. } => None,
        AgentEvent::UserMessage { turn_id, .. }
        | AgentEvent::ModelDelta { turn_id, .. }
        | AgentEvent::ThinkingDelta { turn_id, .. }
        | AgentEvent::AssistantMessage { turn_id, .. }
        | AgentEvent::ToolCallStart { turn_id, .. }
        | AgentEvent::ToolCallDelta { turn_id, .. }
        | AgentEvent::ToolCallResult { turn_id, .. }
        | AgentEvent::TurnDone { turn_id, .. } => Some(turn_id.as_str()),
        AgentEvent::PhaseChanged { turn_id, .. }
        | AgentEvent::CompactApplied { turn_id, .. }
        | AgentEvent::SubRunStarted { turn_id, .. }
        | AgentEvent::SubRunFinished { turn_id, .. }
        | AgentEvent::PromptMetrics { turn_id, .. }
        | AgentEvent::Error { turn_id, .. } => turn_id.as_deref(),
    }
}

fn event_parent_turn_id(event: &AgentEvent) -> Option<&str> {
    event_agent_context(event)?
        .parent_turn_id
        .as_deref()
        .filter(|parent_turn_id| !parent_turn_id.is_empty())
}

fn event_agent_sub_run_id(event: &AgentEvent) -> Option<&str> {
    event_agent_context(event)?
        .sub_run_id
        .as_deref()
        .filter(|sub_run_id| !sub_run_id.is_empty())
}

fn event_sub_run_id<'a>(
    event: &'a AgentEvent,
    turn_owner: &'a HashMap<String, Option<String>>,
) -> Option<&'a str> {
    event_agent_sub_run_id(event).or_else(|| {
        event_turn_id(event)
            .and_then(|turn_id| turn_owner.get(turn_id))
            .and_then(|sub_run_id| sub_run_id.as_deref())
    })
}

fn event_establishes_turn_owner(event: &AgentEvent) -> bool {
    matches!(
        event,
        AgentEvent::UserMessage { .. }
            | AgentEvent::ModelDelta { .. }
            | AgentEvent::ThinkingDelta { .. }
            | AgentEvent::AssistantMessage { .. }
            | AgentEvent::ToolCallStart { .. }
            | AgentEvent::ToolCallDelta { .. }
            | AgentEvent::ToolCallResult { .. }
            | AgentEvent::TurnDone { .. }
            | AgentEvent::Error { .. }
    )
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
        | AgentEvent::PromptMetrics { agent, .. }
        | AgentEvent::TurnDone { agent, .. }
        | AgentEvent::Error { agent, .. } => Some(agent),
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{AgentEventContext, Phase};

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

    fn sub_context(sub_run_id: &str, parent_turn_id: &str) -> AgentEventContext {
        AgentEventContext::sub_run(
            format!("agent-{sub_run_id}"),
            parent_turn_id.to_string(),
            "review",
            sub_run_id.to_string(),
            astrcode_core::SubRunStorageMode::SharedSession,
            None,
        )
    }

    #[test]
    fn direct_children_scope_excludes_grandchildren() {
        let spec = SessionEventFilterSpec {
            target_sub_run_id: "sub-a".to_string(),
            scope: SubRunEventScope::DirectChildren,
        };
        let mut filter = SessionEventFilter::new(spec);
        let events = [
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
                    agent: sub_context("sub-a", "turn-root"),
                    resolved_overrides: astrcode_core::ResolvedSubagentContextOverrides::default(),
                    resolved_limits: astrcode_core::ResolvedExecutionLimitsSnapshot::default(),
                },
            ),
            record(
                "3.0",
                AgentEvent::UserMessage {
                    turn_id: "turn-a".to_string(),
                    agent: sub_context("sub-a", "turn-root"),
                    content: "child".to_string(),
                },
            ),
            record(
                "4.0",
                AgentEvent::SubRunStarted {
                    turn_id: Some("turn-a".to_string()),
                    agent: sub_context("sub-b", "turn-a"),
                    resolved_overrides: astrcode_core::ResolvedSubagentContextOverrides::default(),
                    resolved_limits: astrcode_core::ResolvedExecutionLimitsSnapshot::default(),
                },
            ),
            record(
                "5.0",
                AgentEvent::UserMessage {
                    turn_id: "turn-b".to_string(),
                    agent: sub_context("sub-b", "turn-a"),
                    content: "grandchild".to_string(),
                },
            ),
            record(
                "6.0",
                AgentEvent::SubRunStarted {
                    turn_id: Some("turn-b".to_string()),
                    agent: sub_context("sub-c", "turn-b"),
                    resolved_overrides: astrcode_core::ResolvedSubagentContextOverrides::default(),
                    resolved_limits: astrcode_core::ResolvedExecutionLimitsSnapshot::default(),
                },
            ),
        ];

        let matched = events
            .iter()
            .filter(|event| filter.matches(event))
            .map(|event| event.event_id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(matched, vec!["2.0", "3.0", "4.0", "5.0"]);
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
