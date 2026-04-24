use std::sync::Arc;

use astrcode_core::{
    AgentEvent, ChildSessionNode, CompactAppliedMeta, CompactTrigger, ModeId, Phase, Result,
    SessionEventRecord, SessionId, StoredEvent, TaskSnapshot, TurnTerminalKind,
};
use chrono::{DateTime, Utc};
use tokio::sync::broadcast::error::RecvError;

use crate::{
    InputQueueProjection, SessionCatalog, SessionSnapshot, SessionState, TurnProjectionSnapshot,
    turn_projection::project_turn_projection,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionObserveSnapshot {
    pub state: SessionSnapshot,
}

#[derive(Debug, Clone)]
pub struct SessionReadModelReplay {
    pub cursor: Option<String>,
    pub seed_records: Vec<SessionEventRecord>,
    pub history: Vec<SessionEventRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastCompactMetaSnapshot {
    pub trigger: CompactTrigger,
    pub meta: CompactAppliedMeta,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionControlStateSnapshot {
    pub phase: Phase,
    pub active_turn_id: Option<String>,
    pub manual_compact_pending: bool,
    pub compacting: bool,
    pub last_compact_meta: Option<LastCompactMetaSnapshot>,
    pub current_mode_id: ModeId,
    pub last_mode_changed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct TurnTerminalSnapshot {
    pub phase: Phase,
    pub projection: Option<TurnProjectionSnapshot>,
    pub events: Vec<StoredEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectedTurnOutcome {
    Completed {
        summary: String,
    },
    Cancelled {
        summary: String,
    },
    Failed {
        summary: String,
        technical_message: String,
    },
}

impl SessionCatalog {
    pub async fn session_state(&self, session_id: &SessionId) -> Result<Arc<SessionState>> {
        Ok(Arc::clone(
            &self.ensure_loaded_session(session_id).await?.state,
        ))
    }

    pub async fn observe_session(&self, session_id: &SessionId) -> Result<SessionObserveSnapshot> {
        let loaded = self.ensure_loaded_session(session_id).await?;
        let projected = loaded.state.snapshot_projected_state()?;
        let latest_turn_id = loaded
            .state
            .snapshot_recent_stored_events()?
            .into_iter()
            .rev()
            .find_map(|stored| stored.event.turn_id);
        Ok(SessionObserveSnapshot {
            state: SessionSnapshot {
                session_id: loaded.session_id.clone(),
                working_dir: loaded.working_dir.display().to_string(),
                latest_turn_id: latest_turn_id.map(Into::into),
                turn_count: projected.turn_count,
            },
        })
    }

    pub async fn session_child_nodes(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<ChildSessionNode>> {
        self.ensure_loaded_session(session_id)
            .await?
            .state
            .list_child_session_nodes()
    }

    pub async fn active_task_snapshot(
        &self,
        session_id: &SessionId,
        owner: &str,
    ) -> Result<Option<TaskSnapshot>> {
        self.ensure_loaded_session(session_id)
            .await?
            .state
            .active_tasks_for(owner)
    }

    pub async fn session_control_state(
        &self,
        session_id: &SessionId,
    ) -> Result<SessionControlStateSnapshot> {
        let loaded = self.ensure_loaded_session(session_id).await?;
        let last_compact_meta = loaded
            .state
            .snapshot_recent_stored_events()?
            .into_iter()
            .rev()
            .find_map(|stored| match stored.event.payload {
                astrcode_core::StorageEventPayload::CompactApplied { trigger, meta, .. } => {
                    Some(LastCompactMetaSnapshot { trigger, meta })
                },
                _ => None,
            });
        let (active_turn_id, manual_compact_pending) =
            if let Some(state) = self.turn_mutations.get(session_id) {
                (
                    state
                        .active_turn_id_snapshot()?
                        .map(|turn_id| turn_id.to_string()),
                    state.has_pending_manual_compact()?,
                )
            } else {
                (None, false)
            };

        Ok(SessionControlStateSnapshot {
            phase: loaded.state.current_phase()?,
            active_turn_id,
            manual_compact_pending,
            compacting: false,
            last_compact_meta,
            current_mode_id: loaded.state.current_mode_id()?,
            last_mode_changed_at: loaded.state.last_mode_changed_at()?,
        })
    }

    pub async fn input_queue_projection_for_agent(
        &self,
        session_id: &SessionId,
        agent_id: &str,
    ) -> Result<InputQueueProjection> {
        self.ensure_loaded_session(session_id)
            .await?
            .state
            .input_queue_projection_for_agent(agent_id)
    }

    pub async fn pending_delivery_ids_for_agent(
        &self,
        session_id: &SessionId,
        agent_id: &str,
    ) -> Result<Vec<String>> {
        Ok(self
            .input_queue_projection_for_agent(session_id, agent_id)
            .await?
            .pending_delivery_ids
            .into_iter()
            .map(Into::into)
            .collect())
    }

    pub async fn stored_events(&self, session_id: &SessionId) -> Result<Vec<StoredEvent>> {
        self.ensure_session_exists(session_id).await?;
        self.event_store.replay(session_id).await
    }

    pub async fn conversation_stream_replay(
        &self,
        session_id: &SessionId,
        last_event_id: Option<&str>,
    ) -> Result<SessionReadModelReplay> {
        self.ensure_session_exists(session_id).await?;
        let full = astrcode_core::replay_records(&self.event_store.replay(session_id).await?, None);
        let (seed_records, history) = split_records_at_cursor(full, last_event_id);
        Ok(SessionReadModelReplay {
            cursor: history.last().map(|record| record.event_id.clone()),
            seed_records,
            history,
        })
    }

    pub async fn try_turn_terminal_snapshot(
        &self,
        session_id: &SessionId,
        turn_id: &str,
        allow_durable_fallback: bool,
    ) -> Result<Option<TurnTerminalSnapshot>> {
        let loaded = self.ensure_loaded_session(session_id).await?;
        if let Some(snapshot) = try_turn_terminal_snapshot_from_recent(&loaded.state, turn_id)? {
            return Ok(Some(snapshot));
        }

        if !allow_durable_fallback {
            return Ok(None);
        }

        let events = turn_events(self.event_store.replay(session_id).await?, turn_id);
        let phase = loaded.state.current_phase()?;
        let projection = loaded
            .state
            .turn_projection(turn_id)?
            .or_else(|| project_turn_projection(&events));
        if turn_snapshot_is_terminal(phase, projection.as_ref(), &events) {
            return Ok(Some(TurnTerminalSnapshot {
                phase,
                projection,
                events,
            }));
        }

        Ok(None)
    }

    pub async fn wait_for_turn_terminal_snapshot(
        &self,
        session_id: &SessionId,
        turn_id: &str,
    ) -> Result<TurnTerminalSnapshot> {
        let loaded = self.ensure_loaded_session(session_id).await?;
        let mut receiver = loaded.state.broadcaster.subscribe();
        if let Some(snapshot) = self
            .try_turn_terminal_snapshot(session_id, turn_id, true)
            .await?
        {
            return Ok(snapshot);
        }
        loop {
            match receiver.recv().await {
                Ok(record) => {
                    if !record_targets_turn(&record, turn_id) {
                        continue;
                    }
                    if let Some(snapshot) =
                        try_turn_terminal_snapshot_from_recent(&loaded.state, turn_id)?
                    {
                        return Ok(snapshot);
                    }
                },
                Err(RecvError::Lagged(_)) => {
                    if let Some(snapshot) = self
                        .try_turn_terminal_snapshot(session_id, turn_id, true)
                        .await?
                    {
                        return Ok(snapshot);
                    }
                },
                Err(RecvError::Closed) => {
                    if let Some(snapshot) = self
                        .try_turn_terminal_snapshot(session_id, turn_id, true)
                        .await?
                    {
                        return Ok(snapshot);
                    }
                    return Err(astrcode_core::AstrError::Internal(format!(
                        "session '{}' broadcaster closed before turn '{}' reached a terminal \
                         snapshot",
                        session_id, turn_id
                    )));
                },
            }
        }
    }

    pub async fn project_turn_outcome(
        &self,
        session_id: &SessionId,
        turn_id: &str,
    ) -> Result<ProjectedTurnOutcome> {
        let terminal = self
            .wait_for_turn_terminal_snapshot(session_id, turn_id)
            .await?;
        Ok(project_turn_outcome(
            terminal.phase,
            terminal.projection.as_ref(),
            &terminal.events,
        ))
    }
}

pub fn try_turn_terminal_snapshot_from_recent(
    state: &SessionState,
    turn_id: &str,
) -> Result<Option<TurnTerminalSnapshot>> {
    let events = turn_events(state.snapshot_recent_stored_events()?, turn_id);
    let phase = state.current_phase()?;
    let projection = state
        .turn_projection(turn_id)?
        .or_else(|| project_turn_projection(&events));
    if turn_snapshot_is_terminal(phase, projection.as_ref(), &events) {
        return Ok(Some(TurnTerminalSnapshot {
            phase,
            projection,
            events,
        }));
    }

    Ok(None)
}

fn split_records_at_cursor(
    mut records: Vec<SessionEventRecord>,
    last_event_id: Option<&str>,
) -> (Vec<SessionEventRecord>, Vec<SessionEventRecord>) {
    let Some(last_event_id) = last_event_id else {
        return (Vec::new(), records);
    };
    let Some(index) = records
        .iter()
        .position(|record| record.event_id == last_event_id)
    else {
        return (Vec::new(), records);
    };
    let history = records.split_off(index + 1);
    (records, history)
}

fn turn_events(stored_events: Vec<StoredEvent>, turn_id: &str) -> Vec<StoredEvent> {
    stored_events
        .into_iter()
        .filter(|stored| stored.event.turn_id() == Some(turn_id))
        .collect()
}

fn turn_snapshot_is_terminal(
    phase: Phase,
    projection: Option<&TurnProjectionSnapshot>,
    events: &[StoredEvent],
) -> bool {
    has_terminal_projection(projection)
        || (!events.is_empty() && matches!(phase, Phase::Interrupted))
}

fn has_terminal_projection(projection: Option<&TurnProjectionSnapshot>) -> bool {
    projection.is_some_and(|projection| {
        projection.terminal_kind.is_some() || projection.last_error.is_some()
    })
}

fn project_turn_outcome(
    phase: Phase,
    projection: Option<&TurnProjectionSnapshot>,
    events: &[StoredEvent],
) -> ProjectedTurnOutcome {
    let replayed_projection = project_turn_projection(events);
    let projection = projection.or(replayed_projection.as_ref());
    let last_assistant = last_non_empty_assistant_event(events);
    let last_error = last_non_empty_error_event(events);
    let terminal_kind = resolve_terminal_kind(phase, projection, last_error.as_deref());

    match terminal_kind.as_ref() {
        Some(TurnTerminalKind::Cancelled) => ProjectedTurnOutcome::Cancelled {
            summary: last_error.unwrap_or_else(|| "child agent cancelled".to_string()),
        },
        Some(TurnTerminalKind::Error { message }) => ProjectedTurnOutcome::Failed {
            summary: last_error
                .clone()
                .or(last_assistant)
                .unwrap_or_else(|| "child agent failed without readable output".to_string()),
            technical_message: last_error.unwrap_or_else(|| message.clone()),
        },
        Some(TurnTerminalKind::Completed) | None => ProjectedTurnOutcome::Completed {
            summary: last_assistant
                .unwrap_or_else(|| "child agent completed without readable output".to_string()),
        },
    }
}

fn resolve_terminal_kind(
    phase: Phase,
    projection: Option<&TurnProjectionSnapshot>,
    last_error: Option<&str>,
) -> Option<TurnTerminalKind> {
    if let Some(turn_done_kind) = projection.and_then(|projection| projection.terminal_kind.clone())
    {
        return Some(turn_done_kind);
    }
    if matches!(phase, Phase::Interrupted) {
        return Some(TurnTerminalKind::Cancelled);
    }
    projection
        .and_then(|projection| projection.last_error.as_deref())
        .or(last_error)
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(|message| TurnTerminalKind::Error {
            message: message.to_string(),
        })
}

fn last_non_empty_assistant_event(events: &[StoredEvent]) -> Option<String> {
    events
        .iter()
        .rev()
        .find_map(|stored| match &stored.event.payload {
            astrcode_core::StorageEventPayload::AssistantFinal { content, .. }
                if !content.trim().is_empty() =>
            {
                Some(content.trim().to_string())
            },
            _ => None,
        })
}

fn last_non_empty_error_event(events: &[StoredEvent]) -> Option<String> {
    events
        .iter()
        .rev()
        .find_map(|stored| match &stored.event.payload {
            astrcode_core::StorageEventPayload::Error { message, .. }
                if !message.trim().is_empty() =>
            {
                Some(message.trim().to_string())
            },
            _ => None,
        })
}

fn record_targets_turn(record: &SessionEventRecord, turn_id: &str) -> bool {
    match &record.event {
        AgentEvent::UserMessage { turn_id: id, .. }
        | AgentEvent::ModelDelta { turn_id: id, .. }
        | AgentEvent::ThinkingDelta { turn_id: id, .. }
        | AgentEvent::StreamRetryStarted { turn_id: id, .. }
        | AgentEvent::AssistantMessage { turn_id: id, .. }
        | AgentEvent::ToolCallStart { turn_id: id, .. }
        | AgentEvent::ToolCallDelta { turn_id: id, .. }
        | AgentEvent::ToolCallResult { turn_id: id, .. }
        | AgentEvent::TurnDone { turn_id: id, .. } => id == turn_id,
        AgentEvent::PhaseChanged {
            turn_id: Some(id), ..
        }
        | AgentEvent::PromptMetrics {
            turn_id: Some(id), ..
        }
        | AgentEvent::CompactApplied {
            turn_id: Some(id), ..
        }
        | AgentEvent::SubRunStarted {
            turn_id: Some(id), ..
        }
        | AgentEvent::SubRunFinished {
            turn_id: Some(id), ..
        }
        | AgentEvent::ChildSessionNotification {
            turn_id: Some(id), ..
        }
        | AgentEvent::AgentInputQueued {
            turn_id: Some(id), ..
        }
        | AgentEvent::AgentInputBatchStarted {
            turn_id: Some(id), ..
        }
        | AgentEvent::AgentInputBatchAcked {
            turn_id: Some(id), ..
        }
        | AgentEvent::AgentInputDiscarded {
            turn_id: Some(id), ..
        }
        | AgentEvent::Error {
            turn_id: Some(id), ..
        } => id == turn_id,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{
        AgentEventContext, Phase, StorageEvent, StorageEventPayload, StoredEvent, TurnTerminalKind,
    };

    use super::{ProjectedTurnOutcome, project_turn_outcome, turn_snapshot_is_terminal};
    use crate::TurnProjectionSnapshot;

    #[test]
    fn turn_snapshot_is_terminal_accepts_projection_terminal_kind() {
        assert!(turn_snapshot_is_terminal(
            Phase::Idle,
            Some(&TurnProjectionSnapshot {
                terminal_kind: Some(TurnTerminalKind::Completed),
                last_error: None,
            }),
            &[]
        ));
    }

    #[test]
    fn project_turn_outcome_prefers_assistant_summary_on_success() {
        let outcome = project_turn_outcome(
            Phase::Idle,
            Some(&TurnProjectionSnapshot {
                terminal_kind: Some(TurnTerminalKind::Completed),
                last_error: None,
            }),
            &[StoredEvent {
                storage_seq: 1,
                event: StorageEvent {
                    turn_id: Some("turn-1".to_string()),
                    agent: AgentEventContext::default(),
                    payload: StorageEventPayload::AssistantFinal {
                        content: "done".to_string(),
                        reasoning_content: None,
                        reasoning_signature: None,
                        step_index: None,
                        timestamp: Some(chrono::Utc::now()),
                    },
                },
            }],
        );

        assert_eq!(
            outcome,
            ProjectedTurnOutcome::Completed {
                summary: "done".to_string()
            }
        );
    }
}
