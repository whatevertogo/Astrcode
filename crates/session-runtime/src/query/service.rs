use std::sync::Arc;

use astrcode_core::{
    AgentEvent, AgentLifecycleStatus, ChildSessionNode, Phase, Result, SessionEventRecord,
    SessionId, StorageEventPayload, StoredEvent,
};
use tokio::sync::broadcast::error::RecvError;

use crate::{
    AgentObserveSnapshot, ConversationSnapshotFacts, ConversationStreamReplayFacts,
    LastCompactMetaSnapshot, ProjectedTurnOutcome, SessionControlStateSnapshot,
    SessionModeSnapshot, SessionReplay, SessionRuntime, SessionState, TurnTerminalSnapshot,
    query::{
        agent::build_agent_observe_snapshot,
        conversation::{build_conversation_replay_frames, project_conversation_snapshot},
        input_queue::recoverable_parent_deliveries,
        turn::{has_terminal_turn_signal, project_turn_outcome},
    },
};

pub(crate) struct SessionQueries<'a> {
    runtime: &'a SessionRuntime,
}

impl<'a> SessionQueries<'a> {
    pub fn new(runtime: &'a SessionRuntime) -> Self {
        Self { runtime }
    }

    pub async fn observe(
        &self,
        session_id: &SessionId,
    ) -> Result<crate::observe::SessionObserveSnapshot> {
        let actor = self.runtime.ensure_loaded_session(session_id).await?;
        Ok(crate::observe::SessionObserveSnapshot {
            state: actor.snapshot(),
        })
    }

    pub async fn session_state(&self, session_id: &SessionId) -> Result<Arc<SessionState>> {
        let actor = self.runtime.ensure_loaded_session(session_id).await?;
        Ok(Arc::clone(actor.state()))
    }

    pub async fn session_control_state(
        &self,
        session_id: &str,
    ) -> Result<SessionControlStateSnapshot> {
        let session_id = SessionId::from(crate::normalize_session_id(session_id));
        let actor = self.runtime.ensure_loaded_session(&session_id).await?;
        let last_compact_meta = actor
            .state()
            .snapshot_recent_stored_events()?
            .into_iter()
            .rev()
            .find_map(|stored| match stored.event.payload {
                StorageEventPayload::CompactApplied { trigger, meta, .. } => {
                    Some(LastCompactMetaSnapshot { trigger, meta })
                },
                _ => None,
            });
        Ok(SessionControlStateSnapshot {
            phase: actor.state().current_phase()?,
            active_turn_id: actor.state().active_turn_id_snapshot()?,
            manual_compact_pending: actor.state().manual_compact_pending()?,
            compacting: actor.state().compacting(),
            last_compact_meta,
            current_mode_id: actor.state().current_mode_id()?,
            last_mode_changed_at: actor.state().last_mode_changed_at()?,
        })
    }

    pub async fn session_child_nodes(&self, session_id: &str) -> Result<Vec<ChildSessionNode>> {
        let session_id = SessionId::from(crate::normalize_session_id(session_id));
        let actor = self.runtime.ensure_loaded_session(&session_id).await?;
        actor.state().list_child_session_nodes()
    }

    pub async fn session_mode_state(&self, session_id: &str) -> Result<SessionModeSnapshot> {
        let session_id = SessionId::from(crate::normalize_session_id(session_id));
        let actor = self.runtime.ensure_loaded_session(&session_id).await?;
        Ok(SessionModeSnapshot {
            current_mode_id: actor.state().current_mode_id()?,
            last_mode_changed_at: actor.state().last_mode_changed_at()?,
        })
    }

    pub async fn session_working_dir(&self, session_id: &str) -> Result<String> {
        let session_id = SessionId::from(crate::normalize_session_id(session_id));
        let actor = self.runtime.ensure_loaded_session(&session_id).await?;
        Ok(actor.working_dir().to_string())
    }

    pub async fn stored_events(&self, session_id: &SessionId) -> Result<Vec<StoredEvent>> {
        self.runtime.ensure_session_exists(session_id).await?;
        self.runtime.event_store.replay(session_id).await
    }

