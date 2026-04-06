use std::path::PathBuf;

use astrcode_core::{
    AgentStateProjector, Phase, SessionMeta, StorageEvent, generate_session_id,
    phase_of_storage_event, replay_records,
};
use astrcode_runtime_session::{
    SessionState, SessionWriter, display_name_from_working_dir, normalize_working_dir,
};
use chrono::Utc;

use super::super::blocking_bridge::spawn_blocking_service;
use crate::service::{RuntimeService, ServiceError, ServiceResult, SessionCatalogEvent};

impl RuntimeService {
    pub async fn list_sessions_with_meta(&self) -> ServiceResult<Vec<SessionMeta>> {
        let session_manager = std::sync::Arc::clone(&self.session_manager);
        spawn_blocking_service("list sessions with metadata", move || {
            session_manager
                .list_sessions_with_meta()
                .map_err(ServiceError::from)
        })
        .await
    }

    pub async fn create_session(
        &self,
        working_dir: impl Into<PathBuf>,
    ) -> ServiceResult<SessionMeta> {
        let working_dir = working_dir.into();
        let session_manager = std::sync::Arc::clone(&self.session_manager);
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
        let state = std::sync::Arc::new(SessionState::new(
            phase,
            std::sync::Arc::new(SessionWriter::new(log)),
            AgentStateProjector::from_events(std::slice::from_ref(&stored_session_start.event)),
            replay_records(std::slice::from_ref(&stored_session_start), None),
            vec![stored_session_start.clone()],
        ));
        self.sessions.insert(session_id.clone(), state);

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

        self.emit_session_catalog_event(SessionCatalogEvent::SessionCreated {
            session_id: meta.session_id.clone(),
        });

        Ok(meta)
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::project::project_dir_name;

    use super::*;
    use crate::test_support::{TestEnvGuard, empty_capabilities};

    #[tokio::test]
    async fn create_session_persists_into_project_bucket_directory() {
        let guard = TestEnvGuard::new();
        let service = RuntimeService::from_capabilities(empty_capabilities()).unwrap();
        let temp_dir = tempfile::tempdir().expect("tempdir should be created");

        let meta = service
            .create_session(temp_dir.path())
            .await
            .expect("session should be created");

        let projects_root = guard.home_dir().join(".astrcode").join("projects");
        assert!(
            !guard
                .home_dir()
                .join(".astrcode")
                .join("sessions")
                .join(format!("session-{}.jsonl", meta.session_id))
                .exists(),
            "new layout should avoid writing fresh sessions back into the legacy flat root"
        );

        let bucket_dir = projects_root
            .join(project_dir_name(temp_dir.path()))
            .join("sessions");
        let session_dir = bucket_dir.join(&meta.session_id);
        assert!(
            session_dir
                .join(format!("session-{}.jsonl", meta.session_id))
                .exists(),
            "session file should be nested under a per-session directory inside the project bucket"
        );
    }
}
