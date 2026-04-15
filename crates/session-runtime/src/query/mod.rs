//! 会话查询视图。
//!
//! 这些类型表达的是 session-runtime 对外提供的只读快照，
//! 让 `application` 只消费稳定视图，不再自己拼装会话真相。

pub mod agent;
pub mod mailbox;
mod service;
pub mod terminal;
pub mod transcript;
pub mod turn;

pub use agent::{AgentObserveSnapshot, build_agent_observe_snapshot};
pub use mailbox::recoverable_parent_deliveries;
pub use service::SessionQueries;
pub use terminal::SessionControlStateSnapshot;
pub use transcript::{SessionReplay, SessionTranscriptSnapshot, current_turn_messages};
pub use turn::{
    ProjectedTurnOutcome, TurnTerminalSnapshot, has_terminal_turn_signal, project_turn_outcome,
};
