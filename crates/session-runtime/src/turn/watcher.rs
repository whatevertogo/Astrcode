use astrcode_core::{
    AgentEvent, Phase, Result, SessionEventRecord, SessionId, StoredEvent, TurnProjectionSnapshot,
};
use tokio::sync::broadcast::error::RecvError;

use crate::{
    ProjectedTurnOutcome, SessionRuntime, SessionState, TurnTerminalSnapshot,
    query::turn::project_turn_outcome,
    turn::projector::{has_terminal_projection, project_turn_projection},
};

pub(crate) async fn wait_for_turn_terminal_snapshot(
    runtime: &SessionRuntime,
    session_id: &str,
    turn_id: &str,
) -> Result<TurnTerminalSnapshot> {
    let session_id = SessionId::from(crate::state::normalize_session_id(session_id));
    let actor = runtime.ensure_loaded_session(&session_id).await?;
    let state = actor.state();
    let mut receiver = state.broadcaster.subscribe();
    if let Some(snapshot) =
        try_turn_terminal_snapshot(runtime, &session_id, state.as_ref(), turn_id, true).await?
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
                    try_turn_terminal_snapshot_from_recent(state.as_ref(), turn_id)?
                {
                    return Ok(snapshot);
                }
            },
            Err(RecvError::Lagged(_)) => {
                if let Some(snapshot) =
                    try_turn_terminal_snapshot(runtime, &session_id, state.as_ref(), turn_id, true)
                        .await?
                {
                    return Ok(snapshot);
                }
            },
            Err(RecvError::Closed) => {
                if let Some(snapshot) =
                    try_turn_terminal_snapshot(runtime, &session_id, state.as_ref(), turn_id, true)
                        .await?
                {
                    return Ok(snapshot);
                }
                return Err(astrcode_core::AstrError::Internal(format!(
                    "session '{}' broadcaster closed before turn '{}' reached a terminal snapshot",
                    session_id, turn_id
                )));
            },
        }
    }
}

pub(crate) async fn wait_and_project_turn_outcome(
    runtime: &SessionRuntime,
    session_id: &str,
    turn_id: &str,
) -> Result<ProjectedTurnOutcome> {
    let terminal = wait_for_turn_terminal_snapshot(runtime, session_id, turn_id).await?;
    Ok(project_turn_outcome(
        terminal.phase,
        terminal.projection.as_ref(),
        &terminal.events,
    ))
}

