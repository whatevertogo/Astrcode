use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use astrcode_core::{
    AgentCollaborationFact, AgentEventContext, AgentId, AgentLifecycleStatus, ChildSessionNode,
    ChildSessionNotification, DeleteProjectResult, EventStore, InputBatchAckedPayload,
    InputBatchStartedPayload, InputDiscardedPayload, InputQueuedPayload, Phase,
    PromptFactsProvider, ResolvedRuntimeConfig, Result, RuntimeMetricsRecorder, SessionId,
    SessionMeta, StoredEvent, event::generate_session_id,
};
use astrcode_kernel::{Kernel, PendingParentDelivery};
use chrono::Utc;
use dashmap::DashMap;
use thiserror::Error;
use tokio::sync::broadcast;

mod actor;
mod catalog;
mod command;
mod context_window;
mod heuristics;
pub mod identity;
mod observe;
mod query;
mod state;
mod turn;

use actor::SessionActor;
pub use catalog::SessionCatalogEvent;
pub use observe::{
    SessionEventFilterSpec, SessionObserveSnapshot, SubRunEventScope, SubRunStatusSnapshot,
    SubRunStatusSource,
};
pub use query::{
    AgentObserveSnapshot, ConversationAssistantBlockFacts, ConversationBlockFacts,
    ConversationBlockPatchFacts, ConversationBlockStatus, ConversationChildHandoffBlockFacts,
    ConversationChildHandoffKind, ConversationDeltaFacts, ConversationDeltaFrameFacts,
    ConversationDeltaProjector, ConversationErrorBlockFacts, ConversationPlanBlockFacts,
    ConversationPlanBlockersFacts, ConversationPlanEventKind, ConversationPlanReviewFacts,
    ConversationPlanReviewKind, ConversationSnapshotFacts, ConversationStreamProjector,
    ConversationStreamReplayFacts, ConversationSystemNoteBlockFacts, ConversationSystemNoteKind,
    ConversationThinkingBlockFacts, ConversationTranscriptErrorKind, ConversationUserBlockFacts,
    LastCompactMetaSnapshot, ProjectedTurnOutcome, SessionControlStateSnapshot,
    SessionModeSnapshot, SessionReplay, SessionTranscriptSnapshot, ToolCallBlockFacts,
    ToolCallStreamsFacts, TurnTerminalSnapshot, recoverable_parent_deliveries,
};
pub(crate) use state::SessionStateEventSink;
pub use state::{
    SessionSnapshot, SessionState, display_name_from_working_dir, normalize_working_dir,
};
pub use turn::{
    AgentPromptSubmission, ForkPoint, ForkResult, TurnCollaborationSummary, TurnFinishReason,
    TurnSummary,
};
pub(crate) use turn::{TurnOutcome, TurnRunResult, run_turn};

