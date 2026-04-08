//! 历史回放：从会话状态缓存或磁盘加载事件记录。

use std::{sync::Arc, time::Instant};

use astrcode_core::replay_records;
use astrcode_runtime_session::normalize_session_id;
use async_trait::async_trait;

use crate::service::{
    ReplayPath, ServiceResult, SessionReplay, SessionReplaySource,
    session::{SessionServiceHandle, load_events},
};

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
            None => load_events(Arc::clone(&self.runtime.session_manager), &session_id)
                .await
                .map(|events| {
                    (
                        replay_records(&events, last_event_id),
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
