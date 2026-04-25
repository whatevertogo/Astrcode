use astrcode_core::{Result, StorageEvent, StorageEventPayload, StoredEvent};

use crate::{EventTranslator, SessionCatalog, state};

#[derive(Debug, Clone)]
pub struct CompactPersistResult {
    pub persisted_events: Vec<StoredEvent>,
    pub compact_applied: bool,
}

impl SessionCatalog {
    /// Persist compact events through the host-session writer/projection path.
    ///
    /// Summary generation remains outside this owner; host-session owns the
    /// durable append, projection update, broadcast, and checkpoint trigger.
    pub async fn persist_compact_events(
        &self,
        session_id: &astrcode_core::SessionId,
        events: Vec<StorageEvent>,
    ) -> Result<CompactPersistResult> {
        let loaded = self.ensure_loaded_session(session_id).await?;
        let phase = loaded.state.current_phase()?;
        let mut translator = EventTranslator::new(phase);
        let mut persisted_events = Vec::with_capacity(events.len());

        for event in events {
            persisted_events
                .push(state::append_and_broadcast(&loaded.state, &event, &mut translator).await?);
        }

        state::checkpoint_if_compacted(
            &self.event_store,
            session_id,
            &loaded.state,
            &persisted_events,
        )
        .await;

        let compact_applied = persisted_events.iter().any(|stored| {
            matches!(
                stored.event.payload,
                StorageEventPayload::CompactApplied { .. }
            )
        });
        Ok(CompactPersistResult {
            persisted_events,
            compact_applied,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
    };

    use astrcode_core::{
        AgentEventContext, AstrError, CompactAppliedMeta, CompactMode, CompactTrigger,
        DeleteProjectResult, Phase, Result, SessionId, SessionMeta, SessionTurnAcquireResult,
        SessionTurnLease, StorageEvent, StorageEventPayload, StoredEvent,
    };
    use async_trait::async_trait;
    use chrono::Utc;

    use crate::{EventStore, SessionCatalog};

    #[derive(Debug)]
    struct TestLease;

    impl SessionTurnLease for TestLease {}

    #[derive(Default)]
    struct MemoryEventStore {
        sessions: Mutex<HashMap<SessionId, (PathBuf, Vec<StoredEvent>)>>,
    }

    #[async_trait]
    impl EventStore for MemoryEventStore {
        async fn ensure_session(&self, session_id: &SessionId, working_dir: &Path) -> Result<()> {
            self.sessions
                .lock()
                .expect("sessions lock poisoned")
                .entry(session_id.clone())
                .or_insert_with(|| (working_dir.to_path_buf(), Vec::new()));
            Ok(())
        }

        async fn append(
            &self,
            session_id: &SessionId,
            event: &StorageEvent,
        ) -> Result<StoredEvent> {
            let mut sessions = self.sessions.lock().expect("sessions lock poisoned");
            let (_, events) = sessions
                .get_mut(session_id)
                .ok_or_else(|| AstrError::SessionNotFound(session_id.to_string()))?;
            let stored = StoredEvent {
                storage_seq: events.len() as u64 + 1,
                event: event.clone(),
            };
            events.push(stored.clone());
            Ok(stored)
        }

        async fn replay(&self, session_id: &SessionId) -> Result<Vec<StoredEvent>> {
            Ok(self
                .sessions
                .lock()
                .expect("sessions lock poisoned")
                .get(session_id)
                .map(|(_, events)| events.clone())
                .unwrap_or_default())
        }

        async fn try_acquire_turn(
            &self,
            _session_id: &SessionId,
            _turn_id: &str,
        ) -> Result<SessionTurnAcquireResult> {
            Ok(SessionTurnAcquireResult::Acquired(Box::new(TestLease)))
        }

        async fn list_sessions(&self) -> Result<Vec<SessionId>> {
            Ok(self
                .sessions
                .lock()
                .expect("sessions lock poisoned")
                .keys()
                .cloned()
                .collect())
        }

        async fn list_session_metas(&self) -> Result<Vec<SessionMeta>> {
            let now = Utc::now();
            Ok(self
                .sessions
                .lock()
                .expect("sessions lock poisoned")
                .iter()
                .map(|(session_id, (working_dir, _))| SessionMeta {
                    session_id: session_id.to_string(),
                    working_dir: working_dir.display().to_string(),
                    display_name: "project".to_string(),
                    title: "New Session".to_string(),
                    created_at: now,
                    updated_at: now,
                    parent_session_id: None,
                    parent_storage_seq: None,
                    phase: Phase::Idle,
                })
                .collect())
        }

        async fn delete_session(&self, session_id: &SessionId) -> Result<()> {
            self.sessions
                .lock()
                .expect("sessions lock poisoned")
                .remove(session_id);
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

    #[test]
    fn compact_applied_event_shape_remains_durable() {
        let event = StorageEvent {
            turn_id: None,
            agent: AgentEventContext::default(),
            payload: StorageEventPayload::CompactApplied {
                trigger: CompactTrigger::Manual,
                summary: "summary".to_string(),
                meta: CompactAppliedMeta {
                    mode: CompactMode::Full,
                    instructions_present: false,
                    fallback_used: false,
                    retry_count: 0,
                    input_units: 10,
                    output_summary_chars: 7,
                },
                preserved_recent_turns: 1,
                pre_tokens: 100,
                post_tokens_estimate: 20,
                messages_removed: 2,
                tokens_freed: 80,
                timestamp: chrono::Utc::now(),
            },
        };

        assert!(matches!(
            event.payload,
            StorageEventPayload::CompactApplied { summary, .. } if summary == "summary"
        ));
    }

    #[tokio::test]
    async fn persist_compact_events_survives_event_replay() {
        let store = Arc::new(MemoryEventStore::default());
        let catalog = SessionCatalog::new(store.clone());
        let meta = catalog
            .create_session("D:/workspace/project")
            .await
            .expect("session should be created");
        let session_id = SessionId::from(meta.session_id);
        let event = StorageEvent {
            turn_id: None,
            agent: AgentEventContext::default(),
            payload: StorageEventPayload::CompactApplied {
                trigger: CompactTrigger::Manual,
                summary: "condensed project facts".to_string(),
                meta: CompactAppliedMeta {
                    mode: CompactMode::Full,
                    instructions_present: false,
                    fallback_used: false,
                    retry_count: 0,
                    input_units: 2,
                    output_summary_chars: 23,
                },
                preserved_recent_turns: 1,
                pre_tokens: 100,
                post_tokens_estimate: 30,
                messages_removed: 2,
                tokens_freed: 70,
                timestamp: Utc::now(),
            },
        };

        let result = catalog
            .persist_compact_events(&session_id, vec![event])
            .await
            .expect("compact events should persist");
        assert!(result.compact_applied);

        let recovered = SessionCatalog::new(store);
        let replayed = recovered
            .replay_stored_events(&session_id)
            .await
            .expect("session events should replay");

        assert!(replayed.iter().any(|stored| matches!(
            &stored.event.payload,
            StorageEventPayload::CompactApplied { summary, .. }
                if summary == "condensed project facts"
        )));
    }
}