pub const ROOT_AGENT_ID: &str = "root-agent";

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
    pub(crate) kernel: Arc<Kernel>,
    pub(crate) prompt_facts_provider: Arc<dyn PromptFactsProvider>,
    metrics: Arc<dyn RuntimeMetricsRecorder>,
    sessions: DashMap<SessionId, Arc<LoadedSession>>,
    pub(crate) event_store: Arc<dyn EventStore>,
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

    pub(crate) fn query(&self) -> query::SessionQueries<'_> {
        query::SessionQueries::new(self)
    }

    pub(crate) fn command(&self) -> command::SessionCommands<'_> {
        command::SessionCommands::new(self)
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
            .filter(|entry| entry.value().actor.state().is_running())
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
        self.query().observe(session_id).await
    }

    /// 按需加载 session 并返回内部状态引用。
    ///
    /// 用于 agent 编排层需要直接操作 SessionState 的场景
    /// （如 input queue 追加、对话投影读取等）。
    pub async fn get_session_state(&self, session_id: &SessionId) -> Result<Arc<SessionState>> {
        self.query().session_state(session_id).await
    }

    /// 读取会话控制态快照，供 application / conversation surface 编排使用。
    pub async fn session_control_state(
        &self,
        session_id: &str,
    ) -> Result<SessionControlStateSnapshot> {
        self.query().session_control_state(session_id).await
    }

    pub async fn conversation_snapshot(
        &self,
        session_id: &str,
    ) -> Result<ConversationSnapshotFacts> {
        self.query().conversation_snapshot(session_id).await
    }

    pub async fn conversation_stream_replay(
        &self,
        session_id: &str,
        last_event_id: Option<&str>,
    ) -> Result<ConversationStreamReplayFacts> {
        self.query()
            .conversation_stream_replay(session_id, last_event_id)
            .await
    }

    /// 返回当前 session durable 可见的 direct child lineage 节点。
    pub async fn session_child_nodes(&self, session_id: &str) -> Result<Vec<ChildSessionNode>> {
        self.query().session_child_nodes(session_id).await
    }

    pub async fn session_mode_state(&self, session_id: &str) -> Result<SessionModeSnapshot> {
        self.query().session_mode_state(session_id).await
    }

    pub async fn active_task_snapshot(
        &self,
        session_id: &str,
        owner: &str,
    ) -> Result<Option<astrcode_core::TaskSnapshot>> {
        self.query().active_task_snapshot(session_id, owner).await
    }

    /// 读取指定 session 的工作目录。
    pub async fn get_session_working_dir(&self, session_id: &str) -> Result<String> {
        self.query().session_working_dir(session_id).await
    }

    /// 回放指定 session 的全部持久化事件。
    ///
    /// 用于 agent 编排层需要从 durable 事件中提取 input queue 信封等场景。
    pub async fn replay_stored_events(&self, session_id: &SessionId) -> Result<Vec<StoredEvent>> {
        self.query().stored_events(session_id).await
    }

    /// 等待指定 turn 进入可判定终态，并返回该 turn 的 durable 事件快照。
    pub async fn wait_for_turn_terminal_snapshot(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> Result<TurnTerminalSnapshot> {
        self.query()
            .wait_for_turn_terminal_snapshot(session_id, turn_id)
            .await
    }

    /// 生成面向 agent 编排的单 session observe 快照。
    pub async fn observe_agent_session(
        &self,
        open_session_id: &str,
        target_agent_id: &str,
        lifecycle_status: AgentLifecycleStatus,
    ) -> Result<AgentObserveSnapshot> {
        self.query()
            .observe_agent_session(open_session_id, target_agent_id, lifecycle_status)
            .await
    }

    /// 读取指定 agent 当前 input queue durable 投影中的待处理 delivery id。
    pub async fn pending_delivery_ids_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Vec<String>> {
        self.query()
            .pending_delivery_ids_for_agent(session_id, agent_id)
            .await
    }

    pub async fn append_agent_input_queued(
        &self,
        session_id: &str,
        turn_id: &str,
        agent: AgentEventContext,
        payload: InputQueuedPayload,
    ) -> Result<StoredEvent> {
        self.command()
            .append_agent_input_queued(session_id, turn_id, agent, payload)
            .await
    }

    pub async fn append_agent_input_discarded(
        &self,
        session_id: &str,
        turn_id: &str,
        agent: AgentEventContext,
        payload: InputDiscardedPayload,
    ) -> Result<StoredEvent> {
        self.command()
            .append_agent_input_discarded(session_id, turn_id, agent, payload)
            .await
    }

    pub async fn append_agent_input_batch_started(
        &self,
        session_id: &str,
        turn_id: &str,
        agent: AgentEventContext,
        payload: InputBatchStartedPayload,
    ) -> Result<StoredEvent> {
        self.command()
            .append_agent_input_batch_started(session_id, turn_id, agent, payload)
            .await
    }

    pub async fn append_agent_input_batch_acked(
        &self,
        session_id: &str,
        turn_id: &str,
        agent: AgentEventContext,
        payload: InputBatchAckedPayload,
    ) -> Result<StoredEvent> {
        self.command()
            .append_agent_input_batch_acked(session_id, turn_id, agent, payload)
            .await
    }

    /// 向指定父 session 追加 `ChildSessionNotification` durable 事件。
    pub async fn append_child_session_notification(
        &self,
        session_id: &str,
        turn_id: &str,
        agent: AgentEventContext,
        notification: ChildSessionNotification,
    ) -> Result<StoredEvent> {
        self.command()
            .append_child_session_notification(session_id, turn_id, agent, notification)
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
        self.command()
            .append_agent_collaboration_fact(session_id, turn_id, agent, fact)
            .await
    }

    /// 从 durable input queue + child notification 中恢复仍可重试的父级 delivery。
    pub async fn recoverable_parent_deliveries(
        &self,
        parent_session_id: &str,
    ) -> Result<Vec<PendingParentDelivery>> {
        self.query()
            .recoverable_parent_deliveries(parent_session_id)
            .await
    }

    /// 基于单 session terminal 事件投影出结构化 turn outcome。
    pub async fn project_turn_outcome(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> Result<ProjectedTurnOutcome> {
        self.query().project_turn_outcome(session_id, turn_id).await
    }

    pub async fn delete_session(&self, session_id: &str) -> Result<()> {
        let session_id = SessionId::from(state::normalize_session_id(session_id));
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
        instructions: Option<String>,
    ) -> Result<bool> {
        self.command()
            .compact_session(session_id, &runtime, instructions.as_deref())
            .await
    }

    pub async fn switch_mode(
        &self,
        session_id: &str,
        from: astrcode_core::ModeId,
        to: astrcode_core::ModeId,
    ) -> Result<StoredEvent> {
        self.command().switch_mode(session_id, from, to).await
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
            .find(|meta| state::normalize_session_id(&meta.session_id) == session_id.as_str())
            .ok_or_else(|| SessionRuntimeError::SessionNotFound(session_id.to_string()))?;
        Ok(meta.phase)
    }

    pub(crate) async fn ensure_loaded_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Arc<SessionActor>> {
        if let Some(entry) = self.sessions.get(session_id) {
            return Ok(Arc::clone(&entry.actor));
        }
        let meta = self
            .event_store
            .list_session_metas()
            .await?
            .into_iter()
            .find(|meta| state::normalize_session_id(&meta.session_id) == session_id.as_str())
            .ok_or_else(|| SessionRuntimeError::SessionNotFound(session_id.to_string()))?;
        let recovered = self.event_store.recover_session(session_id).await?;
        let actor = Arc::new(SessionActor::from_recovery(
            session_id.clone(),
            meta.working_dir,
            AgentId::from(ROOT_AGENT_ID.to_string()),
            Arc::clone(&self.event_store),
            recovered,
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

    pub(crate) async fn ensure_session_exists(&self, session_id: &SessionId) -> Result<()> {
        if self.sessions.contains_key(session_id) {
            return Ok(());
        }
        let exists = self
            .event_store
            .list_session_metas()
            .await?
            .into_iter()
            .any(|meta| state::normalize_session_id(&meta.session_id) == session_id.as_str());
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
