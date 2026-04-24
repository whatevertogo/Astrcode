use std::sync::{Arc, Mutex as StdMutex};

use astrcode_core::{
    AgentEvent, ChildSessionNode, EventTranslator, LlmMessage, ModeId, Phase, Result,
    SessionEventRecord, StoredEvent, TaskSnapshot, normalize_recovered_phase,
    support::{self},
};
use chrono::Utc;
use tokio::sync::broadcast;

use crate::{
    AgentState, AgentStateProjector, EventStore, InputQueueProjection, SessionRecoveryCheckpoint,
    TurnProjectionSnapshot, event_log::SessionWriter, projection_registry::ProjectionRegistry,
};

pub const SESSION_BROADCAST_CAPACITY: usize = 2048;
pub const SESSION_LIVE_BROADCAST_CAPACITY: usize = 2048;

pub struct SessionState {
    pub(crate) projection_registry: StdMutex<ProjectionRegistry>,
    pub broadcaster: broadcast::Sender<SessionEventRecord>,
    live_broadcaster: broadcast::Sender<AgentEvent>,
    pub writer: Arc<SessionWriter>,
}

impl std::fmt::Debug for SessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionState").finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSnapshot {
    pub session_id: astrcode_core::SessionId,
    pub working_dir: String,
    pub latest_turn_id: Option<astrcode_core::TurnId>,
    pub turn_count: usize,
}

impl SessionState {
    pub fn new(
        phase: Phase,
        writer: Arc<SessionWriter>,
        projector: AgentStateProjector,
        recent_records: Vec<SessionEventRecord>,
        recent_stored: Vec<StoredEvent>,
    ) -> Self {
        Self::from_parts(phase, writer, projector, recent_records, recent_stored)
    }

    pub fn from_recovery(
        writer: Arc<SessionWriter>,
        checkpoint: &SessionRecoveryCheckpoint,
        tail_events: Vec<StoredEvent>,
    ) -> Result<Self> {
        let phase = normalize_recovered_phase(checkpoint.agent_state.phase);
        let mut projection_registry = ProjectionRegistry::from_recovery(
            phase,
            &checkpoint.agent_state,
            checkpoint.projection_registry_snapshot(),
            Vec::new(),
            Vec::new(),
        );
        for stored in &tail_events {
            stored.event.validate().map_err(|error| {
                astrcode_core::AstrError::Validation(format!(
                    "session '{}' contains invalid stored event at storage_seq {}: {}",
                    checkpoint.agent_state.session_id, stored.storage_seq, error
                ))
            })?;
            projection_registry.apply(stored)?;
        }
        projection_registry.cache_records(&astrcode_core::replay_records(&tail_events, None));
        let (broadcaster, _) = broadcast::channel(SESSION_BROADCAST_CAPACITY);
        let (live_broadcaster, _) = broadcast::channel(SESSION_LIVE_BROADCAST_CAPACITY);

        Ok(Self {
            projection_registry: StdMutex::new(projection_registry),
            broadcaster,
            live_broadcaster,
            writer,
        })
    }

    fn from_parts(
        phase: Phase,
        writer: Arc<SessionWriter>,
        projector: AgentStateProjector,
        recent_records: Vec<SessionEventRecord>,
        recent_stored: Vec<StoredEvent>,
    ) -> Self {
        let (broadcaster, _) = broadcast::channel(SESSION_BROADCAST_CAPACITY);
        let (live_broadcaster, _) = broadcast::channel(SESSION_LIVE_BROADCAST_CAPACITY);
        Self {
            projection_registry: StdMutex::new(ProjectionRegistry::new(
                phase,
                projector,
                recent_records,
                recent_stored,
            )),
            broadcaster,
            live_broadcaster,
            writer,
        }
    }

    pub fn recovery_checkpoint(
        &self,
        checkpoint_storage_seq: u64,
    ) -> Result<SessionRecoveryCheckpoint> {
        let projection_registry =
            support::lock_anyhow(&self.projection_registry, "session projection registry")?;
        Ok(SessionRecoveryCheckpoint::new(
            projection_registry.snapshot_projected_state(),
            projection_registry.projection_snapshot(),
            checkpoint_storage_seq,
        ))
    }

