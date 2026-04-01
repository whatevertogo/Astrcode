use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use astrcode_core::{AgentStateProjector, AstrError, Phase};
use chrono::Utc;

use astrcode_core::{generate_session_id, DeleteProjectResult, SessionMeta};
use astrcode_core::{StorageEvent, StoredEvent};

use super::replay::convert_events_to_messages;
use super::session_state::{SessionState, SessionWriter};
use super::support::spawn_blocking_service;
use super::{RuntimeService, ServiceError, ServiceResult, SessionMessage};
use astrcode_core::{phase_of_storage_event, replay_records};

impl RuntimeService {
    pub async fn list_sessions_with_meta(&self) -> ServiceResult<Vec<SessionMeta>> {
        let session_manager = Arc::clone(&self.session_manager);
        spawn_blocking_service("list sessions with metadata", move || {
            session_manager
                .list_sessions_with_meta()
                .map_err(ServiceError::from)
        })
        .await
    }

    pub async fn list_sessions(&self) -> ServiceResult<Vec<String>> {
        let session_manager = Arc::clone(&self.session_manager);
        spawn_blocking_service("list sessions", move || {
            session_manager.list_sessions().map_err(ServiceError::from)
        })
        .await
    }

    pub async fn create_session(
        &self,
        working_dir: impl Into<PathBuf>,
    ) -> ServiceResult<SessionMeta> {
        let working_dir = working_dir.into();
        let session_manager = Arc::clone(&self.session_manager);
        let (session_id, working_dir, created_at, log, stored_session_start) =
            spawn_blocking_service("create session", move || {
                let working_dir = normalize_working_dir(working_dir)?;
                let session_id = generate_session_id();
                let mut log = session_manager
                    .create_event_log(&session_id)
                    .map_err(ServiceError::from)?;
                let created_at = Utc::now();
                let session_start = StorageEvent::SessionStart {
                    session_id: session_id.clone(),
                    timestamp: created_at,
                    working_dir: working_dir.to_string_lossy().to_string(),
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
            working_dir.clone(),
            phase,
            Arc::new(SessionWriter::new(log)),
            AgentStateProjector::from_events(std::slice::from_ref(&stored_session_start.event)),
            replay_records(std::slice::from_ref(&stored_session_start), None),
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
        let events = load_events(Arc::clone(&self.session_manager), &session_id).await?;
        let cursor = replay_records(&events, None)
            .last()
            .map(|record| record.event_id.clone());
        Ok((convert_events_to_messages(&events), cursor))
    }

    pub async fn delete_session(&self, session_id: &str) -> ServiceResult<()> {
        let normalized = normalize_session_id(session_id);
        self.interrupt(&normalized).await?;
        self.sessions.remove(&normalized);
        let session_manager = Arc::clone(&self.session_manager);
        spawn_blocking_service("delete session", move || {
            session_manager
                .delete_session(&normalized)
                .map_err(ServiceError::from)
        })
        .await
    }

    pub async fn delete_project(&self, working_dir: &str) -> ServiceResult<DeleteProjectResult> {
        let working_dir = working_dir.to_string();
        let session_manager = Arc::clone(&self.session_manager);
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
            let _ = self.interrupt(session_id).await;
            self.sessions.remove(session_id);
        }

        let delete_working_dir = working_dir.clone();
        let session_manager = Arc::clone(&self.session_manager);
        spawn_blocking_service("delete project sessions", move || {
            session_manager
                .delete_sessions_by_working_dir(&delete_working_dir)
                .map_err(ServiceError::from)
        })
        .await
    }

    /// 确保会话已加载到内存中，使用双重检查锁定避免重复加载。
    ///
    /// ## 为什么需要锁
    ///
    /// 多个并发请求可能同时请求同一个 session_id（如多个 SSE 客户端连接）。
    /// 如果不加锁，会导致同一个会话被从磁盘加载两次，创建两个不同的
    /// `SessionState` 和 broadcast channel，后续事件会分散到不同 channel。
    ///
    /// 使用 `session_load_lock` 保证只有一个请求执行实际的磁盘加载，
    /// 其他请求等待锁释放后直接从 `sessions` map 中获取已加载的状态。
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
            let log = session_manager
                .open_event_log(&session_id_owned)
                .map_err(ServiceError::from)?;
            let events = stored
                .iter()
                .map(|record| record.event.clone())
                .collect::<Vec<_>>();
            let projector = AgentStateProjector::from_events(&events);
            let recent_records = replay_records(&stored, None);
            Ok((working_dir, phase, log, projector, recent_records))
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
            }
            Err(error) => {
                self.observability.record_session_rehydrate(elapsed, false);
                log::error!(
                    "failed to rehydrate session '{}' after {}ms: {}",
                    session_id,
                    elapsed.as_millis(),
                    error
                );
            }
        }
        let (working_dir, phase, log, projector, recent_records) = load_result?;

        let state = Arc::new(SessionState::new(
            working_dir,
            phase,
            Arc::new(SessionWriter::new(log)),
            projector,
            recent_records,
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

pub(super) async fn load_events(
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

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use crate::test_support::{empty_capabilities, TestEnvGuard};

    use super::*;

    #[tokio::test]
    async fn ensure_session_loaded_reuses_single_state_under_concurrency() {
        let _guard = TestEnvGuard::new();
        let service = Arc::new(RuntimeService::from_capabilities(empty_capabilities()).unwrap());
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

    #[test]
    fn normalize_session_id_keeps_legacy_inner_prefix() {
        assert_eq!(
            normalize_session_id("session-session-2026-03-08T10-00-00-aaaaaaaa"),
            "session-2026-03-08T10-00-00-aaaaaaaa"
        );
    }

    #[test]
    fn normalize_working_dir_rejects_file_paths() {
        let temp_dir = tempfile::tempdir().expect("tempdir should be created");
        let file = temp_dir.path().join("file.txt");
        std::fs::write(&file, "demo").expect("file should be created");

        let err =
            normalize_working_dir(file).expect_err("file paths should not be accepted as workdir");

        assert!(matches!(err, ServiceError::InvalidInput(_)));
        assert!(err.to_string().contains("is not a directory"));
    }

    #[test]
    fn normalize_working_dir_rejects_missing_paths() {
        let temp_dir = tempfile::tempdir().expect("tempdir should be created");
        let missing = temp_dir.path().join("missing");

        let err = normalize_working_dir(missing).expect_err("missing workdir should fail");

        assert!(matches!(err, ServiceError::InvalidInput(_)));
        assert!(err.to_string().contains("is invalid"));
    }

    #[test]
    fn display_name_from_working_dir_uses_default_for_root() {
        #[cfg(windows)]
        let root = Path::new(r"C:\");
        #[cfg(not(windows))]
        let root = Path::new("/");

        assert_eq!(display_name_from_working_dir(root), "默认项目");
    }

    #[test]
    fn display_name_from_working_dir_ignores_trailing_separator() {
        let temp_dir = tempfile::tempdir().expect("tempdir should be created");
        let rendered = format!("{}{}", temp_dir.path().display(), std::path::MAIN_SEPARATOR);

        assert_eq!(
            display_name_from_working_dir(Path::new(&rendered)),
            temp_dir
                .path()
                .file_name()
                .and_then(|name| name.to_str())
                .expect("tempdir name should be utf-8")
        );
    }
}
