use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use astrcode_core::{
    AgentCollaborationFact, AgentEventContext, AgentId, AgentLifecycleStatus,
    ChildSessionNotification, DeleteProjectResult, EventStore, EventTranslator,
    MailboxBatchAckedPayload, MailboxBatchStartedPayload, MailboxDiscardedPayload,
    MailboxQueuedPayload, Phase, PromptFactsProvider, ResolvedRuntimeConfig, Result,
    RuntimeMetricsRecorder, SessionId, SessionMeta, StorageEvent, StorageEventPayload, StoredEvent,
    event::generate_session_id,
};
use astrcode_kernel::{Kernel, PendingParentDelivery};
use chrono::Utc;
use dashmap::DashMap;
use thiserror::Error;
use tokio::{sync::broadcast, time::sleep};

pub mod actor;
pub mod catalog;
pub mod context;
pub mod context_window;
pub mod factory;
mod heuristics;
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
pub use query::{
    AgentObserveSnapshot, ProjectedTurnOutcome, SessionHistorySnapshot, SessionReplay,
    SessionViewSnapshot, TurnTerminalSnapshot, build_agent_observe_snapshot, current_turn_messages,
    has_terminal_turn_signal, project_turn_outcome, recoverable_parent_deliveries,
};
pub use state::{
    MailboxEventAppend, SessionSnapshot, SessionState, SessionStateEventSink, SessionWriter,
    append_and_broadcast, append_batch_acked, append_batch_started, append_mailbox_discarded,
    append_mailbox_event, append_mailbox_queued, complete_session_execution,
    display_name_from_working_dir, normalize_session_id, normalize_working_dir,
    prepare_session_execution, recent_turn_event_tail, should_record_compaction_tail_event,
};
pub use turn::{
    AgentPromptSubmission, TurnCollaborationSummary, TurnFinishReason, TurnOutcome, TurnRunRequest,
    TurnRunResult, TurnSummary, run_turn,
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
    metrics: Arc<dyn RuntimeMetricsRecorder>,
    sessions: DashMap<SessionId, Arc<LoadedSession>>,
    event_store: Arc<dyn EventStore>,
    catalog_events: broadcast::Sender<SessionCatalogEvent>,
}

