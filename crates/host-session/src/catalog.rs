use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use astrcode_core::{
    AstrError, EventTranslator, ModeId, Phase, Result, SessionId, SessionMeta,
    SessionTurnAcquireResult, StorageEvent, StorageEventPayload, StoredEvent,
    event::generate_session_id,
};
use chrono::Utc;
use dashmap::DashMap;
use tokio::sync::broadcast;

use crate::{
    AgentStateProjector, EventStore, SessionCatalogEvent, SessionState, SessionWriter,
    state::{SESSION_BROADCAST_CAPACITY, append_and_broadcast},
    turn_mutation::TurnMutationState,
};

#[derive(Debug)]
pub struct LoadedSession {
    pub session_id: SessionId,
    pub working_dir: PathBuf,
    pub state: Arc<SessionState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionModeState {
    pub current_mode_id: ModeId,
    pub last_mode_changed_at: Option<chrono::DateTime<Utc>>,
}

/// Host-session owned catalog and loaded-session registry.
///
/// This is the durable-session owner surface that replaces the old
/// `session-runtime` DashMap/catalog responsibilities. It deliberately owns
/// only event-log recovery, loaded `SessionState`, and catalog broadcasts.
pub struct SessionCatalog {
    pub(crate) event_store: Arc<dyn EventStore>,
    sessions: DashMap<SessionId, Arc<LoadedSession>>,
    pub(crate) turn_mutations: DashMap<SessionId, Arc<TurnMutationState>>,
    pub(crate) catalog_events: broadcast::Sender<SessionCatalogEvent>,
}

impl SessionCatalog {
    pub fn new(event_store: Arc<dyn EventStore>) -> Self {
        let (catalog_events, _) = broadcast::channel(SESSION_BROADCAST_CAPACITY);
        Self {
            event_store,
            sessions: DashMap::new(),
            turn_mutations: DashMap::new(),
            catalog_events,
        }
    }

    pub fn subscribe_catalog_events(&self) -> broadcast::Receiver<SessionCatalogEvent> {
        self.catalog_events.subscribe()
    }

    pub fn list_loaded_sessions(&self) -> Vec<SessionId> {
        let mut sessions = self
            .sessions
            .iter()
            .map(|entry| entry.key().clone())
            .collect::<Vec<_>>();
        sessions.sort();
        sessions
    }

    pub async fn list_session_metas(&self) -> Result<Vec<SessionMeta>> {
        let mut metas = self.event_store.list_session_metas().await?;
        for meta in &mut metas {
            let session_id: SessionId = meta.session_id.clone().into();
            if let Some(entry) = self.sessions.get(&session_id) {
                meta.phase = entry.state.current_phase()?;
            }
        }
        metas.sort_by_key(|meta| meta.updated_at);
        Ok(metas)
    }

    pub async fn create_session(&self, working_dir: impl Into<String>) -> Result<SessionMeta> {
        self.create_session_with_parent(working_dir, None, None)
            .await
    }

    pub async fn create_child_session(
        &self,
        working_dir: impl Into<String>,
        parent_session_id: impl Into<String>,
        parent_storage_seq: Option<u64>,
    ) -> Result<SessionMeta> {
        self.create_session_with_parent(
            working_dir,
            Some(parent_session_id.into()),
            parent_storage_seq,
        )
        .await
    }

    async fn create_session_with_parent(
        &self,
        working_dir: impl Into<String>,
        parent_session_id: Option<String>,
        parent_storage_seq: Option<u64>,
    ) -> Result<SessionMeta> {
        let working_dir = normalize_working_dir(PathBuf::from(working_dir.into()))?;
        let session_id_raw = generate_session_id();
        let session_id: SessionId = session_id_raw.clone().into();
        if self.sessions.contains_key(&session_id) {
            return Err(AstrError::Validation(format!(
                "session '{}' already exists",
                session_id
            )));
        }

        self.event_store
            .ensure_session(&session_id, &working_dir)
            .await?;

        let writer = Arc::new(SessionWriter::from_event_store(
            Arc::clone(&self.event_store),
            session_id.clone(),
        ));
        let state = Arc::new(SessionState::new(
            Phase::Idle,
            writer,
            AgentStateProjector::default(),
            Vec::new(),
            Vec::new(),
        ));
        let created_at = Utc::now();
        let start = StorageEvent {
            turn_id: None,
            agent: astrcode_core::AgentEventContext::default(),
            payload: StorageEventPayload::SessionStart {
                session_id: session_id.to_string(),
                timestamp: created_at,
                working_dir: working_dir.display().to_string(),
                parent_session_id: parent_session_id.clone(),
                parent_storage_seq,
            },
        };
        let mut translator = EventTranslator::new(Phase::Idle);
        append_and_broadcast(&state, &start, &mut translator).await?;

        let loaded = Arc::new(LoadedSession {
            session_id: session_id.clone(),
            working_dir: working_dir.clone(),
            state: Arc::clone(&state),
        });
        self.sessions.insert(session_id.clone(), loaded);

        let meta = SessionMeta {
            session_id: session_id.to_string(),
            working_dir: working_dir.display().to_string(),
            display_name: display_name_from_working_dir(&working_dir),
            title: "New Session".to_string(),
            created_at,
            updated_at: created_at,
            parent_session_id,
            parent_storage_seq,
            phase: Phase::Idle,
        };
        let _ = self
            .catalog_events
            .send(SessionCatalogEvent::SessionCreated {
                session_id: session_id.to_string(),
            });
        Ok(meta)
    }

