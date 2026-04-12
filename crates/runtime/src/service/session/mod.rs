//! session owner handle 与内部辅助逻辑。
//!
//! 对外只通过 `RuntimeService::sessions()` 暴露单一 surface；
//! 其余 crate 内部代码只复用这里的加载/回放辅助函数，不再经过第二层 façade。

mod create;
mod delete;

use std::{path::PathBuf, sync::Arc, time::Instant};

use astrcode_core::{
    AgentEvent, AgentStateProjector, AstrError, DeleteProjectResult, Phase, SessionEventRecord,
    SessionMeta, StorageEvent, StorageEventPayload, StoredEvent, generate_session_id,
    phase_of_storage_event, replay_records,
};
use astrcode_runtime_agent_loop::CompactionTailSnapshot;
use astrcode_runtime_execution::{ExecutionLineageIndex, ExecutionLineageScope};
use astrcode_runtime_session::{
    SessionState, SessionWriter, display_name_from_working_dir, normalize_session_id,
    normalize_working_dir, recent_turn_event_tail,
};
use async_trait::async_trait;
use chrono::Utc;
use tokio::sync::broadcast;

use super::{
    RuntimeService, ServiceError, ServiceResult, SessionCatalogEvent, SessionEventFilterSpec,
    SessionHistorySnapshot, SessionViewSnapshot, SubRunEventScope,
    blocking_bridge::{lock_anyhow, spawn_blocking_service},
};

/// `runtime-session` 的唯一 owner handle。
#[derive(Clone)]
pub struct SessionServiceHandle {
    pub(crate) runtime: Arc<RuntimeService>,
}

#[derive(Debug, Clone)]
pub struct SessionEventFilter {
    spec: SessionEventFilterSpec,
    lineage: ExecutionLineageIndex,
}

impl SessionServiceHandle {
    pub(super) fn new(runtime: Arc<RuntimeService>) -> Self {
        Self { runtime }
    }

    pub async fn list(&self) -> ServiceResult<Vec<SessionMeta>> {
        let session_manager = Arc::clone(&self.runtime.session_manager);
        spawn_blocking_service("list sessions with metadata", move || {
            session_manager
                .list_sessions_with_meta()
                .map_err(ServiceError::from)
        })
        .await
    }

    pub async fn create(&self, working_dir: impl Into<PathBuf>) -> ServiceResult<SessionMeta> {
        let working_dir = working_dir.into();
        let session_manager = Arc::clone(&self.runtime.session_manager);
        let (session_id, working_dir, created_at, log, stored_session_start) =
            spawn_blocking_service("create session", move || {
                let working_dir = normalize_working_dir(working_dir)?;
                let session_id = generate_session_id();
                let mut log = session_manager
                    .create_event_log(&session_id, &working_dir)
                    .map_err(ServiceError::from)?;
                let created_at = Utc::now();
                let session_start = StorageEvent {
                    turn_id: None,
                    agent: astrcode_core::AgentEventContext::default(),
                    payload: StorageEventPayload::SessionStart {
                        session_id: session_id.clone(),
                        timestamp: created_at,
                        working_dir: working_dir.to_string_lossy().to_string(),
                        parent_session_id: None,
                        parent_storage_seq: None,
                    },
                };
                let stored_session_start =
                    log.append(&session_start).map_err(ServiceError::from)?;
                Ok((
                    session_id,
                    working_dir,
                    created_at,
                    log,
                    stored_session_start,
                ))
            })
            .await?;

        let phase = phase_of_storage_event(&stored_session_start.event);
        let state = Arc::new(SessionState::new(
            phase,
            Arc::new(SessionWriter::new(log)),
            AgentStateProjector::from_events(std::slice::from_ref(&stored_session_start.event)),
            replay_records(std::slice::from_ref(&stored_session_start), None),
            vec![stored_session_start.clone()],
        ));
        self.runtime.sessions.insert(session_id.clone(), state);

        let meta = SessionMeta {
            session_id,
            working_dir: working_dir.to_string_lossy().to_string(),
            display_name: display_name_from_working_dir(&working_dir),
            title: "新会话".to_string(),
            created_at,
            updated_at: created_at,
            parent_session_id: None,
            parent_storage_seq: None,
            phase: Phase::Idle,
        };

        // 故意忽略：通道关闭表示服务已关闭，无需处理
        let _ = self
            .runtime
            .session_catalog_events
            .send(SessionCatalogEvent::SessionCreated {
                session_id: meta.session_id.clone(),
            });

        Ok(meta)
    }

