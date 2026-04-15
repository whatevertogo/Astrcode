//! Conversation v1 HTTP / SSE DTO。
//!
//! 当前 authoritative conversation read model 线缆形状与既有 terminal surface
//! 保持一致，但协议职责已经提升为跨 Web/Desktop/TUI 的统一读取面。

pub use crate::http::terminal::v1::{
    TerminalAssistantBlockDto as ConversationAssistantBlockDto,
    TerminalBannerDto as ConversationBannerDto,
    TerminalBannerErrorCodeDto as ConversationBannerErrorCodeDto,
    TerminalBlockDto as ConversationBlockDto, TerminalBlockPatchDto as ConversationBlockPatchDto,
    TerminalBlockStatusDto as ConversationBlockStatusDto,
    TerminalChildHandoffBlockDto as ConversationChildHandoffBlockDto,
    TerminalChildHandoffKindDto as ConversationChildHandoffKindDto,
    TerminalChildSummaryDto as ConversationChildSummaryDto,
    TerminalControlStateDto as ConversationControlStateDto,
    TerminalCursorDto as ConversationCursorDto, TerminalDeltaDto as ConversationDeltaDto,
    TerminalErrorBlockDto as ConversationErrorBlockDto,
    TerminalErrorEnvelopeDto as ConversationErrorEnvelopeDto,
    TerminalSlashActionKindDto as ConversationSlashActionKindDto,
    TerminalSlashCandidateDto as ConversationSlashCandidateDto,
    TerminalSlashCandidatesResponseDto as ConversationSlashCandidatesResponseDto,
    TerminalSnapshotResponseDto as ConversationSnapshotResponseDto,
    TerminalStreamEnvelopeDto as ConversationStreamEnvelopeDto,
    TerminalSystemNoteBlockDto as ConversationSystemNoteBlockDto,
    TerminalSystemNoteKindDto as ConversationSystemNoteKindDto,
    TerminalThinkingBlockDto as ConversationThinkingBlockDto,
    TerminalToolCallBlockDto as ConversationToolCallBlockDto,
    TerminalToolStreamBlockDto as ConversationToolStreamBlockDto,
    TerminalTranscriptErrorCodeDto as ConversationTranscriptErrorCodeDto,
    TerminalUserBlockDto as ConversationUserBlockDto,
};
