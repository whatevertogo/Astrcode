use std::path::{Path, PathBuf};
use std::sync::Arc;

use astrcode_core::{AstrError, Phase};
use chrono::Utc;

use crate::event_log::{generate_session_id, DeleteProjectResult, EventLog, SessionMeta};
use crate::events::{StorageEvent, StoredEvent};

use super::replay::{convert_events_to_messages, phase_of_storage_event, replay_records};
use super::session_state::{SessionState, SessionWriter};
use super::support::spawn_blocking_service;
use super::{AgentService, ServiceError, ServiceResult, SessionMessage};

impl AgentService {
    pub async fn list_sessions_with_meta(&self) -> ServiceResult<Vec<SessionMeta>> {
        spawn_blocking_service("list sessions with metadata", || {
            EventLog::list_sessions_with_meta().map_err(ServiceError::from)
        })
        .await
    }

    pub async fn list_sessions(&self) -> ServiceResult<Vec<String>> {
        spawn_blocking_service("list sessions", || {
            EventLog::list_sessions().map_err(ServiceError::from)
        })
        .await
    }

    pub async fn create_session(
        &self,
        working_dir: impl Into<PathBuf>,
    ) -> ServiceResult<SessionMeta> {
        let working_dir = working_dir.into();
        let (session_id, working_dir, created_at, log) =
            spawn_blocking_service("create session", move || {
                let working_dir = normalize_working_dir(working_dir)?;
                let session_id = generate_session_id();
                let mut log = EventLog::create(&session_id).map_err(ServiceError::from)?;
                let created_at = Utc::now();
                let session_start = StorageEvent::SessionStart {
                    session_id: session_id.clone(),
                    timestamp: created_at,
                    working_dir: working_dir.to_string_lossy().to_string(),
                };
                let _ = log.append(&session_start).map_err(ServiceError::from)?;
                Ok((session_id, working_dir, created_at, log))
            })
            .await?;

        let state = Arc::new(SessionState::new(
            working_dir.clone(),
            Phase::Idle,
            Arc::new(SessionWriter::new(log)),
        ));
        self.sessions.insert(session_id.clone(), state);

        Ok(SessionMeta {
            session_id,
            working_dir: working_dir.to_string_lossy().to_string(),
            display_name: display_name_from_working_dir(&working_dir),
            title: "新会话".to_string(),
            created_at,
            updated_at: created_at,
            phase: Phase::Idle,
        })
    }

    pub async fn load_session_messages(
        &self,
        session_id: &str,
    ) -> ServiceResult<Vec<SessionMessage>> {
        Ok(self.load_session_snapshot(session_id).await?.0)
    }

    pub async fn load_session_snapshot(
        &self,
        session_id: &str,
    ) -> ServiceResult<(Vec<SessionMessage>, Option<String>)> {
        let session_id = normalize_session_id(session_id);
        let events = load_events(&session_id).await?;
        let cursor = replay_records(&events, None)
            .last()
            .map(|record| record.event_id.clone());
        Ok((convert_events_to_messages(&events), cursor))
    }

    pub async fn delete_session(&self, session_id: &str) -> ServiceResult<()> {
        let normalized = normalize_session_id(session_id);
        self.interrupt(&normalized).await?;
        self.sessions.remove(&normalized);
        spawn_blocking_service("delete session", move || {
            EventLog::delete_session(&normalized).map_err(ServiceError::from)
        })
        .await
    }

    pub async fn delete_project(&self, working_dir: &str) -> ServiceResult<DeleteProjectResult> {
        let working_dir = working_dir.to_string();
        let metas = spawn_blocking_service("list project sessions", || {
            EventLog::list_sessions_with_meta().map_err(ServiceError::from)
        })
        .await?;
        let targets = metas
            .into_iter()
            .filter(|meta| meta.working_dir == working_dir)
            .map(|meta| meta.session_id)
            .collect::<Vec<_>>();

        for session_id in &targets {
            let _ = self.interrupt(session_id).await;
            self.sessions.remove(session_id);
        }

        let delete_working_dir = working_dir.clone();
        spawn_blocking_service("delete project sessions", move || {
            EventLog::delete_sessions_by_working_dir(&delete_working_dir)
                .map_err(ServiceError::from)
        })
        .await
    }