    pub async fn ensure_loaded_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Arc<LoadedSession>> {
        if let Some(entry) = self.sessions.get(session_id) {
            return Ok(Arc::clone(entry.value()));
        }

        let meta = self.find_meta(session_id).await?;
        let recovered = self.event_store.recover_session(session_id).await?;
        let writer = Arc::new(SessionWriter::from_event_store(
            Arc::clone(&self.event_store),
            session_id.clone(),
        ));
        let state = Arc::new(match recovered.checkpoint {
            Some(checkpoint) => {
                SessionState::from_recovery(writer, &checkpoint, recovered.tail_events)?
            },
            None => {
                let events = recovered
                    .tail_events
                    .iter()
                    .map(|stored| stored.event.clone())
                    .collect::<Vec<_>>();
                SessionState::new(
                    normalize_recovered_phase_from_events(&recovered.tail_events),
                    writer,
                    AgentStateProjector::from_events(&events),
                    astrcode_core::replay_records(&recovered.tail_events, None),
                    recovered.tail_events,
                )
            },
        });

        let loaded = Arc::new(LoadedSession {
            session_id: session_id.clone(),
            working_dir: PathBuf::from(meta.working_dir),
            state,
        });
        match self.sessions.entry(session_id.clone()) {
            dashmap::mapref::entry::Entry::Occupied(entry) => Ok(Arc::clone(entry.get())),
            dashmap::mapref::entry::Entry::Vacant(entry) => {
                entry.insert(Arc::clone(&loaded));
                Ok(loaded)
            },
        }
    }

    pub async fn ensure_session_exists(&self, session_id: &SessionId) -> Result<()> {
        if self.sessions.contains_key(session_id) {
            return Ok(());
        }
        self.find_meta(session_id).await.map(|_| ())
    }

    pub async fn try_acquire_turn(
        &self,
        session_id: &SessionId,
        turn_id: &str,
    ) -> Result<SessionTurnAcquireResult> {
        self.ensure_session_exists(session_id).await?;
        self.event_store.try_acquire_turn(session_id, turn_id).await
    }

    pub async fn replay_stored_events(&self, session_id: &SessionId) -> Result<Vec<StoredEvent>> {
        self.ensure_session_exists(session_id).await?;
        self.event_store.replay(session_id).await
    }

    pub async fn session_mode_state(&self, session_id: &SessionId) -> Result<SessionModeState> {
        let loaded = self.ensure_loaded_session(session_id).await?;
        Ok(SessionModeState {
            current_mode_id: loaded.state.current_mode_id()?,
            last_mode_changed_at: loaded.state.last_mode_changed_at()?,
        })
    }

    pub async fn switch_mode(
        &self,
        session_id: &SessionId,
        target_mode_id: ModeId,
    ) -> Result<SessionModeState> {
        let loaded = self.ensure_loaded_session(session_id).await?;
        let current_mode_id = loaded.state.current_mode_id()?;
        if current_mode_id == target_mode_id {
            return Ok(SessionModeState {
                current_mode_id,
                last_mode_changed_at: loaded.state.last_mode_changed_at()?,
            });
        }

        let mut translator = EventTranslator::new(loaded.state.current_phase()?);
        append_and_broadcast(
            &loaded.state,
            &StorageEvent {
                turn_id: None,
                agent: astrcode_core::AgentEventContext::default(),
                payload: StorageEventPayload::ModeChanged {
                    from: current_mode_id,
                    to: target_mode_id,
                    timestamp: Utc::now(),
                },
            },
            &mut translator,
        )
        .await?;

        Ok(SessionModeState {
            current_mode_id: loaded.state.current_mode_id()?,
            last_mode_changed_at: loaded.state.last_mode_changed_at()?,
        })
    }

    pub async fn delete_session(&self, session_id: &SessionId) -> Result<()> {
        self.ensure_session_exists(session_id).await?;
        self.event_store.delete_session(session_id).await?;
        self.sessions.remove(session_id);
        let _ = self
            .catalog_events
            .send(SessionCatalogEvent::SessionDeleted {
                session_id: session_id.to_string(),
            });
        Ok(())
    }