    pub async fn history(&self, session_id: &str) -> ServiceResult<SessionHistorySnapshot> {
        let session_id = normalize_session_id(session_id);
        let state = self.runtime.ensure_session_loaded(&session_id).await?;
        let phase = state
            .current_phase()
            .map_err(|error| ServiceError::Internal(AstrError::Internal(error.to_string())))?;
        let history = self.replay(&session_id, None).await?.history;
        let cursor = history.last().map(|record| record.event_id.clone());
        Ok(SessionHistorySnapshot {
            history,
            cursor,
            phase,
        })
    }

    pub async fn history_filtered(
        &self,
        session_id: &str,
        filter_spec: Option<SessionEventFilterSpec>,
    ) -> ServiceResult<SessionHistorySnapshot> {
        let snapshot = self.history(session_id).await?;
        let SessionHistorySnapshot {
            history,
            cursor: _,
            phase,
        } = snapshot;
        let filtered_history = if let Some(filter_spec) = filter_spec {
            filter_owned_history(history, filter_spec)?
        } else {
            history
                .into_iter()
                .filter(parent_timeline_event_visible)
                .collect::<Vec<_>>()
        };
        let cursor = filtered_history
            .last()
            .map(|record| record.event_id.clone());
        Ok(SessionHistorySnapshot {
            history: filtered_history,
            cursor,
            phase,
        })
    }

    pub async fn view(
        &self,
        session_id: &str,
        filter_spec: Option<SessionEventFilterSpec>,
    ) -> ServiceResult<SessionViewSnapshot> {
        let snapshot = self.history(session_id).await?;
        let cursor = snapshot.cursor.clone();
        let phase = snapshot.phase;
        let history = snapshot.history;

        let focus_history = filter_spec.as_ref().map_or_else(
            || {
                Ok(history
                    .iter()
                    .filter(|record| parent_timeline_event_visible(record))
                    .cloned()
                    .collect::<Vec<_>>())
            },
            |spec| filter_cloned_history(&history, spec.clone()),
        )?;

        let direct_children_history = if let Some(spec) = filter_spec.as_ref() {
            filter_cloned_history(
                &history,
                SessionEventFilterSpec {
                    target_sub_run_id: spec.target_sub_run_id.clone(),
                    scope: SubRunEventScope::DirectChildren,
                },
            )?
        } else {
            Vec::new()
        };

        Ok(SessionViewSnapshot {
            focus_history,
            direct_children_history,
            cursor,
            phase,
        })
    }

    pub async fn compact(&self, session_id: &str) -> ServiceResult<()> {
        let session_id = normalize_session_id(session_id);
        let session = self.runtime.ensure_session_loaded(&session_id).await?;
        if session.running.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(ServiceError::Conflict(format!(
                "session '{}' is busy; manual compact is only allowed while idle",
                session_id
            )));
        }

        let loop_ = self.runtime.loop_surface().current_loop().await;
        let projected = session
            .snapshot_projected_state()
            .map_err(|error| ServiceError::Internal(AstrError::Internal(error.to_string())))?;
        let recent_stored_events = session
            .snapshot_recent_stored_events()
            .map_err(|error| ServiceError::Internal(AstrError::Internal(error.to_string())))?;
        let compaction_tail = CompactionTailSnapshot::from_seed(recent_turn_event_tail(
            &recent_stored_events,
            loop_.compact_keep_recent_turns(),
        ));
        let compact_event = loop_
            .manual_compact_event(&projected, compaction_tail, Some(&recent_stored_events))
            .await
            .map_err(ServiceError::from)?;

