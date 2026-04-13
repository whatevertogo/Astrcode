use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use astrcode_core::{
    AgentEventContext, AgentId, DeleteProjectResult, EventStore, EventTranslator, LlmMessage,
    Phase, PromptFactsProvider, Result, SessionId, SessionMeta, StorageEvent, StorageEventPayload,
    StoredEvent, event::generate_session_id,
};
use astrcode_kernel::Kernel;
use chrono::Utc;
use dashmap::DashMap;
use thiserror::Error;
use tokio::sync::broadcast;

pub mod actor;
pub mod catalog;
pub mod context;
pub mod context_window;
pub mod factory;
pub mod observe;
pub mod query;
pub mod state;
pub mod turn;

use actor::SessionActor;
pub use catalog::SessionCatalogEvent;
pub use context::ResolvedContextSnapshot;
use observe::SessionObserveSnapshot;
pub use observe::{
    SessionEventFilterSpec, SubRunEventScope, SubRunStatusSnapshot, SubRunStatusSource,
};
pub use query::{SessionHistorySnapshot, SessionReplay, SessionViewSnapshot};
pub use state::{
    SessionSnapshot, SessionState, SessionStateEventSink, SessionTokenBudgetState, SessionWriter,
    append_and_broadcast, append_and_broadcast_from_turn_callback, append_batch_acked,
    append_batch_started, append_mailbox_discarded, append_mailbox_queued,
    complete_session_execution, display_name_from_working_dir, normalize_session_id,
    normalize_working_dir, prepare_session_execution, recent_turn_event_tail,
    should_record_compaction_tail_event,
};
pub use turn::{
    TurnFinishReason, TurnOutcome, TurnRunRequest, TurnRunResult, TurnSummary, run_turn,
};

const ROOT_AGENT_ID: &str = "root-agent";

#[derive(Debug)]
struct LoadedSession {
    actor: Arc<SessionActor>,
}

#[derive(Debug, Error)]
pub enum SessionRuntimeError {
    #[error("session '{0}' already exists")]
    SessionAlreadyExists(String),
    #[error("session '{0}' not found")]
    SessionNotFound(String),
    #[error("session '{session_id}' initialization failed: {message}")]
    SessionInitializationFailed { session_id: String, message: String },
}

impl From<SessionRuntimeError> for astrcode_core::AstrError {
    fn from(value: SessionRuntimeError) -> Self {
        match value {
            SessionRuntimeError::SessionAlreadyExists(session_id) => {
                astrcode_core::AstrError::Validation(format!(
                    "session '{}' already exists",
                    session_id
                ))
            },
            SessionRuntimeError::SessionNotFound(session_id) => {
                astrcode_core::AstrError::SessionNotFound(session_id)
            },
            SessionRuntimeError::SessionInitializationFailed {
                session_id,
                message,
            } => astrcode_core::AstrError::Internal(format!(
                "session '{}' initialization failed: {}",
                session_id, message
            )),
        }
    }
}

/// 单 session 真相面。
pub struct SessionRuntime {
    kernel: Arc<Kernel>,
    prompt_facts_provider: Arc<dyn PromptFactsProvider>,
    sessions: DashMap<SessionId, Arc<LoadedSession>>,
    event_store: Arc<dyn EventStore>,
    catalog_events: broadcast::Sender<SessionCatalogEvent>,
}

impl SessionRuntime {
    pub fn new(
        kernel: Arc<Kernel>,
        prompt_facts_provider: Arc<dyn PromptFactsProvider>,
        event_store: Arc<dyn EventStore>,
    ) -> Self {
        let (catalog_events, _) = broadcast::channel(256);
        Self {
            kernel,
            prompt_facts_provider,
            sessions: DashMap::new(),
            event_store,
            catalog_events,
        }
    }

    pub fn subscribe_catalog_events(&self) -> broadcast::Receiver<SessionCatalogEvent> {
        self.catalog_events.subscribe()
    }