    pub async fn delete_project(
        &self,
        working_dir: &str,
    ) -> Result<astrcode_core::DeleteProjectResult> {
        let deleted = self
            .event_store
            .delete_sessions_by_working_dir(working_dir)
            .await?;

        let target = normalize_path(working_dir);
        let to_remove = self
            .sessions
            .iter()
            .filter_map(|entry| {
                (normalize_path(&entry.working_dir.display().to_string()) == target)
                    .then_some(entry.key().clone())
            })
            .collect::<Vec<_>>();
        for session_id in to_remove {
            self.sessions.remove(&session_id);
        }

        let _ = self
            .catalog_events
            .send(SessionCatalogEvent::ProjectDeleted {
                working_dir: working_dir.to_string(),
            });
        Ok(deleted)
    }

    async fn find_meta(&self, session_id: &SessionId) -> Result<SessionMeta> {
        self.event_store
            .list_session_metas()
            .await?
            .into_iter()
            .find(|meta| normalize_session_id(&meta.session_id) == session_id.as_str())
            .ok_or_else(|| AstrError::SessionNotFound(session_id.to_string()))
    }
}

pub fn normalize_session_id(session_id: &str) -> String {
    session_id.trim().to_string()
}

pub fn normalize_working_dir(path: PathBuf) -> Result<PathBuf> {
    if path.as_os_str().is_empty() {
        return std::env::current_dir().map_err(|error| {
            AstrError::Internal(format!("resolve current working directory failed: {error}"))
        });
    }
    Ok(path)
}

pub fn display_name_from_working_dir(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| path.to_str().unwrap_or("session"))
        .to_string()
}

fn normalize_path(value: &str) -> String {
    value.replace('\\', "/").trim_end_matches('/').to_string()
}

fn normalize_recovered_phase_from_events(events: &[StoredEvent]) -> Phase {
    let phase = events
        .last()
        .map(|stored| astrcode_core::phase_of_storage_event(&stored.event))
        .unwrap_or(Phase::Idle);
    astrcode_core::normalize_recovered_phase(phase)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
    };

    use astrcode_core::{
        DeleteProjectResult, ModeId, SessionMeta, SessionTurnAcquireResult, SessionTurnLease,
        StoredEvent,
    };
    use async_trait::async_trait;

    use super::*;

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
                    display_name: display_name_from_working_dir(working_dir),
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
            working_dir: &str,
        ) -> Result<DeleteProjectResult> {
            let target = normalize_path(working_dir);
            let mut sessions = self.sessions.lock().expect("sessions lock poisoned");
            let before = sessions.len();
            sessions.retain(|_, (path, _)| normalize_path(&path.display().to_string()) != target);
            Ok(DeleteProjectResult {
                success_count: before.saturating_sub(sessions.len()),
                failed_session_ids: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn catalog_creates_loads_and_deletes_sessions() {
        let store = Arc::new(MemoryEventStore::default());
        let catalog = SessionCatalog::new(store);
        let mut events = catalog.subscribe_catalog_events();

        let meta = catalog
            .create_session("D:/workspace/project")
            .await
            .expect("session should be created");
        let session_id = SessionId::from(meta.session_id.clone());

        assert_eq!(catalog.list_loaded_sessions(), vec![session_id.clone()]);
        assert!(matches!(
            events.try_recv(),
            Ok(SessionCatalogEvent::SessionCreated { .. })
        ));
        assert_eq!(
            catalog
                .ensure_loaded_session(&session_id)
                .await
                .expect("session should load")
                .state
                .snapshot_recent_stored_events()
                .expect("events should be cached")
                .len(),
            1
        );
        assert!(matches!(
            catalog.try_acquire_turn(&session_id, "turn-1").await,
            Ok(SessionTurnAcquireResult::Acquired(_))
        ));

        catalog
            .delete_session(&session_id)
            .await
            .expect("session should delete");
        assert!(catalog.list_loaded_sessions().is_empty());
    }

    #[tokio::test]
    async fn catalog_switch_mode_updates_projected_mode_state() {
        let store = Arc::new(MemoryEventStore::default());
        let catalog = SessionCatalog::new(store);
        let meta = catalog
            .create_session("D:/workspace/project")
            .await
            .expect("session should be created");
        let session_id = SessionId::from(meta.session_id);

        let initial = catalog
            .session_mode_state(&session_id)
            .await
            .expect("mode state should read");
        assert_eq!(initial.current_mode_id, ModeId::code());
        assert!(initial.last_mode_changed_at.is_none());

        let switched = catalog
            .switch_mode(&session_id, ModeId::plan())
            .await
            .expect("mode should switch");

        assert_eq!(switched.current_mode_id, ModeId::plan());
        assert!(switched.last_mode_changed_at.is_some());
    }
}