        let Some(compact_event) = compact_event else {
            if let Ok(mut failures) =
                lock_anyhow(&session.compact_failure_count, "compact failures")
            {
                *failures = 0;
            }
            return Err(ServiceError::InvalidInput(
                "manual compact found no compressible history; the session needs at least 2 user \
                 turns before it can be compacted"
                    .to_string(),
            ));
        };

        let initial_phase = lock_anyhow(&session.phase, "session phase")
            .map(|guard| *guard)
            .unwrap_or(Phase::Idle);
        let mut translator = astrcode_core::EventTranslator::new(initial_phase);
        astrcode_runtime_session::append_and_broadcast(&session, &compact_event, &mut translator)
            .await
            .map_err(|error| ServiceError::Internal(AstrError::Internal(error.to_string())))?;
        if let Ok(mut phase) = lock_anyhow(&session.phase, "session phase") {
            *phase = translator.phase();
        }
        if let Ok(mut failures) = lock_anyhow(&session.compact_failure_count, "compact failures") {
            *failures = 0;
        }
        Ok(())
    }

    pub(crate) async fn purge_session_durable(&self, session_id: &str) -> ServiceResult<()> {
        let normalized = normalize_session_id(session_id);
        let _guard = self.runtime.session_load_lock.lock().await;
        self.runtime.sessions.remove(&normalized);
        let session_manager = Arc::clone(&self.runtime.session_manager);
        let delete_session_id = normalized.clone();
        spawn_blocking_service("delete session", move || {
            session_manager
                .delete_session(&delete_session_id)
                .map_err(ServiceError::from)
        })
        .await?;
        // 故意忽略：通道关闭表示服务已关闭，无需处理
        let _ = self
            .runtime
            .session_catalog_events
            .send(SessionCatalogEvent::SessionDeleted {
                session_id: normalized,
            });
        Ok(())
    }

    pub(crate) async fn purge_project_durable(
        &self,
        working_dir: &str,
    ) -> ServiceResult<DeleteProjectResult> {
        let working_dir = working_dir.to_string();
        let session_manager = Arc::clone(&self.runtime.session_manager);
        let metas = spawn_blocking_service("list project sessions", move || {
            session_manager
                .list_sessions_with_meta()
                .map_err(ServiceError::from)
        })
        .await?;
        let targets = metas
            .into_iter()
            .filter(|meta| meta.working_dir == working_dir)
            .map(|meta| meta.session_id)
            .collect::<Vec<_>>();

        for session_id in &targets {
            self.runtime.sessions.remove(session_id);
        }

        let delete_working_dir = working_dir.clone();
        let session_manager = Arc::clone(&self.runtime.session_manager);
        let result = spawn_blocking_service("delete project sessions", move || {
            session_manager
                .delete_sessions_by_working_dir(&delete_working_dir)
                .map_err(ServiceError::from)
        })
        .await?;
        // 故意忽略：通道关闭表示服务已关闭，无需处理
        let _ = self
            .runtime
            .session_catalog_events
            .send(SessionCatalogEvent::ProjectDeleted { working_dir });
        Ok(result)
    }

    pub fn subscribe_catalog(&self) -> broadcast::Receiver<SessionCatalogEvent> {
        self.runtime.session_catalog_events.subscribe()
    }
}

#[async_trait]
impl astrcode_core::SessionTruthBoundary for SessionServiceHandle {
    async fn create_session(
        &self,
        working_dir: &std::path::Path,
    ) -> std::result::Result<SessionMeta, AstrError> {
        self.create(working_dir.to_path_buf())
            .await
            .map_err(service_error_to_astr)
    }

    async fn list_sessions(&self) -> std::result::Result<Vec<SessionMeta>, AstrError> {
        self.list().await.map_err(service_error_to_astr)
    }

    async fn load_history(
        &self,
        session_id: &str,
    ) -> std::result::Result<Vec<astrcode_core::SessionEventRecord>, AstrError> {
        self.history(session_id)
            .await
            .map(|snapshot| snapshot.history)
            .map_err(service_error_to_astr)
    }
}

