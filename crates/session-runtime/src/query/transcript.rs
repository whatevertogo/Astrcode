//! 只读 transcript 查询与会话快照类型。
//!
//! Why: 这里集中表达“从单 session 真相里能读到什么 transcript/快照”，
//! 避免把这类只读投影继续塞回 `factory` 或 `application`。

use astrcode_core::{AgentEvent, LlmMessage, Phase, Result, SessionEventRecord};
use tokio::sync::broadcast;

use crate::SessionState;

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

pub fn current_turn_messages(session: &SessionState) -> Result<Vec<LlmMessage>> {
    Ok(session.snapshot_projected_state()?.messages)
}

#[cfg(test)]
mod tests {
    use astrcode_core::SessionId;

    use super::current_turn_messages;
    use crate::actor::SessionActor;

    #[test]
    fn current_turn_messages_returns_projected_messages() {
        let actor = SessionActor::new_idle(
            SessionId::from("session-1".to_string()),
            ".",
            "root-agent".into(),
        );

        let messages = current_turn_messages(actor.state()).expect("projection should succeed");
        assert!(messages.is_empty());
    }
}