    pub async fn wait_for_turn_terminal_snapshot(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> Result<TurnTerminalSnapshot> {
        let session_id = SessionId::from(crate::normalize_session_id(session_id));
        let state = self.session_state(&session_id).await?;
        let mut receiver = state.broadcaster.subscribe();
        if let Some(snapshot) = self
            .try_turn_terminal_snapshot(&session_id, state.as_ref(), turn_id, true)
            .await?
        {
            return Ok(snapshot);
        }
        loop {
            match receiver.recv().await {
                Ok(record) => {
                    if !record_targets_turn(&record, turn_id)
                        && !matches!(state.current_phase()?, Phase::Interrupted)
                    {
                        continue;
                    }
                    if let Some(snapshot) =
                        try_turn_terminal_snapshot_from_recent(state.as_ref(), turn_id)?
                    {
                        return Ok(snapshot);
                    }
                },
                Err(RecvError::Lagged(_)) => {
                    if let Some(snapshot) = self
                        .try_turn_terminal_snapshot(&session_id, state.as_ref(), turn_id, true)
                        .await?
                    {
                        return Ok(snapshot);
                    }
                },
                Err(RecvError::Closed) => {
                    if let Some(snapshot) = self
                        .try_turn_terminal_snapshot(&session_id, state.as_ref(), turn_id, true)
                        .await?
                    {
                        return Ok(snapshot);
                    }
                    receiver = state.broadcaster.subscribe();
                },
            }
        }
    }

    pub async fn observe_agent_session(
        &self,
        open_session_id: &str,
        target_agent_id: &str,
        lifecycle_status: AgentLifecycleStatus,
    ) -> Result<AgentObserveSnapshot> {
        let session_id = SessionId::from(crate::normalize_session_id(open_session_id));
        let session_state = self.session_state(&session_id).await?;
        let projected = session_state.snapshot_projected_state()?;
        let input_queue_projection =
            session_state.input_queue_projection_for_agent(target_agent_id)?;
        Ok(build_agent_observe_snapshot(
            lifecycle_status,
            &projected,
            &input_queue_projection,
        ))
    }

    pub async fn conversation_snapshot(
        &self,
        session_id: &str,
    ) -> Result<ConversationSnapshotFacts> {
        let session_id = SessionId::from(crate::normalize_session_id(session_id));
        let records = self.runtime.replay_history(&session_id, None).await?;
        let phase = self.runtime.session_phase(&session_id).await?;
        Ok(project_conversation_snapshot(&records, phase))
    }

    pub async fn conversation_stream_replay(
        &self,
        session_id: &str,
        last_event_id: Option<&str>,
    ) -> Result<ConversationStreamReplayFacts> {
        let session_id = SessionId::from(crate::normalize_session_id(session_id));
        let actor = self.runtime.ensure_loaded_session(&session_id).await?;
        let full_history = self.runtime.replay_history(&session_id, None).await?;
        let (seed_records, replay_history) = split_records_at_cursor(full_history, last_event_id);
        let phase = self.runtime.session_phase(&session_id).await?;

        Ok(ConversationStreamReplayFacts {
            cursor: replay_history.last().map(|record| record.event_id.clone()),
            phase,
            replay_frames: build_conversation_replay_frames(&seed_records, &replay_history),
            seed_records,
            replay: SessionReplay {
                history: replay_history,
                receiver: actor.state().broadcaster.subscribe(),
                live_receiver: actor.state().subscribe_live(),
            },
        })
    }

    pub async fn pending_delivery_ids_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Vec<String>> {
        let session_id = SessionId::from(crate::normalize_session_id(session_id));
        let session_state = self.session_state(&session_id).await?;
        Ok(session_state
            .input_queue_projection_for_agent(agent_id)?
            .pending_delivery_ids
            .into_iter()
            .map(Into::into)
            .collect())
    }

    pub async fn recoverable_parent_deliveries(
        &self,
        parent_session_id: &str,
    ) -> Result<Vec<astrcode_kernel::PendingParentDelivery>> {
        let session_id = SessionId::from(crate::normalize_session_id(parent_session_id));
        let events = self.stored_events(&session_id).await?;
        Ok(recoverable_parent_deliveries(&events))
    }

    pub async fn project_turn_outcome(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> Result<ProjectedTurnOutcome> {
        let terminal = self
            .wait_for_turn_terminal_snapshot(session_id, turn_id)
            .await?;
        Ok(project_turn_outcome(terminal.phase, &terminal.events))
    }

    async fn try_turn_terminal_snapshot(
        &self,
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

        let phase = state.current_phase()?;
        let events = turn_events(self.stored_events(session_id).await?, turn_id);
        if turn_snapshot_is_terminal(phase, &events) {
            return Ok(Some(TurnTerminalSnapshot { phase, events }));
        }

        Ok(None)
    }
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

    let replay_records = records.split_off(index + 1);
    (records, replay_records)
}

fn try_turn_terminal_snapshot_from_recent(
    state: &SessionState,
    turn_id: &str,
) -> Result<Option<TurnTerminalSnapshot>> {
    let phase = state.current_phase()?;
    let events = turn_events(state.snapshot_recent_stored_events()?, turn_id);
    if turn_snapshot_is_terminal(phase, &events) {
        return Ok(Some(TurnTerminalSnapshot { phase, events }));
    }

    Ok(None)
}

fn turn_events(stored_events: Vec<StoredEvent>, turn_id: &str) -> Vec<StoredEvent> {
    stored_events
        .into_iter()
        .filter(|stored| stored.event.turn_id() == Some(turn_id))
        .collect()
}

fn turn_snapshot_is_terminal(phase: Phase, events: &[StoredEvent]) -> bool {
    has_terminal_turn_signal(events) || (!events.is_empty() && matches!(phase, Phase::Interrupted))
}

fn record_targets_turn(record: &SessionEventRecord, turn_id: &str) -> bool {
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
        SessionEventRecord, SessionId, SessionMeta, SessionTurnAcquireResult, StorageEvent,
        StorageEventPayload, StoredEvent, UserMessageOrigin,
    };
    use async_trait::async_trait;
    use tokio::time::{Duration, timeout};

    use super::{split_records_at_cursor, turn_snapshot_is_terminal};
    use crate::{
        state::append_and_broadcast,
        turn::test_support::{StubEventStore, test_runtime},
    };

    #[test]
    fn split_records_at_cursor_keeps_seed_prefix_and_replay_suffix() {
        let records = vec![
            SessionEventRecord {
                event_id: "1.0".to_string(),
                event: astrcode_core::AgentEvent::SessionStarted {
                    session_id: "session-1".to_string(),
                },
            },
            SessionEventRecord {
                event_id: "2.0".to_string(),
                event: astrcode_core::AgentEvent::SessionStarted {
                    session_id: "session-1".to_string(),
                },
            },
            SessionEventRecord {
                event_id: "3.0".to_string(),
                event: astrcode_core::AgentEvent::SessionStarted {
                    session_id: "session-1".to_string(),
                },
            },
        ];

        let (seed, replay) = split_records_at_cursor(records, Some("2.0"));

        assert_eq!(
            seed.iter()
                .map(|record| record.event_id.as_str())
                .collect::<Vec<_>>(),
            vec!["1.0", "2.0"]
        );
        assert_eq!(
            replay
                .iter()
                .map(|record| record.event_id.as_str())
                .collect::<Vec<_>>(),
            vec!["3.0"]
        );
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
            async move {
                runtime
                    .wait_for_turn_terminal_snapshot(&session_id, &turn_id)
                    .await
            }
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

        assert!(turn_snapshot_is_terminal(snapshot.phase, &snapshot.events));
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
            async move {
                runtime
                    .wait_for_turn_terminal_snapshot(&session_id, &turn_id)
                    .await
            }
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
    async fn conversation_stream_replay_reuses_single_history_load_when_cache_is_truncated() {
        let event_store = Arc::new(CountingEventStore::with_events(build_large_history()));
        let runtime = test_runtime(event_store.clone());

        runtime
            .get_session_state(&SessionId::from("1".to_string()))
            .await
            .expect("state should load from durable history");
        event_store.reset_replay_count();

        let replay = runtime
            .conversation_stream_replay("session-1", Some("1.0"))
            .await
            .expect("replay facts should build");

        assert_eq!(
            replay
                .seed_records
                .last()
                .map(|record| record.event_id.as_str()),
            Some("1.0")
        );
        assert_eq!(
            event_store.replay_count(),
            1,
            "truncated cache should trigger only one durable replay for stream recovery"
        );
    }

    fn build_large_history() -> Vec<StoredEvent> {
        let mut events = Vec::with_capacity(16_386);
        events.push(StoredEvent {
            storage_seq: 1,
            event: StorageEvent {
                turn_id: None,
                agent: AgentEventContext::default(),
                payload: StorageEventPayload::SessionStart {
                    session_id: "1".to_string(),
                    timestamp: chrono::Utc::now(),
                    working_dir: ".".to_string(),
                    parent_session_id: None,
                    parent_storage_seq: None,
                },
            },
        });
        for storage_seq in 2..=16_386 {
            events.push(StoredEvent {
                storage_seq,
                event: StorageEvent {
                    turn_id: Some(format!("turn-{storage_seq}")),
                    agent: AgentEventContext::default(),
                    payload: StorageEventPayload::UserMessage {
                        content: format!("message {storage_seq}"),
                        origin: UserMessageOrigin::User,
                        timestamp: chrono::Utc::now(),
                    },
                },
            });
        }
        events
    }

    #[derive(Debug, Default)]
    struct CountingEventStore {
        events: Mutex<Vec<StoredEvent>>,
        next_seq: AtomicU64,
        replay_count: AtomicUsize,
    }

    impl CountingEventStore {
        fn with_events(events: Vec<StoredEvent>) -> Self {
            let next_seq = events
                .last()
                .map(|stored| stored.storage_seq)
                .unwrap_or_default();
            Self {
                events: Mutex::new(events),
                next_seq: AtomicU64::new(next_seq),
                replay_count: AtomicUsize::new(0),
            }
        }

        fn replay_count(&self) -> usize {
            self.replay_count.load(Ordering::SeqCst)
        }

        fn reset_replay_count(&self) {
            self.replay_count.store(0, Ordering::SeqCst);
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
            Ok(vec![SessionId::from("1".to_string())])
        }

        async fn list_session_metas(&self) -> Result<Vec<SessionMeta>> {
            Ok(vec![SessionMeta {
                session_id: "1".to_string(),
                working_dir: ".".to_string(),
                display_name: "session-1".to_string(),
                title: "session-1".to_string(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                parent_session_id: None,
                parent_storage_seq: None,
                phase: Phase::Idle,
            }])
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