pub(crate) async fn try_turn_terminal_snapshot(
    runtime: &SessionRuntime,
    session_id: &SessionId,
    state: &SessionState,
    turn_id: &str,
    allow_durable_fallback: bool,
) -> Result<Option<TurnTerminalSnapshot>> {
    if let Some(snapshot) = try_turn_terminal_snapshot_from_recent(state, turn_id)? {
        return Ok(Some(snapshot));
    }

    if !allow_durable_fallback {
        return Ok(None);
    }

    runtime.ensure_session_exists(session_id).await?;
    let events = turn_events(runtime.event_store.replay(session_id).await?, turn_id);
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

pub(crate) fn try_turn_terminal_snapshot_from_recent(
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

pub(crate) fn turn_events(stored_events: Vec<StoredEvent>, turn_id: &str) -> Vec<StoredEvent> {
    stored_events
        .into_iter()
        .filter(|stored| stored.event.turn_id() == Some(turn_id))
        .collect()
}

pub(crate) fn turn_snapshot_is_terminal(
    phase: Phase,
    projection: Option<&TurnProjectionSnapshot>,
    events: &[StoredEvent],
) -> bool {
    has_terminal_projection(projection)
        || (!events.is_empty() && matches!(phase, Phase::Interrupted))
}

pub(crate) fn record_targets_turn(record: &SessionEventRecord, turn_id: &str) -> bool {
    match &record.event {
        AgentEvent::UserMessage { turn_id: id, .. }
        | AgentEvent::ModelDelta { turn_id: id, .. }
        | AgentEvent::ThinkingDelta { turn_id: id, .. }
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
    use std::{
        path::Path,
        sync::{
            Arc, Mutex,
            atomic::{AtomicU64, AtomicUsize, Ordering},
        },
    };

    use astrcode_core::{
        AgentEventContext, DeleteProjectResult, EventStore, EventTranslator, Phase, Result,
        SessionId, SessionMeta, SessionTurnAcquireResult, StorageEvent, StorageEventPayload,
        StoredEvent, TurnProjectionSnapshot,
    };
    use async_trait::async_trait;
    use tokio::time::{Duration, timeout};

    use super::{turn_snapshot_is_terminal, wait_for_turn_terminal_snapshot};
    use crate::{
        state::append_and_broadcast,
        turn::test_support::{StubEventStore, test_runtime},
    };

    #[test]
    fn turn_snapshot_is_terminal_accepts_replayed_terminal_projection() {
        let projection = TurnProjectionSnapshot {
            terminal_kind: Some(astrcode_core::TurnTerminalKind::Completed),
            last_error: None,
        };

        assert!(turn_snapshot_is_terminal(
            Phase::Idle,
            Some(&projection),
            &[]
        ));
    }

    #[test]
    fn turn_snapshot_is_terminal_accepts_interrupted_phase_with_turn_history() {
        let events = vec![StoredEvent {
            storage_seq: 1,
            event: StorageEvent {
                turn_id: Some("turn-1".to_string()),
                agent: AgentEventContext::default(),
                payload: StorageEventPayload::Error {
                    message: "interrupted".to_string(),
                    timestamp: Some(chrono::Utc::now()),
                },
            },
        }];

        assert!(turn_snapshot_is_terminal(Phase::Interrupted, None, &events));
    }

    #[tokio::test]
    async fn wait_for_turn_terminal_snapshot_wakes_on_broadcast_event() {
        let runtime = test_runtime(Arc::new(StubEventStore::default()));
        let session = runtime
            .create_session(".")
            .await
            .expect("session should be created");
        let session_id = session.session_id.clone();
        let turn_id = "turn-1".to_string();

        let waiter = {
            let runtime = &runtime;
            let session_id = session_id.clone();
            let turn_id = turn_id.clone();
            async move { wait_for_turn_terminal_snapshot(runtime, &session_id, &turn_id).await }
        };

        let state = runtime
            .get_session_state(&session_id.clone().into())
            .await
            .expect("state should load");
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            let mut translator = EventTranslator::new(Phase::Idle);
            append_and_broadcast(
                state.as_ref(),
                &StorageEvent {
                    turn_id: Some(turn_id),
                    agent: AgentEventContext::default(),
                    payload: StorageEventPayload::TurnDone {
                        timestamp: chrono::Utc::now(),
                        terminal_kind: Some(astrcode_core::TurnTerminalKind::Completed),
                        reason: Some("completed".to_string()),
                    },
                },
                &mut translator,
            )
            .await
            .expect("turn done should append");
        });

        let snapshot = timeout(Duration::from_secs(1), waiter)
            .await
            .expect("wait should complete")
            .expect("snapshot should load");

        assert!(turn_snapshot_is_terminal(
            snapshot.phase,
            snapshot.projection.as_ref(),
            &snapshot.events,
        ));
        assert_eq!(snapshot.events.len(), 1);
        assert_eq!(snapshot.events[0].event.turn_id(), Some("turn-1"));
    }

    #[tokio::test]
    async fn wait_for_turn_terminal_snapshot_replays_only_once_while_waiting() {
        let event_store = Arc::new(CountingEventStore::default());
        let runtime = test_runtime(event_store.clone());
        let session = runtime
            .create_session(".")
            .await
            .expect("session should be created");
        let session_id = session.session_id.clone();
        let turn_id = "turn-1".to_string();

        let waiter = {
            let runtime = &runtime;
            let session_id = session_id.clone();
            let turn_id = turn_id.clone();
            async move { wait_for_turn_terminal_snapshot(runtime, &session_id, &turn_id).await }
        };

        let state = runtime
            .get_session_state(&session_id.clone().into())
            .await
            .expect("state should load");
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(75)).await;
            let mut translator = EventTranslator::new(Phase::Idle);
            append_and_broadcast(
                state.as_ref(),
                &StorageEvent {
                    turn_id: Some(turn_id),
                    agent: AgentEventContext::default(),
                    payload: StorageEventPayload::TurnDone {
                        timestamp: chrono::Utc::now(),
                        terminal_kind: Some(astrcode_core::TurnTerminalKind::Completed),
                        reason: Some("completed".to_string()),
                    },
                },
                &mut translator,
            )
            .await
            .expect("turn done should append");
        });

        timeout(Duration::from_secs(1), waiter)
            .await
            .expect("wait should complete")
            .expect("snapshot should load");

        assert_eq!(
            event_store.replay_count(),
            1,
            "live wait should not repeatedly rescan durable history"
        );
    }

    #[tokio::test]
    async fn wait_for_turn_terminal_snapshot_projects_legacy_reason_history() {
        let runtime = test_runtime(Arc::new(StubEventStore::default()));
        let session = runtime
            .create_session(".")
            .await
            .expect("session should be created");
        let session_id = session.session_id.clone();
        let state = runtime
            .get_session_state(&session_id.clone().into())
            .await
            .expect("state should load");

        let mut translator = EventTranslator::new(Phase::Idle);
        append_and_broadcast(
            state.as_ref(),
            &StorageEvent {
                turn_id: Some("turn-legacy".to_string()),
                agent: AgentEventContext::default(),
                payload: StorageEventPayload::TurnDone {
                    timestamp: chrono::Utc::now(),
                    terminal_kind: None,
                    reason: Some("token_exceeded".to_string()),
                },
            },
            &mut translator,
        )
        .await
        .expect("legacy turn done should append");

        let snapshot = wait_for_turn_terminal_snapshot(&runtime, &session_id, "turn-legacy")
            .await
            .expect("terminal snapshot should load");
        let outcome = runtime
            .project_turn_outcome(&session_id, "turn-legacy")
            .await
            .expect("turn outcome should project");

        assert_eq!(
            snapshot
                .projection
                .as_ref()
                .and_then(|projection| projection.terminal_kind.clone()),
            Some(astrcode_core::TurnTerminalKind::MaxOutputContinuationLimitReached)
        );
        assert_eq!(
            outcome.outcome,
            astrcode_core::AgentTurnOutcome::TokenExceeded
        );
    }

    #[derive(Debug, Default)]
    struct CountingEventStore {
        events: Mutex<Vec<StoredEvent>>,
        next_seq: AtomicU64,
        replay_count: AtomicUsize,
    }

    impl CountingEventStore {
        fn replay_count(&self) -> usize {
            self.replay_count.load(Ordering::SeqCst)
        }
    }

    struct CountingTurnLease;

    impl astrcode_core::SessionTurnLease for CountingTurnLease {}

    #[async_trait]
    impl EventStore for CountingEventStore {
        async fn ensure_session(&self, _session_id: &SessionId, _working_dir: &Path) -> Result<()> {
            Ok(())
        }

        async fn append(
            &self,
            _session_id: &SessionId,
            event: &StorageEvent,
        ) -> Result<StoredEvent> {
            let stored = StoredEvent {
                storage_seq: self.next_seq.fetch_add(1, Ordering::SeqCst) + 1,
                event: event.clone(),
            };
            self.events
                .lock()
                .expect("counting event store should lock")
                .push(stored.clone());
            Ok(stored)
        }

        async fn replay(&self, _session_id: &SessionId) -> Result<Vec<StoredEvent>> {
            self.replay_count.fetch_add(1, Ordering::SeqCst);
            Ok(self
                .events
                .lock()
                .expect("counting event store should lock")
                .clone())
        }

        async fn try_acquire_turn(
            &self,
            _session_id: &SessionId,
            _turn_id: &str,
        ) -> Result<SessionTurnAcquireResult> {
            Ok(SessionTurnAcquireResult::Acquired(Box::new(
                CountingTurnLease,
            )))
        }

        async fn list_sessions(&self) -> Result<Vec<SessionId>> {
            Ok(vec![])
        }

        async fn list_session_metas(&self) -> Result<Vec<SessionMeta>> {
            Ok(vec![])
        }

        async fn delete_session(&self, _session_id: &SessionId) -> Result<()> {
            Ok(())
        }

        async fn delete_sessions_by_working_dir(
            &self,
            _working_dir: &str,
        ) -> Result<DeleteProjectResult> {
            Ok(DeleteProjectResult {
                success_count: 0,
                failed_session_ids: Vec::new(),
            })
        }
    }
}
