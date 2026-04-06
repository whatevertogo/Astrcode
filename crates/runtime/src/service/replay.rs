//! # 会话回放 (Session Replay)
//!
//! 实现 `SessionReplaySource` trait，为 SSE 客户端提供会话历史回放和实时订阅。

use async_trait::async_trait;

use super::{RuntimeService, ServiceResult, SessionReplay, SessionReplaySource};

#[async_trait]
impl SessionReplaySource for RuntimeService {
    async fn replay(
        &self,
        session_id: &str,
        last_event_id: Option<&str>,
    ) -> ServiceResult<SessionReplay> {
        self.execution_service()
            .replay(session_id, last_event_id)
            .await
    }
}