impl RuntimeService {
    pub fn sessions(self: &Arc<Self>) -> SessionServiceHandle {
        SessionServiceHandle::new(Arc::clone(self))
    }

    /// 确保会话只在首次访问时从磁盘重建一次，避免并发加载把事件广播拆成多份状态。
    pub(crate) async fn ensure_session_loaded(
        &self,
        session_id: &str,
    ) -> ServiceResult<Arc<SessionState>> {
        if let Some(existing) = self.sessions.get(session_id) {
            return Ok(existing.clone());
        }

        let _guard = Arc::clone(&self.session_load_lock).lock_owned().await;
        if let Some(existing) = self.sessions.get(session_id) {
            return Ok(existing.clone());
        }

        let session_id_owned = session_id.to_string();
        let session_manager = Arc::clone(&self.session_manager);
        let started_at = Instant::now();
        let load_result = spawn_blocking_service("load session state", move || {
            let stored: Vec<StoredEvent> = session_manager
                .replay_events(&session_id_owned)
                .map_err(ServiceError::from)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(ServiceError::from)?;
            let Some(first) = stored.first() else {
                return Err(ServiceError::NotFound(format!(
                    "session '{}' is empty",
                    session_id_owned
                )));
            };

            let working_dir = match &first.event.payload {
                StorageEventPayload::SessionStart { working_dir, .. } => PathBuf::from(working_dir),
                _ => {
                    return Err(ServiceError::Internal(AstrError::Internal(format!(
                        "session '{}' is missing sessionStart",
                        session_id_owned
                    ))));
                },
            };
            let phase = stored
                .last()
                .map(|event| phase_of_storage_event(&event.event))
                .unwrap_or(Phase::Idle);
            let log = session_manager
                .open_event_log(&session_id_owned)
                .map_err(ServiceError::from)?;
            let events = stored
                .iter()
                .map(|record| record.event.clone())
                .collect::<Vec<_>>();
            let projector = AgentStateProjector::from_events(&events);
            let recent_records = replay_records(&stored, None);
            Ok((phase, log, projector, recent_records, stored, working_dir))
        })
        .await;
        let elapsed = started_at.elapsed();
        match &load_result {
            Ok(_) => {
                self.observability.record_session_rehydrate(elapsed, true);
                if elapsed.as_millis() >= 250 {
                    log::warn!(
                        "session '{}' rehydrate took {}ms",
                        session_id,
                        elapsed.as_millis()
                    );
                }
            },
            Err(error) => {
                self.observability.record_session_rehydrate(elapsed, false);
                log::error!(
                    "failed to rehydrate session '{}' after {}ms: {}",
                    session_id,
                    elapsed.as_millis(),
                    error
                );
            },
        }
        let (phase, log, projector, recent_records, recent_stored, _working_dir) = load_result?;

        let state = Arc::new(SessionState::new(
            phase,
            Arc::new(SessionWriter::new(log)),
            projector,
            recent_records,
            recent_stored,
        ));
        self.sessions.insert(session_id.to_string(), state.clone());
        Ok(state)
    }
}

impl SessionEventFilter {
    pub fn new(
        spec: SessionEventFilterSpec,
        history: &[SessionEventRecord],
    ) -> ServiceResult<Self> {
        let lineage = ExecutionLineageIndex::from_session_history(history);
        lineage
            .require_scope(&spec.target_sub_run_id, map_lineage_scope(spec.scope))
            .map_err(ServiceError::Conflict)?;
        Ok(Self { spec, lineage })
    }

    pub fn matches(&mut self, record: &SessionEventRecord) -> bool {
        self.lineage.observe_session_record(record);
        let Some(event_sub_run_id) = event_sub_run_id(&record.event) else {
            return false;
        };

        match self.spec.scope {
            SubRunEventScope::SelfOnly => event_sub_run_id == self.spec.target_sub_run_id,
            SubRunEventScope::DirectChildren => self
                .lineage
                .is_direct_child_of(event_sub_run_id, &self.spec.target_sub_run_id),
            SubRunEventScope::Subtree => self
                .lineage
                .is_in_subtree(event_sub_run_id, &self.spec.target_sub_run_id),
        }
    }

