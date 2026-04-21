//! 会话查询视图。
//!
//! 这些类型表达的是 session-runtime 对外提供的只读快照，
//! 让 `application` 只消费稳定视图，不再自己拼装会话真相。
//! 异步等待 turn 终态的 watcher 归 `turn/` 拥有，`query/` 只保留纯读 / replay / snapshot 语义。

mod agent;
mod conversation;
mod input_queue;
mod replay;
mod service;
mod terminal;
mod text;
mod transcript;
pub(crate) mod turn;

pub use agent::AgentObserveSnapshot;
pub use conversation::{
    ConversationAssistantBlockFacts, ConversationBlockFacts, ConversationBlockPatchFacts,
    ConversationBlockStatus, ConversationChildHandoffBlockFacts, ConversationChildHandoffKind,
    ConversationDeltaFacts, ConversationDeltaFrameFacts, ConversationDeltaProjector,
    ConversationErrorBlockFacts, ConversationPlanBlockFacts, ConversationPlanBlockersFacts,
    ConversationPlanEventKind, ConversationPlanReviewFacts, ConversationPlanReviewKind,
    ConversationPromptMetricsBlockFacts, ConversationSnapshotFacts, ConversationStepCursorFacts,
    ConversationStepProgressFacts, ConversationStreamProjector, ConversationStreamReplayFacts,
    ConversationSystemNoteBlockFacts, ConversationSystemNoteKind, ConversationThinkingBlockFacts,
    ConversationTranscriptErrorKind, ConversationUserBlockFacts, ToolCallBlockFacts,
    ToolCallStreamsFacts,
};
pub use input_queue::recoverable_parent_deliveries;
pub(crate) use service::SessionQueries;
pub use terminal::{LastCompactMetaSnapshot, SessionControlStateSnapshot, SessionModeSnapshot};
pub use transcript::{SessionReplay, SessionTranscriptSnapshot};
pub use turn::{ProjectedTurnOutcome, TurnTerminalSnapshot};