impl SessionRuntime {
    pub fn new(
        kernel: Arc<Kernel>,
        prompt_facts_provider: Arc<dyn PromptFactsProvider>,
        event_store: Arc<dyn EventStore>,
        metrics: Arc<dyn RuntimeMetricsRecorder>,
    ) -> Self {
        let (catalog_events, _) = broadcast::channel(256);
        Self {
            kernel,
            prompt_facts_provider,
            metrics,
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

    pub fn list_running_sessions(&self) -> Vec<SessionId> {
        let mut sessions = self
            .sessions
            .iter()
            .filter(|entry| {
                entry
                    .value()
                    .actor
                    .state()
                    .running
                    .load(std::sync::atomic::Ordering::SeqCst)
            })
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
        self.create_session_with_parent(working_dir, None).await
    }

    pub async fn create_child_session(
        &self,
        working_dir: impl Into<String>,
        parent_session_id: impl Into<String>,
    ) -> Result<SessionMeta> {
        self.create_session_with_parent(working_dir, Some(parent_session_id.into()))
            .await
    }

    async fn create_session_with_parent(
        &self,
        working_dir: impl Into<String>,
        parent_session_id: Option<String>,
    ) -> Result<SessionMeta> {
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
        let lineage_parent_session_id = parent_session_id.clone();
        let actor = Arc::new(
            SessionActor::new_persistent_with_lineage(
                session_id.clone(),
                working_dir.display().to_string(),
                AgentId::from(ROOT_AGENT_ID.to_string()),
                Arc::clone(&self.event_store),
                lineage_parent_session_id,
                None,
            )
            .await
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
            parent_session_id,
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

    /// 读取指定 session 的工作目录。
    pub async fn get_session_working_dir(&self, session_id: &str) -> Result<String> {
        let session_id = SessionId::from(normalize_session_id(session_id));
        let actor = self.ensure_loaded_session(&session_id).await?;
        Ok(actor.working_dir().to_string())
    }

    /// 回放指定 session 的全部持久化事件。
    ///
    /// 用于 agent 编排层需要从 durable 事件中提取 mailbox 信封等场景。
    pub async fn replay_stored_events(&self, session_id: &SessionId) -> Result<Vec<StoredEvent>> {
        self.ensure_session_exists(session_id).await?;
        self.event_store.replay(session_id).await
    }

    /// 等待指定 turn 进入可判定终态，并返回该 turn 的 durable 事件快照。
    pub async fn wait_for_turn_terminal_snapshot(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> Result<TurnTerminalSnapshot> {
        let session_id = SessionId::from(normalize_session_id(session_id));
        loop {
            let state = self.get_session_state(&session_id).await?;
            let phase = state.current_phase()?;
            if matches!(phase, Phase::Idle | Phase::Interrupted | Phase::Done) {
                let events = self
                    .replay_stored_events(&session_id)
                    .await?
                    .into_iter()
                    .filter(|stored| stored.event.turn_id() == Some(turn_id))
                    .collect::<Vec<_>>();
                if has_terminal_turn_signal(&events) || matches!(phase, Phase::Interrupted) {
                    return Ok(TurnTerminalSnapshot { phase, events });
                }
            }
            sleep(Duration::from_millis(20)).await;
        }
    }

    /// 生成面向 agent 编排的单 session observe 快照。
    pub async fn observe_agent_session(
        &self,
        open_session_id: &str,
        target_agent_id: &str,
        lifecycle_status: AgentLifecycleStatus,
    ) -> Result<AgentObserveSnapshot> {
        let session_id = SessionId::from(normalize_session_id(open_session_id));
        let session_state = self.get_session_state(&session_id).await?;
        let projected = session_state.snapshot_projected_state()?;
        let mailbox_projection = session_state.mailbox_projection_for_agent(target_agent_id)?;
        let stored_events = self.replay_stored_events(&session_id).await?;
        Ok(build_agent_observe_snapshot(
            lifecycle_status,
            &projected,
            &mailbox_projection,
            &stored_events,
            target_agent_id,
        ))
    }

    /// 读取指定 agent 当前 mailbox durable 投影中的待处理 delivery id。
    pub async fn pending_delivery_ids_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Vec<String>> {
        let session_id = SessionId::from(normalize_session_id(session_id));
        let session_state = self.get_session_state(&session_id).await?;
        Ok(session_state
            .mailbox_projection_for_agent(agent_id)?
            .pending_delivery_ids)
    }

    pub async fn append_agent_mailbox_queued(
        &self,
        session_id: &str,
        turn_id: &str,
        agent: AgentEventContext,
        payload: MailboxQueuedPayload,
    ) -> Result<StoredEvent> {
        self.append_agent_mailbox_event(
            session_id,
            turn_id,
            agent,
            MailboxEventAppend::Queued(payload),
        )
        .await
    }

    pub async fn append_agent_mailbox_discarded(
        &self,
        session_id: &str,
        turn_id: &str,
        agent: AgentEventContext,
        payload: MailboxDiscardedPayload,
    ) -> Result<StoredEvent> {
        self.append_agent_mailbox_event(
            session_id,
            turn_id,
            agent,
            MailboxEventAppend::Discarded(payload),
        )
        .await
    }

    pub async fn append_agent_mailbox_batch_started(
        &self,
        session_id: &str,
        turn_id: &str,
        agent: AgentEventContext,
        payload: MailboxBatchStartedPayload,
    ) -> Result<StoredEvent> {
        self.append_agent_mailbox_event(
            session_id,
            turn_id,
            agent,
            MailboxEventAppend::BatchStarted(payload),
        )
        .await
    }

    pub async fn append_agent_mailbox_batch_acked(
        &self,
        session_id: &str,
        turn_id: &str,
        agent: AgentEventContext,
        payload: MailboxBatchAckedPayload,
    ) -> Result<StoredEvent> {
        self.append_agent_mailbox_event(
            session_id,
            turn_id,
            agent,
            MailboxEventAppend::BatchAcked(payload),
        )
        .await
    }

    async fn append_agent_mailbox_event(
        &self,
        session_id: &str,
        turn_id: &str,
        agent: AgentEventContext,
        event: MailboxEventAppend,
    ) -> Result<StoredEvent> {
        let session_id = SessionId::from(normalize_session_id(session_id));
        let session_state = self.get_session_state(&session_id).await?;
        let mut translator = EventTranslator::new(session_state.current_phase()?);
        append_mailbox_event(&session_state, turn_id, agent, event, &mut translator).await
    }

    /// 向指定父 session 追加 `ChildSessionNotification` durable 事件。
    pub async fn append_child_session_notification(
        &self,
        session_id: &str,
        turn_id: &str,
        agent: AgentEventContext,
        notification: ChildSessionNotification,
    ) -> Result<StoredEvent> {
        let session_id = SessionId::from(normalize_session_id(session_id));
        let session_state = self.get_session_state(&session_id).await?;
        let mut translator = EventTranslator::new(session_state.current_phase()?);
        append_and_broadcast(
            &session_state,
            &StorageEvent {
                turn_id: Some(turn_id.to_string()),
                agent,
                payload: StorageEventPayload::ChildSessionNotification {
                    notification,
                    timestamp: Some(Utc::now()),
                },
            },
            &mut translator,
        )
        .await
    }

    /// 向指定 session 追加 agent collaboration durable 事实。
    pub async fn append_agent_collaboration_fact(
        &self,
        session_id: &str,
        turn_id: &str,
        agent: AgentEventContext,
        fact: AgentCollaborationFact,
    ) -> Result<StoredEvent> {
        let session_id = SessionId::from(normalize_session_id(session_id));
        let session_state = self.get_session_state(&session_id).await?;
        let mut translator = EventTranslator::new(session_state.current_phase()?);
        append_and_broadcast(
            &session_state,
            &StorageEvent {
                turn_id: Some(turn_id.to_string()),
                agent,
                payload: StorageEventPayload::AgentCollaborationFact {
                    fact,
                    timestamp: Some(Utc::now()),
                },
            },
            &mut translator,
        )
        .await
    }

    /// 从 durable mailbox + child notification 中恢复仍可重试的父级 delivery。
    pub async fn recoverable_parent_deliveries(
        &self,
        parent_session_id: &str,
    ) -> Result<Vec<PendingParentDelivery>> {
        let session_id = SessionId::from(normalize_session_id(parent_session_id));
        let events = self.replay_stored_events(&session_id).await?;
        Ok(recoverable_parent_deliveries(&events))
    }

    /// 基于单 session terminal 事件投影出结构化 turn outcome。
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

    pub async fn compact_session(
        &self,
        session_id: &str,
        runtime: ResolvedRuntimeConfig,
    ) -> Result<bool> {
        let session_id = SessionId::from(normalize_session_id(session_id));
        let actor = self.ensure_loaded_session(&session_id).await?;
        if actor
            .state()
            .running
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            actor.state().request_manual_compact(runtime)?;
            return Ok(true);
        }
        let mut translator = EventTranslator::new(actor.state().current_phase()?);
        if let Some(events) = crate::turn::manual_compact::build_manual_compact_events(
            crate::turn::manual_compact::ManualCompactRequest {
                gateway: self.kernel.gateway(),
                prompt_facts_provider: self.prompt_facts_provider.as_ref(),
                session_state: actor.state(),
                session_id: session_id.as_str(),
                working_dir: Path::new(actor.working_dir()),
                runtime: &runtime,
            },
        )
        .await?
        {
            for event in &events {
                append_and_broadcast(actor.state(), event, &mut translator).await?;
            }
        }
        Ok(false)
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