    pub(super) async fn ensure_session_loaded(
        &self,
        session_id: &str,
    ) -> ServiceResult<Arc<SessionState>> {
        if let Some(existing) = self.sessions.get(session_id) {
            return Ok(existing.clone());
        }

        let _guard = self.session_load_lock.lock().await;
        if let Some(existing) = self.sessions.get(session_id) {
            return Ok(existing.clone());
        }

        let session_id_owned = session_id.to_string();
        let (working_dir, phase, log) = spawn_blocking_service("load session state", move || {
            let stored =
                EventLog::load(&session_id_owned).map_err(|error| match error.to_string() {
                    message if message.contains("session file not found") => {
                        ServiceError::NotFound(message)
                    }
                    _ => ServiceError::from(error),
                })?;
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
                    ))))
                }
            };
            let phase = stored
                .last()
                .map(|event| phase_of_storage_event(&event.event))
                .unwrap_or(Phase::Idle);
            let log = EventLog::open(&session_id_owned).map_err(ServiceError::from)?;
            Ok((working_dir, phase, log))
        })
        .await?;

        let state = Arc::new(SessionState::new(
            working_dir,
            phase,
            Arc::new(SessionWriter::new(log)),
        ));
        self.sessions.insert(session_id.to_string(), state.clone());
        Ok(state)
    }
}

pub(super) fn normalize_session_id(session_id: &str) -> String {
    session_id
        .strip_prefix("session-")
        .unwrap_or(session_id)
        .trim()
        .to_string()
}

pub(super) fn normalize_working_dir(working_dir: PathBuf) -> ServiceResult<PathBuf> {
    let path = if working_dir.is_absolute() {
        working_dir
    } else {
        std::env::current_dir()
            .map_err(|error| {
                ServiceError::Internal(AstrError::io("failed to get current directory", error))
            })?
            .join(working_dir)
    };

    let metadata = std::fs::metadata(&path).map_err(|error| {
        ServiceError::InvalidInput(format!(
            "workingDir '{}' is invalid: {}",
            path.display(),
            error
        ))
    })?;
    if !metadata.is_dir() {
        return Err(ServiceError::InvalidInput(format!(
            "workingDir '{}' is not a directory",
            path.display()
        )));
    }

    std::fs::canonicalize(&path)
        .map_err(|e| {
            AstrError::io(
                format!("failed to canonicalize workingDir '{}'", path.display()),
                e,
            )
        })
        .map_err(ServiceError::from)
}

pub(super) fn display_name_from_working_dir(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("默认项目")
        .to_string()
}

pub(super) async fn load_events(session_id: &str) -> ServiceResult<Vec<StoredEvent>> {
    let session_id = session_id.to_string();
    spawn_blocking_service("load session events", move || {
        EventLog::load(&session_id).map_err(ServiceError::from)
    })
    .await
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::test_support::TestEnvGuard;
    use crate::tool_registry::ToolRegistry;

    use super::*;

    #[tokio::test]
    async fn ensure_session_loaded_reuses_single_state_under_concurrency() {
        let _guard = TestEnvGuard::new();
        let service = Arc::new(AgentService::new(ToolRegistry::builder().build()).unwrap());
        let temp_dir = tempfile::tempdir().expect("tempdir should be created");
        let meta = service
            .create_session(temp_dir.path())
            .await
            .expect("session should be created");
        service.sessions.remove(&meta.session_id);

        let mut handles = Vec::new();
        for _ in 0..8 {
            let service = service.clone();
            let session_id = meta.session_id.clone();
            handles.push(tokio::spawn(async move {
                service
                    .ensure_session_loaded(&session_id)
                    .await
                    .expect("session should load")
            }));
        }

        let states = futures_util::future::join_all(handles)
            .await
            .into_iter()
            .map(|result| result.expect("task should join"))
            .collect::<Vec<_>>();

        let first = Arc::as_ptr(&states[0]);
        assert!(states
            .iter()
            .all(|state| std::ptr::eq(Arc::as_ptr(state), first)));
        assert_eq!(service.sessions.len(), 1);
    }
}