    pub fn snapshot_projected_state(&self) -> Result<AgentState> {
        Ok(
            support::lock_anyhow(&self.projection_registry, "session projection registry")?
                .snapshot_projected_state(),
        )
    }

    pub fn current_turn_messages(&self) -> Result<Vec<LlmMessage>> {
        Ok(self.snapshot_projected_state()?.messages)
    }

    pub fn subscribe_live(&self) -> broadcast::Receiver<AgentEvent> {
        self.live_broadcaster.subscribe()
    }

    pub fn broadcast_live_event(&self, event: AgentEvent) {
        let _ = self.live_broadcaster.send(event);
    }

    pub fn current_phase(&self) -> Result<Phase> {
        Ok(
            support::lock_anyhow(&self.projection_registry, "session projection registry")?
                .current_phase(),
        )
    }

    pub fn current_mode_id(&self) -> Result<ModeId> {
        Ok(
            support::lock_anyhow(&self.projection_registry, "session projection registry")?
                .current_mode_id(),
        )
    }

    pub fn last_mode_changed_at(&self) -> Result<Option<chrono::DateTime<Utc>>> {
        Ok(
            support::lock_anyhow(&self.projection_registry, "session projection registry")?
                .last_mode_changed_at(),
        )
    }

    pub fn translate_store_and_cache(
        &self,
        stored: &StoredEvent,
        translator: &mut EventTranslator,
    ) -> Result<Vec<SessionEventRecord>> {
        stored.event.validate()?;
        let mut projection_registry =
            support::lock_anyhow(&self.projection_registry, "session projection registry")?;
        projection_registry.apply(stored)?;
        let records = translator.translate(stored);
        projection_registry.cache_records(&records);
        Ok(records)
    }

    pub fn recent_records_after(
        &self,
        last_event_id: Option<&str>,
    ) -> Result<Option<Vec<SessionEventRecord>>> {
        Ok(
            support::lock_anyhow(&self.projection_registry, "session projection registry")?
                .recent_records_after(last_event_id),
        )
    }

    pub fn snapshot_recent_stored_events(&self) -> Result<Vec<StoredEvent>> {
        Ok(
            support::lock_anyhow(&self.projection_registry, "session projection registry")?
                .snapshot_recent_stored_events(),
        )
    }

    pub fn turn_projection(&self, turn_id: &str) -> Result<Option<TurnProjectionSnapshot>> {
        Ok(
            support::lock_anyhow(&self.projection_registry, "session projection registry")?
                .turn_projection(turn_id),
        )
    }

    pub fn upsert_child_session_node(&self, node: ChildSessionNode) -> Result<()> {
        support::lock_anyhow(&self.projection_registry, "session projection registry")?
            .children
            .upsert(node);
        Ok(())
    }

    pub fn child_session_node(&self, sub_run_id: &str) -> Result<Option<ChildSessionNode>> {
        Ok(
            support::lock_anyhow(&self.projection_registry, "session projection registry")?
                .child_session_node(sub_run_id),
        )
    }

    pub fn list_child_session_nodes(&self) -> Result<Vec<ChildSessionNode>> {
        Ok(
            support::lock_anyhow(&self.projection_registry, "session projection registry")?
                .list_child_session_nodes(),
        )
    }

    pub fn child_nodes_for_parent(&self, parent_agent_id: &str) -> Result<Vec<ChildSessionNode>> {
        Ok(
            support::lock_anyhow(&self.projection_registry, "session projection registry")?
                .child_nodes_for_parent(parent_agent_id),
        )
    }

    pub fn subtree_nodes(&self, root_agent_id: &str) -> Result<Vec<ChildSessionNode>> {
        Ok(
            support::lock_anyhow(&self.projection_registry, "session projection registry")?
                .subtree_nodes(root_agent_id),
        )
    }

    pub fn active_tasks_for(&self, owner: &str) -> Result<Option<TaskSnapshot>> {
        Ok(
            support::lock_anyhow(&self.projection_registry, "session projection registry")?
                .active_tasks_for(owner),
        )
    }

