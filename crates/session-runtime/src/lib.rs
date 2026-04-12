use std::sync::Arc;

use astrcode_core::{AgentId, EventStore, SessionId};
use astrcode_kernel::Kernel;
use dashmap::DashMap;
use thiserror::Error;
use tokio::sync::broadcast;

pub mod actor;
pub mod catalog;
pub mod context;
pub mod factory;
pub mod observe;
pub mod state;
pub mod turn;

use actor::SessionActor;
pub use catalog::SessionCatalogEvent;
pub use context::ResolvedContextSnapshot;
pub use factory::LoopRuntimeDeps;
use observe::SessionObserveSnapshot;
pub use observe::{
    SessionEventFilterSpec, SubRunEventScope, SubRunStatusSnapshot, SubRunStatusSource,
};
pub use state::{
    SessionSnapshot, SessionState, SessionStateEventSink, SessionTokenBudgetState, SessionWriter,
    append_and_broadcast, append_and_broadcast_from_turn_callback, append_batch_acked,
    append_batch_started, append_mailbox_discarded, append_mailbox_queued,
    complete_session_execution, display_name_from_working_dir, normalize_session_id,
    normalize_working_dir, prepare_session_execution, recent_turn_event_tail,
    should_record_compaction_tail_event,
};
pub use turn::{TurnOutcome, TurnRunRequest};

#[derive(Debug, Error)]
pub enum SessionRuntimeError {
    #[error("session '{0}' already exists")]
    SessionAlreadyExists(String),
    #[error("session '{0}' not found")]
    SessionNotFound(String),
}

/// 单 session 真相面。
pub struct SessionRuntime {
    sessions: DashMap<SessionId, Arc<SessionActor>>,
    event_store: Arc<dyn EventStore>,
    kernel: Arc<Kernel>,
    catalog_events: broadcast::Sender<SessionCatalogEvent>,
}

impl SessionRuntime {
    pub fn new(kernel: Arc<Kernel>, event_store: Arc<dyn EventStore>) -> Self {
        let (catalog_events, _) = broadcast::channel(256);
        Self {
            sessions: DashMap::new(),
            event_store,
            kernel,
            catalog_events,
        }
    }

    pub fn subscribe_catalog_events(&self) -> broadcast::Receiver<SessionCatalogEvent> {
        self.catalog_events.subscribe()
    }

    pub fn kernel(&self) -> &Arc<Kernel> {
        &self.kernel
    }

    pub fn event_store(&self) -> &Arc<dyn EventStore> {
        &self.event_store
    }

    pub fn create_session(
        &self,
        session_id: SessionId,
        working_dir: impl Into<String>,
        root_agent_id: AgentId,
    ) -> Result<Arc<SessionActor>, SessionRuntimeError> {
        if self.sessions.contains_key(&session_id) {
            return Err(SessionRuntimeError::SessionAlreadyExists(
                session_id.to_string(),
            ));
        }
        let actor = Arc::new(SessionActor::new_idle(
            session_id.clone(),
            working_dir,
            root_agent_id,
        ));
        self.sessions.insert(session_id.clone(), Arc::clone(&actor));
        let _ = self
            .catalog_events
            .send(SessionCatalogEvent::SessionCreated {
                session_id: session_id.to_string(),
            });
        Ok(actor)
    }

    pub fn session(
        &self,
        session_id: &SessionId,
    ) -> Result<Arc<SessionActor>, SessionRuntimeError> {
        self.sessions
            .get(session_id)
            .map(|entry| Arc::clone(entry.value()))
            .ok_or_else(|| SessionRuntimeError::SessionNotFound(session_id.to_string()))
    }

    pub fn observe(
        &self,
        session_id: &SessionId,
    ) -> Result<SessionObserveSnapshot, SessionRuntimeError> {
        let actor = self.session(session_id)?;
        Ok(SessionObserveSnapshot {
            state: actor.snapshot(),
        })
    }

    pub fn list_sessions(&self) -> Vec<SessionId> {
        let mut sessions = self
            .sessions
            .iter()
            .map(|entry| entry.key().clone())
            .collect::<Vec<_>>();
        sessions.sort();
        sessions
    }

    pub fn remove_session(&self, session_id: &SessionId) -> Result<(), SessionRuntimeError> {
        if self.sessions.remove(session_id).is_none() {
            return Err(SessionRuntimeError::SessionNotFound(session_id.to_string()));
        }
        let _ = self
            .catalog_events
            .send(SessionCatalogEvent::SessionDeleted {
                session_id: session_id.to_string(),
            });
        Ok(())
    }
}
