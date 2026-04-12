//! 会话查询视图。
//!
//! 这些类型表达的是 session-runtime 对外提供的只读快照，
//! 让 `application` 只消费稳定视图，不再自己拼装会话真相。

use astrcode_core::{AgentEvent, Phase, SessionEventRecord};
use tokio::sync::broadcast;

#[derive(Debug)]
pub struct SessionReplay {
    pub history: Vec<SessionEventRecord>,
    pub receiver: broadcast::Receiver<SessionEventRecord>,
    pub live_receiver: broadcast::Receiver<AgentEvent>,
}

#[derive(Debug, Clone)]
pub struct SessionHistorySnapshot {
    pub history: Vec<SessionEventRecord>,
    pub cursor: Option<String>,
    pub phase: Phase,
}

#[derive(Debug, Clone)]
pub struct SessionViewSnapshot {
    pub focus_history: Vec<SessionEventRecord>,
    pub direct_children_history: Vec<SessionEventRecord>,
    pub cursor: Option<String>,
    pub phase: Phase,
}
