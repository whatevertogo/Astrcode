//! 历史回放：从会话状态缓存或磁盘加载事件记录。

use std::{sync::Arc, time::Instant};

use astrcode_core::replay_records;
use astrcode_runtime_session::normalize_session_id;
use async_trait::async_trait;
use futures_util::future::{BoxFuture, FutureExt, Shared};

use crate::service::{
    ReplayPath, ServiceError, ServiceResult, SessionReplay, SessionReplaySource,
    session::{SessionServiceHandle, load_events},
};

pub(crate) type ReplayFallbackFuture = Shared<BoxFuture<'static, ReplayFallbackResult>>;

type ReplayFallbackResult = Result<Arc<Vec<astrcode_core::StoredEvent>>, ReplayFallbackError>;

#[derive(Debug, Clone)]
pub(crate) struct ReplayFallbackError {
    kind: ReplayFallbackErrorKind,
    message: String,
}

#[derive(Debug, Clone, Copy)]
enum ReplayFallbackErrorKind {
    NotFound,
    Conflict,
    InvalidInput,
    Internal,
}

impl SessionServiceHandle {
    /// 回放指定会话的事件历史。
    ///
    /// 优先从内存缓存中读取；缓存不满足时回退到磁盘加载。
    /// 返回的事件记录和 SSE 广播 receiver 供前端消费。
    pub async fn replay(
        &self,
        session_id: &str,
        last_event_id: Option<&str>,
    ) -> ServiceResult<SessionReplay> {
        let session_id = normalize_session_id(session_id);
        let state = self.runtime.ensure_session_loaded(&session_id).await?;

        let receiver = state.broadcaster.subscribe();
        let started_at = Instant::now();
        let replay_result = match state.recent_records_after(last_event_id)? {
            Some(history) => Ok((history, ReplayPath::Cache)),
            None => self
                .load_replay_fallback_once(&session_id)
                .await
                .map(|events| {
                    (
                        replay_records(events.as_ref(), last_event_id),
                        ReplayPath::DiskFallback,
                    )
                }),
        };
        let elapsed = started_at.elapsed();
        match &replay_result {
            Ok((history, path)) => {
                self.runtime.observability.record_sse_catch_up(
                    elapsed,
                    true,
                    path.clone(),
                    history.len(),
                );
                if matches!(path, ReplayPath::DiskFallback) {
                    log::warn!(
                        "session '{}' replay used durable fallback and recovered {} events in {}ms",
                        session_id,
                        history.len(),
                        elapsed.as_millis()
                    );
                }
            },
            Err(error) => {
                self.runtime.observability.record_sse_catch_up(
                    elapsed,
                    false,
                    ReplayPath::DiskFallback,
                    0,
                );
                log::error!(
                    "failed to replay session '{}' after {}ms: {}",
                    session_id,
                    elapsed.as_millis(),
                    error
                );
            },
        }
        let (history, _) = replay_result?;
        Ok(SessionReplay { history, receiver })
    }
}

impl SessionServiceHandle {
    async fn load_replay_fallback_once(
        &self,
        session_id: &str,
    ) -> ServiceResult<Arc<Vec<astrcode_core::StoredEvent>>> {
        if let Some(existing) = self.runtime.replay_fallbacks.get(session_id) {
            return existing
                .clone()
                .await
                .map_err(replay_fallback_error_to_service_error);
        }

        let session_id_owned = session_id.to_string();
        let session_manager = Arc::clone(&self.runtime.session_manager);
        let future = async move {
            load_events(session_manager, &session_id_owned)
                .await
                .map(Arc::new)
                .map_err(service_error_to_replay_fallback_error)
        }
        .boxed()
        .shared();

        let shared = match self.runtime.replay_fallbacks.entry(session_id.to_string()) {
            dashmap::mapref::entry::Entry::Occupied(existing) => existing.get().clone(),
            dashmap::mapref::entry::Entry::Vacant(vacant) => {
                vacant.insert(future.clone());
                future
            },
        };

        let result = shared.await;
        self.runtime.replay_fallbacks.remove(session_id);
        result.map_err(replay_fallback_error_to_service_error)
    }
}

fn service_error_to_replay_fallback_error(error: ServiceError) -> ReplayFallbackError {
    match error {
        ServiceError::NotFound(message) => ReplayFallbackError {
            kind: ReplayFallbackErrorKind::NotFound,
            message,
        },
        ServiceError::Conflict(message) => ReplayFallbackError {
            kind: ReplayFallbackErrorKind::Conflict,
            message,
        },
        ServiceError::InvalidInput(message) => ReplayFallbackError {
            kind: ReplayFallbackErrorKind::InvalidInput,
            message,
        },
        ServiceError::Internal(error) => ReplayFallbackError {
            kind: ReplayFallbackErrorKind::Internal,
            message: error.to_string(),
        },
    }
}

fn replay_fallback_error_to_service_error(error: ReplayFallbackError) -> ServiceError {
    match error.kind {
        ReplayFallbackErrorKind::NotFound => ServiceError::NotFound(error.message),
        ReplayFallbackErrorKind::Conflict => ServiceError::Conflict(error.message),
        ReplayFallbackErrorKind::InvalidInput => ServiceError::InvalidInput(error.message),
        ReplayFallbackErrorKind::Internal => {
            ServiceError::Internal(astrcode_core::AstrError::Internal(error.message))
        },
    }
}

#[async_trait]
impl SessionReplaySource for SessionServiceHandle {
    async fn replay(
        &self,
        session_id: &str,
        last_event_id: Option<&str>,
    ) -> ServiceResult<SessionReplay> {
        SessionServiceHandle::replay(self, session_id, last_event_id).await
    }
}
