//! 会话查询视图。
//!
//! 这些类型表达的是 session-runtime 对外提供的只读快照，
//! 让 `application` 只消费稳定视图，不再自己拼装会话真相。

mod agent;
mod conversation;
mod input_queue;
mod service;
mod terminal;
mod text;
mod transcript;
mod turn;

pub use agent::AgentObserveSnapshot;
pub use conversation::{
    ConversationAssistantBlockFacts, ConversationBlockFacts, ConversationBlockPatchFacts,
    ConversationBlockStatus, ConversationChildHandoffBlockFacts, ConversationChildHandoffKind,
    ConversationDeltaFacts, ConversationDeltaFrameFacts, ConversationDeltaProjector,
    ConversationErrorBlockFacts, ConversationSnapshotFacts, ConversationStreamProjector,
    ConversationStreamReplayFacts, ConversationSystemNoteBlockFacts, ConversationSystemNoteKind,
    ConversationThinkingBlockFacts, ConversationTranscriptErrorKind, ConversationUserBlockFacts,
    ToolCallBlockFacts, ToolCallStreamsFacts,
};
pub use input_queue::recoverable_parent_deliveries;
pub(crate) use service::SessionQueries;
pub use terminal::{LastCompactMetaSnapshot, SessionControlStateSnapshot, SessionModeSnapshot};
pub(crate) use transcript::current_turn_messages;
pub use transcript::{SessionReplay, SessionTranscriptSnapshot};
pub use turn::{ProjectedTurnOutcome, TurnTerminalSnapshot};
