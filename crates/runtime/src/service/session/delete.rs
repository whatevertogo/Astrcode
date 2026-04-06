use std::sync::Arc;

use astrcode_core::DeleteProjectResult;
use astrcode_runtime_session::normalize_session_id;

use super::super::blocking_bridge::spawn_blocking_service;
use crate::service::{RuntimeService, ServiceError, ServiceResult, SessionCatalogEvent};

impl RuntimeService {
    pub async fn delete_session(&self, session_id: &str) -> ServiceResult<()> {
        let normalized = normalize_session_id(session_id);
        let _guard = self.session_load_lock.lock().await;
        self.interrupt(&normalized).await?;
        self.sessions.remove(&normalized);
        let session_manager = Arc::clone(&self.session_manager);
        let delete_session_id = normalized.clone();
        spawn_blocking_service("delete session", move || {
            session_manager
                .delete_session(&delete_session_id)
                .map_err(ServiceError::from)
        })
        .await?;
        self.emit_session_catalog_event(SessionCatalogEvent::SessionDeleted {
            session_id: normalized,
        });
        Ok(())
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
        let result = spawn_blocking_service("delete project sessions", move || {
            session_manager
                .delete_sessions_by_working_dir(&delete_working_dir)
                .map_err(ServiceError::from)
        })
        .await?;
        self.emit_session_catalog_event(SessionCatalogEvent::ProjectDeleted { working_dir });
        Ok(result)
    }
}