    /// 返回当前已加载到内存中的 session ID。
    ///
    /// Why: 治理视图关心的是 live runtime 负载，而不是磁盘上全部 durable session。
    pub fn list_sessions(&self) -> Vec<SessionId> {
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
                meta.phase = entry.actor.state().current_phase()?;
            }
        }
        metas.sort_by_key(|meta| meta.updated_at);
        Ok(metas)
    }

    pub async fn create_session(&self, working_dir: impl Into<String>) -> Result<SessionMeta> {
        let working_dir = normalize_working_dir(PathBuf::from(working_dir.into()))?;
        let session_id_raw = generate_session_id();
        let session_id: SessionId = session_id_raw.clone().into();
        if self.sessions.contains_key(&session_id) {
            return Err(SessionRuntimeError::SessionAlreadyExists(session_id_raw).into());
        }

        self.event_store
            .ensure_session(&session_id, &working_dir)
            .await?;

        let created_at = Utc::now();
        let actor = Arc::new(
            SessionActor::new_persistent(
                session_id.clone(),
                working_dir.display().to_string(),
                AgentId::from(ROOT_AGENT_ID.to_string()),
                Arc::clone(&self.event_store),
            )
            .map_err(|error| SessionRuntimeError::SessionInitializationFailed {
                session_id: session_id.to_string(),
                message: error.to_string(),
            })?,
        );
        self.sessions.insert(
            session_id.clone(),
            Arc::new(LoadedSession {
                actor: Arc::clone(&actor),
            }),
        );

        let meta = SessionMeta {
            session_id: session_id.to_string(),
            working_dir: actor.working_dir().to_string(),
            display_name: display_name_from_working_dir(Path::new(actor.working_dir())),
            title: "New Session".to_string(),
            created_at,
            updated_at: created_at,
            parent_session_id: None,
            parent_storage_seq: None,
            phase: Phase::Idle,
        };
        let _ = self
            .catalog_events
            .send(SessionCatalogEvent::SessionCreated {
                session_id: session_id.to_string(),
            });
        Ok(meta)
    }

    pub async fn observe(&self, session_id: &SessionId) -> Result<SessionObserveSnapshot> {
        let actor = self.ensure_loaded_session(session_id).await?;
        Ok(SessionObserveSnapshot {
            state: actor.snapshot(),
        })
    }

    /// 按需加载 session 并返回内部状态引用。
    ///
    /// 用于 agent 编排层需要直接操作 SessionState 的场景
    /// （如 mailbox 追加、对话投影读取等）。
    pub async fn get_session_state(&self, session_id: &SessionId) -> Result<Arc<SessionState>> {
        let actor = self.ensure_loaded_session(session_id).await?;
        Ok(Arc::clone(actor.state()))
    }

    /// 回放指定 session 的全部持久化事件。
    ///
    /// 用于 agent 编排层需要从 durable 事件中提取 mailbox 信封等场景。
    pub async fn replay_stored_events(&self, session_id: &SessionId) -> Result<Vec<StoredEvent>> {
        self.ensure_session_exists(session_id).await?;
        self.event_store.replay(session_id).await
    }

    pub async fn delete_session(&self, session_id: &str) -> Result<()> {
        let session_id = SessionId::from(normalize_session_id(session_id));
        self.ensure_session_exists(&session_id).await?;
        self.event_store.delete_session(&session_id).await?;
        self.sessions.remove(&session_id);
        let _ = self
            .catalog_events
            .send(SessionCatalogEvent::SessionDeleted {
                session_id: session_id.to_string(),
            });
        Ok(())
    }

    pub async fn delete_project(&self, working_dir: &str) -> Result<DeleteProjectResult> {
        let deleted = self
            .event_store
            .delete_sessions_by_working_dir(working_dir)
            .await?;

        let target = normalize_path(working_dir);
        let to_remove = self
            .sessions
            .iter()
            .filter_map(|entry| {
                (normalize_path(entry.value().actor.working_dir()) == target)
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

    pub async fn compact_session(&self, session_id: &str) -> Result<()> {
        let session_id = SessionId::from(normalize_session_id(session_id));
        let actor = self.ensure_loaded_session(&session_id).await?;
        let projected = actor.state().snapshot_projected_state()?;
        let summary = projected
            .messages
            .iter()
            .rev()
            .find_map(|message| match message {
                LlmMessage::Assistant { content, .. } if !content.trim().is_empty() => {
                    Some(content.clone())
                },
                LlmMessage::User { content, .. } if !content.trim().is_empty() => {
                    Some(content.clone())
                },
                _ => None,
            })
            .unwrap_or_else(|| "compacted".to_string());

        let mut translator = EventTranslator::new(actor.state().current_phase()?);
        let event = StorageEvent {
            turn_id: None,
            agent: AgentEventContext::default(),
            payload: StorageEventPayload::CompactApplied {
                trigger: astrcode_core::CompactTrigger::Manual,
                summary,
                preserved_recent_turns: 1,
                pre_tokens: 0,
                post_tokens_estimate: 0,
                messages_removed: 0,
                tokens_freed: 0,
                timestamp: Utc::now(),
            },
        };
        append_and_broadcast(actor.state(), &event, &mut translator).await?;
        Ok(())
    }

    async fn session_phase(&self, session_id: &SessionId) -> Result<Phase> {
        if let Some(entry) = self.sessions.get(session_id) {
            return entry.actor.state().current_phase();
        }
        let meta = self
            .event_store
            .list_session_metas()
            .await?
            .into_iter()
            .find(|meta| normalize_session_id(&meta.session_id) == session_id.as_str())
            .ok_or_else(|| SessionRuntimeError::SessionNotFound(session_id.to_string()))?;
        Ok(meta.phase)
    }

    async fn ensure_loaded_session(&self, session_id: &SessionId) -> Result<Arc<SessionActor>> {
        if let Some(entry) = self.sessions.get(session_id) {
            return Ok(Arc::clone(&entry.actor));
        }
        let meta = self
            .event_store
            .list_session_metas()
            .await?
            .into_iter()
            .find(|meta| normalize_session_id(&meta.session_id) == session_id.as_str())
            .ok_or_else(|| SessionRuntimeError::SessionNotFound(session_id.to_string()))?;
        let stored = self.event_store.replay(session_id).await?;
        let actor = Arc::new(SessionActor::from_replay(
            session_id.clone(),
            meta.working_dir,
            AgentId::from(ROOT_AGENT_ID.to_string()),
            Arc::clone(&self.event_store),
            stored,
        )?);
        let loaded = Arc::new(LoadedSession {
            actor: Arc::clone(&actor),
        });
        match self.sessions.entry(session_id.clone()) {
            dashmap::mapref::entry::Entry::Occupied(entry) => Ok(Arc::clone(&entry.get().actor)),
            dashmap::mapref::entry::Entry::Vacant(entry) => {
                entry.insert(loaded);
                Ok(actor)
            },
        }
    }

    async fn ensure_session_exists(&self, session_id: &SessionId) -> Result<()> {
        if self.sessions.contains_key(session_id) {
            return Ok(());
        }
        let exists = self
            .event_store
            .list_session_metas()
            .await?
            .into_iter()
            .any(|meta| normalize_session_id(&meta.session_id) == session_id.as_str());
        if exists {
            Ok(())
        } else {
            Err(SessionRuntimeError::SessionNotFound(session_id.to_string()).into())
        }
    }
}

fn normalize_path(value: &str) -> String {
    value.replace('\\', "/").trim_end_matches('/').to_string()
}