    pub fn spec(&self) -> &SessionEventFilterSpec {
        &self.spec
    }
}

pub(crate) async fn load_events(
    session_manager: Arc<dyn astrcode_core::SessionManager>,
    session_id: &str,
) -> ServiceResult<Vec<StoredEvent>> {
    let session_id = session_id.to_string();
    spawn_blocking_service("load session events", move || {
        session_manager
            .replay_events(&session_id)
            .map_err(ServiceError::from)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(ServiceError::from)
    })
    .await
}

fn service_error_to_astr(error: ServiceError) -> AstrError {
    match error {
        ServiceError::NotFound(message)
        | ServiceError::Conflict(message)
        | ServiceError::InvalidInput(message) => AstrError::Validation(message),
        ServiceError::Internal(error) => error,
    }
}

fn filter_owned_history(
    history: Vec<SessionEventRecord>,
    filter_spec: SessionEventFilterSpec,
) -> ServiceResult<Vec<SessionEventRecord>> {
    let mut filter = SessionEventFilter::new(filter_spec, &history)?;
    Ok(history
        .into_iter()
        .filter(|record| filter.matches(record))
        .collect())
}

fn filter_cloned_history(
    history: &[SessionEventRecord],
    filter_spec: SessionEventFilterSpec,
) -> ServiceResult<Vec<SessionEventRecord>> {
    let mut filter = SessionEventFilter::new(filter_spec, history)?;
    Ok(history
        .iter()
        .filter(|record| filter.matches(record))
        .cloned()
        .collect())
}

pub(crate) fn parent_timeline_event_visible(record: &SessionEventRecord) -> bool {
    match &record.event {
        AgentEvent::SubRunStarted { agent, .. } | AgentEvent::SubRunFinished { agent, .. } => {
            agent.storage_mode != Some(astrcode_core::SubRunStorageMode::IndependentSession)
        },
        _ => true,
    }
}

fn map_lineage_scope(scope: SubRunEventScope) -> ExecutionLineageScope {
    match scope {
        SubRunEventScope::SelfOnly => ExecutionLineageScope::SelfOnly,
        SubRunEventScope::DirectChildren => ExecutionLineageScope::DirectChildren,
        SubRunEventScope::Subtree => ExecutionLineageScope::Subtree,
    }
}

fn event_sub_run_id(event: &AgentEvent) -> Option<&str> {
    event_agent_context(event)?
        .sub_run_id
        .as_deref()
        .filter(|sub_run_id| !sub_run_id.is_empty())
}

fn event_agent_context(event: &AgentEvent) -> Option<&astrcode_core::AgentEventContext> {
    match event {
        AgentEvent::SessionStarted { .. } => None,
        AgentEvent::UserMessage { agent, .. }
        | AgentEvent::PhaseChanged { agent, .. }
        | AgentEvent::ModelDelta { agent, .. }
        | AgentEvent::ThinkingDelta { agent, .. }
        | AgentEvent::AssistantMessage { agent, .. }
        | AgentEvent::ToolCallStart { agent, .. }
        | AgentEvent::ToolCallDelta { agent, .. }
        | AgentEvent::ToolCallResult { agent, .. }
        | AgentEvent::CompactApplied { agent, .. }
        | AgentEvent::SubRunStarted { agent, .. }
        | AgentEvent::SubRunFinished { agent, .. }
        | AgentEvent::ChildSessionNotification { agent, .. }
        | AgentEvent::PromptMetrics { agent, .. }
        | AgentEvent::TurnDone { agent, .. }
        | AgentEvent::Error { agent, .. }
        | AgentEvent::AgentMailboxQueued { agent, .. }
        | AgentEvent::AgentMailboxBatchStarted { agent, .. }
        | AgentEvent::AgentMailboxBatchAcked { agent, .. }
        | AgentEvent::AgentMailboxDiscarded { agent, .. } => Some(agent),
    }
}
