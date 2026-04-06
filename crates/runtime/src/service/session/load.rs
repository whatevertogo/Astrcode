use std::sync::Arc;

use astrcode_core::StoredEvent;
use astrcode_runtime_session::SessionState;

use super::super::blocking_bridge::spawn_blocking_service;
use crate::service::{RuntimeService, ServiceError, ServiceResult, SessionHistorySnapshot};

impl RuntimeService {
    pub async fn load_session_history(
        &self,
        session_id: &str,
    ) -> ServiceResult<SessionHistorySnapshot> {
        self.session_service()
            .load_session_history(session_id)
            .await
    }

    /// 确保会话只在首次访问时从磁盘重建一次，避免并发加载把事件广播拆成多份状态。
    pub(crate) async fn ensure_session_loaded(
        &self,
        session_id: &str,
    ) -> ServiceResult<Arc<SessionState>> {
        self.session_service()
            .ensure_session_loaded(session_id)
            .await
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::test_support::{TestEnvGuard, empty_capabilities};

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
        assert!(
            states
                .iter()
                .all(|state| std::ptr::eq(Arc::as_ptr(state), first))
        );
        assert_eq!(service.sessions.len(), 1);
    }
}
