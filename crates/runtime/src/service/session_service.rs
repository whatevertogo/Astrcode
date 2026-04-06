use std::{path::PathBuf, sync::Arc, time::Instant};

use astrcode_core::{
    AgentStateProjector, AstrError, DeleteProjectResult, Phase, SessionMeta, StorageEvent,
    StoredEvent, generate_session_id, phase_of_storage_event, replay_records,
};
use astrcode_runtime_session::{
    SessionState, SessionWriter, display_name_from_working_dir, normalize_session_id,
    normalize_working_dir,
};
use chrono::Utc;

use super::{
    RuntimeService, ServiceError, ServiceResult, SessionCatalogEvent, SessionHistorySnapshot,
    SessionReplaySource,
};
use crate::service::blocking_bridge::spawn_blocking_service;

/// 会话服务：封装会话生命周期与状态重建逻辑。
///
/// 拆分该组件的目的是把 RuntimeService 降级为门面，
/// 会话域后续可单独演进（例如批量加载策略、缓存淘汰策略）。
pub(super) struct SessionService<'a> {
    runtime: &'a RuntimeService,
}

impl<'a> SessionService<'a> {
    pub(super) fn new(runtime: &'a RuntimeService) -> Self {
        Self { runtime }
    }

    pub(super) async fn list_sessions_with_meta(&self) -> ServiceResult<Vec<SessionMeta>> {
        let session_manager = Arc::clone(&self.runtime.session_manager);
        spawn_blocking_service("list sessions with metadata", move || {
            session_manager
                .list_sessions_with_meta()
                .map_err(ServiceError::from)
        })
        .await
    }

    pub(super) async fn create_session(
        &self,
        working_dir: impl Into<PathBuf>,
    ) -> ServiceResult<SessionMeta> {
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
                let session_start = StorageEvent::SessionStart {
                    session_id: session_id.clone(),
                    timestamp: created_at,
                    working_dir: working_dir.to_string_lossy().to_string(),
                    parent_session_id: None,
                    parent_storage_seq: None,
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

        self.runtime
            .emit_session_catalog_event(SessionCatalogEvent::SessionCreated {
                session_id: meta.session_id.clone(),
            });

        Ok(meta)
    }

    pub(super) async fn load_session_history(
        &self,
        session_id: &str,
    ) -> ServiceResult<SessionHistorySnapshot> {
        let session_id = normalize_session_id(session_id);
        let state = self.ensure_session_loaded(&session_id).await?;
        let phase = state
            .current_phase()
            .map_err(|error| ServiceError::Internal(AstrError::Internal(error.to_string())))?;
        let history = self.runtime.replay(&session_id, None).await?.history;
        let cursor = history.last().map(|record| record.event_id.clone());
        Ok(SessionHistorySnapshot {
            history,
            cursor,
            phase,
        })
    }

    pub(super) async fn ensure_session_loaded(
        &self,
        session_id: &str,
    ) -> ServiceResult<Arc<SessionState>> {
        if let Some(existing) = self.runtime.sessions.get(session_id) {
            return Ok(existing.clone());
        }

        let _guard = self.runtime.session_load_lock.lock().await;
        if let Some(existing) = self.runtime.sessions.get(session_id) {
            return Ok(existing.clone());
        }

        let session_id_owned = session_id.to_string();
        let session_manager = Arc::clone(&self.runtime.session_manager);
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

            let working_dir = match &first.event {
                StorageEvent::SessionStart { working_dir, .. } => PathBuf::from(working_dir),
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
                self.runtime
                    .observability
                    .record_session_rehydrate(elapsed, true);
                if elapsed.as_millis() >= 250 {
                    log::warn!(
                        "session '{}' rehydrate took {}ms",
                        session_id,
                        elapsed.as_millis()
                    );
                }
            },
            Err(error) => {
                self.runtime
                    .observability
                    .record_session_rehydrate(elapsed, false);
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
        self.runtime
            .sessions
            .insert(session_id.to_string(), state.clone());
        Ok(state)
    }

    pub(super) async fn delete_session(&self, session_id: &str) -> ServiceResult<()> {
        let normalized = normalize_session_id(session_id);
        let _guard = self.runtime.session_load_lock.lock().await;
        self.runtime.interrupt(&normalized).await?;
        self.runtime.sessions.remove(&normalized);
        let session_manager = Arc::clone(&self.runtime.session_manager);
        let delete_session_id = normalized.clone();
        spawn_blocking_service("delete session", move || {
            session_manager
                .delete_session(&delete_session_id)
                .map_err(ServiceError::from)
        })
        .await?;
        self.runtime
            .emit_session_catalog_event(SessionCatalogEvent::SessionDeleted {
                session_id: normalized,
            });
        Ok(())
    }

    pub(super) async fn delete_project(
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
            let _ = self.runtime.interrupt(session_id).await;
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
        self.runtime
            .emit_session_catalog_event(SessionCatalogEvent::ProjectDeleted { working_dir });
        Ok(result)
    }
}
