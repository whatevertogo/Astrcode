use astrcode_session_runtime::{
    ConversationBlockFacts, ConversationChildHandoffBlockFacts, ConversationErrorBlockFacts,
    ConversationSnapshotFacts, ConversationSystemNoteBlockFacts, ToolCallBlockFacts,
};

use crate::terminal::{latest_transcript_cursor, truncate_terminal_summary};

pub(super) fn latest_terminal_summary(snapshot: &ConversationSnapshotFacts) -> Option<String> {
    snapshot
        .blocks
        .iter()
        .rev()
        .find_map(summary_from_block)
        .or_else(|| latest_transcript_cursor(snapshot).map(|cursor| format!("cursor:{cursor}")))
}

fn summary_from_block(block: &ConversationBlockFacts) -> Option<String> {
    match block {
        ConversationBlockFacts::Assistant(block) => summary_from_markdown(&block.markdown),
        ConversationBlockFacts::ToolCall(block) => summary_from_tool_call(block),
        ConversationBlockFacts::ChildHandoff(block) => summary_from_child_handoff(block),
        ConversationBlockFacts::Error(block) => summary_from_error_block(block),
        ConversationBlockFacts::SystemNote(block) => summary_from_system_note(block),
        ConversationBlockFacts::User(_) | ConversationBlockFacts::Thinking(_) => None,
    }
}

fn summary_from_markdown(markdown: &str) -> Option<String> {
    (!markdown.trim().is_empty()).then(|| truncate_terminal_summary(markdown))
}

fn summary_from_tool_call(block: &ToolCallBlockFacts) -> Option<String> {
    block
        .summary
        .as_deref()
        .filter(|summary| !summary.trim().is_empty())
        .map(truncate_terminal_summary)
        .or_else(|| {
            block
                .error
                .as_deref()
                .filter(|error| !error.trim().is_empty())
                .map(truncate_terminal_summary)
        })
        .or_else(|| summary_from_markdown(&block.streams.stderr))
        .or_else(|| summary_from_markdown(&block.streams.stdout))
}

fn summary_from_child_handoff(block: &ConversationChildHandoffBlockFacts) -> Option<String> {
    block
        .message
        .as_deref()
        .filter(|message| !message.trim().is_empty())
        .map(truncate_terminal_summary)
}

fn summary_from_error_block(block: &ConversationErrorBlockFacts) -> Option<String> {
    summary_from_markdown(&block.message)
}

fn summary_from_system_note(block: &ConversationSystemNoteBlockFacts) -> Option<String> {
    summary_from_markdown(&block.markdown)
}
