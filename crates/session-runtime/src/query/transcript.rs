//! 只读 transcript 查询与会话快照类型。
//!
//! Why: 这里集中表达“从单 session 真相里能读到什么 transcript/快照”，
//! 避免把这类只读投影继续塞回 `factory` 或 `application`。

use astrcode_core::{AgentEvent, Phase, SessionEventRecord};
use tokio::sync::broadcast;

#[derive(Debug)]
pub struct SessionReplay {
    pub history: Vec<SessionEventRecord>,
    pub receiver: broadcast::Receiver<SessionEventRecord>,
    pub live_receiver: broadcast::Receiver<AgentEvent>,
}

#[derive(Debug, Clone)]
pub struct SessionTranscriptSnapshot {
    pub records: Vec<SessionEventRecord>,
    pub cursor: Option<String>,
    pub phase: Phase,
}

#[cfg(test)]
mod tests {
    use astrcode_core::SessionId;

    use crate::actor::SessionActor;

    #[test]
    fn current_turn_messages_returns_projected_messages() {
        let actor = SessionActor::new_idle(
            SessionId::from("session-1".to_string()),
            ".",
            "root-agent".into(),
        );

        let messages = actor
            .state()
            .current_turn_messages()
            .expect("projection should succeed");
        assert!(messages.is_empty());
    }
}