    pub fn input_queue_projection_for_agent(&self, agent_id: &str) -> Result<InputQueueProjection> {
        Ok(
            support::lock_anyhow(&self.projection_registry, "session projection registry")?
                .input_queue_projection_for_agent(agent_id),
        )
    }

    pub async fn append_and_broadcast(
        &self,
        event: &astrcode_core::StorageEvent,
        translator: &mut EventTranslator,
    ) -> Result<StoredEvent> {
        let stored = self.writer.clone().append(event.clone()).await?;
        let records = self.translate_store_and_cache(&stored, translator)?;
        for record in records {
            let _ = self.broadcaster.send(record);
        }
        Ok(stored)
    }
}

pub async fn append_and_broadcast(
    session: &SessionState,
    event: &astrcode_core::StorageEvent,
    translator: &mut EventTranslator,
) -> Result<StoredEvent> {
    session.append_and_broadcast(event, translator).await
}

pub async fn checkpoint_if_compacted(
    event_store: &Arc<dyn EventStore>,
    session_id: &astrcode_core::SessionId,
    session_state: &Arc<SessionState>,
    persisted_events: &[StoredEvent],
) {
    let Some(checkpoint_storage_seq) = persisted_events.last().map(|stored| stored.storage_seq)
    else {
        return;
    };
    if !persisted_events.iter().any(|stored| {
        matches!(
            stored.event.payload,
            astrcode_core::StorageEventPayload::CompactApplied { .. }
        )
    }) {
        return;
    }
    let checkpoint = match session_state.recovery_checkpoint(checkpoint_storage_seq) {
        Ok(checkpoint) => checkpoint,
        Err(error) => {
            log::warn!(
                "failed to build recovery checkpoint for session '{}': {}",
                session_id,
                error
            );
            return;
        },
    };
    if let Err(error) = event_store
        .checkpoint_session(session_id, &checkpoint)
        .await
    {
        log::warn!(
            "failed to persist recovery checkpoint for session '{}': {}",
            session_id,
            error
        );
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use astrcode_core::{
        AgentEventContext, AgentLifecycleStatus, ChildSessionLineageKind, ChildSessionNotification,
        ChildSessionNotificationKind, ChildSessionStatusSource, EventLogWriter, EventTranslator,
        InputQueuedPayload, Phase, QueuedInputEnvelope, StorageEvent, StorageEventPayload,
        StoreResult, StoredEvent, SubRunStorageMode, UserMessageOrigin,
    };

    use super::{SessionState, SessionWriter};
    use crate::{AgentStateProjector, SubRunHandle};

    #[derive(Default)]
    struct NoopEventLogWriter {
        next_seq: u64,
    }

    impl EventLogWriter for NoopEventLogWriter {
        fn append(&mut self, event: &StorageEvent) -> StoreResult<StoredEvent> {
            self.next_seq += 1;
            Ok(StoredEvent {
                storage_seq: self.next_seq,
                event: event.clone(),
            })
        }
    }

    fn session_state() -> SessionState {
        SessionState::new(
            Phase::Idle,
            Arc::new(SessionWriter::new(Box::new(NoopEventLogWriter::default()))),
            AgentStateProjector::default(),
            Vec::new(),
            Vec::new(),
        )
    }

    fn user_message(content: &str) -> StorageEvent {
        StorageEvent {
            turn_id: Some("turn-1".to_string()),
            agent: AgentEventContext::default(),
            payload: StorageEventPayload::UserMessage {
                content: content.to_string(),
                origin: UserMessageOrigin::User,
                timestamp: chrono::Utc::now(),
            },
        }
    }

    #[tokio::test]
    async fn append_and_broadcast_persists_projects_caches_and_broadcasts() {
        let session = session_state();
        let mut receiver = session.broadcaster.subscribe();
        let mut translator = EventTranslator::new(Phase::Idle);

        let stored = session
            .append_and_broadcast(&user_message("hello"), &mut translator)
            .await
            .expect("event should append");

        assert_eq!(stored.storage_seq, 1);
        assert_eq!(session.snapshot_recent_stored_events().unwrap().len(), 1);
        assert!(
            !session
                .recent_records_after(None)
                .unwrap()
                .unwrap()
                .is_empty()
        );
        assert_eq!(session.current_turn_messages().unwrap().len(), 1);
        assert!(receiver.try_recv().is_ok());
    }

    #[test]
    fn from_recovery_replays_tail_events_into_projection() {
        let checkpoint = crate::SessionRecoveryCheckpoint::new(
            crate::AgentState::default(),
            crate::ProjectionRegistrySnapshot::default(),
            0,
        );
        let stored = StoredEvent {
            storage_seq: 1,
            event: user_message("replayed"),
        };

        let recovered = SessionState::from_recovery(
            Arc::new(SessionWriter::new(Box::new(NoopEventLogWriter::default()))),
            &checkpoint,
            vec![stored],
        )
        .expect("recovery should replay tail events");

        assert_eq!(recovered.current_turn_messages().unwrap().len(), 1);
        assert_eq!(recovered.snapshot_recent_stored_events().unwrap().len(), 1);
    }

    #[test]
    fn from_recovery_restores_child_session_nodes_and_input_queue_projection() {
        let checkpoint = crate::SessionRecoveryCheckpoint::new(
            crate::AgentState::default(),
            crate::ProjectionRegistrySnapshot::default(),
            0,
        );
        let handle = SubRunHandle {
            sub_run_id: "subrun-1".into(),
            agent_id: "agent-child".into(),
            session_id: "session-parent".into(),
            child_session_id: Some("session-child".into()),
            depth: 1,
            parent_turn_id: "turn-parent".into(),
            parent_agent_id: Some("agent-parent".into()),
            parent_sub_run_id: None,
            lineage_kind: ChildSessionLineageKind::Spawn,
            agent_profile: "coding".to_string(),
            storage_mode: SubRunStorageMode::IndependentSession,
            lifecycle: AgentLifecycleStatus::Running,
            last_turn_outcome: None,
            resolved_limits: Default::default(),
            delegation: None,
        };
        let notification = ChildSessionNotification {
            notification_id: "delivery-child".into(),
            child_ref: handle.child_ref_with_status(AgentLifecycleStatus::Running),
            kind: ChildSessionNotificationKind::Started,
            source_tool_call_id: Some("call-1".into()),
            delivery: None,
        };
        let recovered = SessionState::from_recovery(
            Arc::new(SessionWriter::new(Box::new(NoopEventLogWriter::default()))),
            &checkpoint,
            vec![
                StoredEvent {
                    storage_seq: 1,
                    event: StorageEvent {
                        turn_id: Some("turn-parent".into()),
                        agent: AgentEventContext::default(),
                        payload: StorageEventPayload::ChildSessionNotification {
                            notification,
                            timestamp: Some(chrono::Utc::now()),
                        },
                    },
                },
                StoredEvent {
                    storage_seq: 2,
                    event: StorageEvent {
                        turn_id: Some("turn-parent".into()),
                        agent: AgentEventContext::default(),
                        payload: StorageEventPayload::AgentInputQueued {
                            payload: InputQueuedPayload {
                                envelope: QueuedInputEnvelope {
                                    delivery_id: "delivery-1".into(),
                                    from_agent_id: "agent-parent".into(),
                                    to_agent_id: "agent-child".into(),
                                    message: "resume work".into(),
                                    queued_at: chrono::Utc::now(),
                                    sender_lifecycle_status: AgentLifecycleStatus::Running,
                                    sender_last_turn_outcome: None,
                                    sender_open_session_id: "session-parent".into(),
                                },
                            },
                        },
                    },
                },
            ],
        )
        .expect("recovery should rebuild collaboration projections");

        let node = recovered
            .child_session_node("subrun-1")
            .expect("projection access should succeed")
            .expect("child session node should be restored");
        assert_eq!(node.child_session_id.as_str(), "session-child");
        assert_eq!(node.status_source, ChildSessionStatusSource::Durable);

        let queue = recovered
            .input_queue_projection_for_agent("agent-child")
            .expect("input queue projection should rebuild");
        assert_eq!(queue.pending_delivery_ids, vec!["delivery-1".into()]);
    }
}
