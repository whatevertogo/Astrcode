use std::{path::PathBuf, sync::Arc, time::Instant};

use astrcode_core::{
    AgentStateProjector, AstrError, Phase, StorageEvent, StoredEvent, phase_of_storage_event,
    replay_records,
};
use astrcode_runtime_session::{SessionState, SessionWriter, normalize_session_id};

use super::super::blocking_bridge::spawn_blocking_service;
use crate::service::{
    RuntimeService, ServiceError, ServiceResult, SessionHistorySnapshot, SessionMessage,
    SessionReplaySource, replay::convert_events_to_messages,
};

impl RuntimeService {
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

    pub async fn load_session_history(
        &self,
        session_id: &str,
    ) -> ServiceResult<SessionHistorySnapshot> {
        let session_id = normalize_session_id(session_id);
        let state = self.ensure_session_loaded(&session_id).await?;
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

    /// 确保会话只在首次访问时从磁盘重建一次，避免并发加载把事件广播拆成多份状态。
    pub(crate) async fn ensure_session_loaded(
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
