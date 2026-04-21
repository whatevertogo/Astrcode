use astrcode_session_runtime as runtime;
use tokio::sync::broadcast;

use super::contracts::{
    ConversationAssistantBlockFacts, ConversationBlockFacts, ConversationBlockPatchFacts,
    ConversationBlockStatus, ConversationChildHandoffBlockFacts, ConversationChildHandoffKind,
    ConversationDeltaFacts, ConversationDeltaFrameFacts, ConversationErrorBlockFacts,
    ConversationPlanBlockFacts, ConversationPlanBlockersFacts, ConversationPlanEventKind,
    ConversationPlanReviewFacts, ConversationPlanReviewKind, ConversationSnapshotFacts,
    ConversationStreamReplayFacts, ConversationSystemNoteBlockFacts, ConversationSystemNoteKind,
    ConversationThinkingBlockFacts, ConversationTranscriptErrorKind, ConversationUserBlockFacts,
    ToolCallBlockFacts, ToolCallStreamsFacts,
};
use crate::SessionReplay;

pub(crate) struct MappedConversationStreamReplay {
    pub replay: ConversationStreamReplayFacts,
    pub stream: SessionReplay,
}

pub(crate) fn map_snapshot(facts: runtime::ConversationSnapshotFacts) -> ConversationSnapshotFacts {
    ConversationSnapshotFacts {
        cursor: facts.cursor,
        phase: facts.phase,
        blocks: facts.blocks.into_iter().map(map_block).collect(),
    }
}

pub(crate) fn map_stream_replay(
    facts: runtime::ConversationStreamReplayFacts,
) -> MappedConversationStreamReplay {
    let runtime::ConversationStreamReplayFacts {
        cursor,
        phase,
        seed_records,
        replay_frames,
        replay,
    } = facts;
    let history = replay.history.clone();

    MappedConversationStreamReplay {
        replay: ConversationStreamReplayFacts {
            cursor,
            phase,
            seed_records,
            replay_frames: replay_frames.into_iter().map(map_frame).collect(),
            history,
        },
        stream: replay,
    }
}

pub(crate) fn into_runtime_stream_replay(
    facts: &ConversationStreamReplayFacts,
) -> runtime::ConversationStreamReplayFacts {
    let (_durable_tx, receiver) = broadcast::channel(1);
    let (_live_tx, live_receiver) = broadcast::channel(1);

    runtime::ConversationStreamReplayFacts {
        cursor: facts.cursor.clone(),
        phase: facts.phase,
        seed_records: facts.seed_records.clone(),
        replay_frames: facts
            .replay_frames
            .iter()
            .cloned()
            .map(into_runtime_frame)
            .collect(),
        replay: runtime::SessionReplay {
            history: facts.history.clone(),
            receiver,
            live_receiver,
        },
    }
}

pub(crate) fn map_frame(
    frame: runtime::ConversationDeltaFrameFacts,
) -> ConversationDeltaFrameFacts {
    ConversationDeltaFrameFacts {
        cursor: frame.cursor,
        delta: map_delta(frame.delta),
    }
}

pub(crate) fn map_delta(delta: runtime::ConversationDeltaFacts) -> ConversationDeltaFacts {
    match delta {
        runtime::ConversationDeltaFacts::AppendBlock { block } => {
            ConversationDeltaFacts::AppendBlock {
                block: Box::new(map_block(*block)),
            }
        },
        runtime::ConversationDeltaFacts::PatchBlock { block_id, patch } => {
            ConversationDeltaFacts::PatchBlock {
                block_id,
                patch: map_patch(patch),
            }
        },
        runtime::ConversationDeltaFacts::CompleteBlock { block_id, status } => {
            ConversationDeltaFacts::CompleteBlock {
                block_id,
                status: map_block_status(status),
            }
        },
    }
}

fn map_patch(patch: runtime::ConversationBlockPatchFacts) -> ConversationBlockPatchFacts {
    match patch {
        runtime::ConversationBlockPatchFacts::AppendMarkdown { markdown } => {
            ConversationBlockPatchFacts::AppendMarkdown { markdown }
        },
        runtime::ConversationBlockPatchFacts::ReplaceMarkdown { markdown } => {
            ConversationBlockPatchFacts::ReplaceMarkdown { markdown }
        },
        runtime::ConversationBlockPatchFacts::AppendToolStream { stream, chunk } => {
            ConversationBlockPatchFacts::AppendToolStream { stream, chunk }
        },
        runtime::ConversationBlockPatchFacts::ReplaceSummary { summary } => {
            ConversationBlockPatchFacts::ReplaceSummary { summary }
        },
        runtime::ConversationBlockPatchFacts::ReplaceMetadata { metadata } => {
            ConversationBlockPatchFacts::ReplaceMetadata { metadata }
        },
        runtime::ConversationBlockPatchFacts::ReplaceError { error } => {
            ConversationBlockPatchFacts::ReplaceError { error }
        },
        runtime::ConversationBlockPatchFacts::ReplaceDuration { duration_ms } => {
            ConversationBlockPatchFacts::ReplaceDuration { duration_ms }
        },
        runtime::ConversationBlockPatchFacts::ReplaceChildRef { child_ref } => {
            ConversationBlockPatchFacts::ReplaceChildRef { child_ref }
        },
        runtime::ConversationBlockPatchFacts::SetTruncated { truncated } => {
            ConversationBlockPatchFacts::SetTruncated { truncated }
        },
        runtime::ConversationBlockPatchFacts::SetStatus { status } => {
            ConversationBlockPatchFacts::SetStatus {
                status: map_block_status(status),
            }
        },
    }
}

fn map_block(block: runtime::ConversationBlockFacts) -> ConversationBlockFacts {
    match block {
        runtime::ConversationBlockFacts::User(block) => {
            ConversationBlockFacts::User(ConversationUserBlockFacts {
                id: block.id,
                turn_id: block.turn_id,
                markdown: block.markdown,
            })
        },
        runtime::ConversationBlockFacts::Assistant(block) => {
            ConversationBlockFacts::Assistant(ConversationAssistantBlockFacts {
                id: block.id,
                turn_id: block.turn_id,
                status: map_block_status(block.status),
                markdown: block.markdown,
            })
        },
        runtime::ConversationBlockFacts::Thinking(block) => {
            ConversationBlockFacts::Thinking(ConversationThinkingBlockFacts {
                id: block.id,
                turn_id: block.turn_id,
                status: map_block_status(block.status),
                markdown: block.markdown,
            })
        },
        runtime::ConversationBlockFacts::Plan(block) => {
            ConversationBlockFacts::Plan(Box::new(ConversationPlanBlockFacts {
                id: block.id,
                turn_id: block.turn_id,
                tool_call_id: block.tool_call_id,
                event_kind: map_plan_event_kind(block.event_kind),
                title: block.title,
                plan_path: block.plan_path,
                summary: block.summary,
                status: block.status,
                slug: block.slug,
                updated_at: block.updated_at,
                content: block.content,
                review: block.review.map(|review| ConversationPlanReviewFacts {
                    kind: map_plan_review_kind(review.kind),
                    checklist: review.checklist,
                }),
                blockers: ConversationPlanBlockersFacts {
                    missing_headings: block.blockers.missing_headings,
                    invalid_sections: block.blockers.invalid_sections,
                },
            }))
        },
        runtime::ConversationBlockFacts::ToolCall(block) => {
            ConversationBlockFacts::ToolCall(Box::new(ToolCallBlockFacts {
                id: block.id,
                turn_id: block.turn_id,
                tool_call_id: block.tool_call_id,
                tool_name: block.tool_name,
                status: map_block_status(block.status),
                input: block.input,
                summary: block.summary,
                error: block.error,
                duration_ms: block.duration_ms,
                truncated: block.truncated,
                metadata: block.metadata,
                child_ref: block.child_ref,
                streams: ToolCallStreamsFacts {
                    stdout: block.streams.stdout,
                    stderr: block.streams.stderr,
                },
            }))
        },
        runtime::ConversationBlockFacts::Error(block) => {
            ConversationBlockFacts::Error(ConversationErrorBlockFacts {
                id: block.id,
                turn_id: block.turn_id,
                code: map_transcript_error_kind(block.code),
                message: block.message,
            })
        },
        runtime::ConversationBlockFacts::SystemNote(block) => {
            ConversationBlockFacts::SystemNote(ConversationSystemNoteBlockFacts {
                id: block.id,
                note_kind: map_system_note_kind(block.note_kind),
                markdown: block.markdown,
                compact_trigger: block.compact_trigger,
                compact_meta: block.compact_meta,
                compact_preserved_recent_turns: block.compact_preserved_recent_turns,
            })
        },
        runtime::ConversationBlockFacts::ChildHandoff(block) => {
            ConversationBlockFacts::ChildHandoff(ConversationChildHandoffBlockFacts {
                id: block.id,
                handoff_kind: map_child_handoff_kind(block.handoff_kind),
                child_ref: block.child_ref,
                message: block.message,
            })
        },
    }
}

fn into_runtime_frame(frame: ConversationDeltaFrameFacts) -> runtime::ConversationDeltaFrameFacts {
    runtime::ConversationDeltaFrameFacts {
        cursor: frame.cursor,
        delta: into_runtime_delta(frame.delta),
    }
}

fn into_runtime_delta(delta: ConversationDeltaFacts) -> runtime::ConversationDeltaFacts {
    match delta {
        ConversationDeltaFacts::AppendBlock { block } => {
            runtime::ConversationDeltaFacts::AppendBlock {
                block: Box::new(into_runtime_block(*block)),
            }
        },
        ConversationDeltaFacts::PatchBlock { block_id, patch } => {
            runtime::ConversationDeltaFacts::PatchBlock {
                block_id,
                patch: into_runtime_patch(patch),
            }
        },
        ConversationDeltaFacts::CompleteBlock { block_id, status } => {
            runtime::ConversationDeltaFacts::CompleteBlock {
                block_id,
                status: into_runtime_block_status(status),
            }
        },
    }
}

fn into_runtime_patch(patch: ConversationBlockPatchFacts) -> runtime::ConversationBlockPatchFacts {
    match patch {
        ConversationBlockPatchFacts::AppendMarkdown { markdown } => {
            runtime::ConversationBlockPatchFacts::AppendMarkdown { markdown }
        },
        ConversationBlockPatchFacts::ReplaceMarkdown { markdown } => {
            runtime::ConversationBlockPatchFacts::ReplaceMarkdown { markdown }
        },
        ConversationBlockPatchFacts::AppendToolStream { stream, chunk } => {
            runtime::ConversationBlockPatchFacts::AppendToolStream { stream, chunk }
        },
        ConversationBlockPatchFacts::ReplaceSummary { summary } => {
            runtime::ConversationBlockPatchFacts::ReplaceSummary { summary }
        },
        ConversationBlockPatchFacts::ReplaceMetadata { metadata } => {
            runtime::ConversationBlockPatchFacts::ReplaceMetadata { metadata }
        },
        ConversationBlockPatchFacts::ReplaceError { error } => {
            runtime::ConversationBlockPatchFacts::ReplaceError { error }
        },
        ConversationBlockPatchFacts::ReplaceDuration { duration_ms } => {
            runtime::ConversationBlockPatchFacts::ReplaceDuration { duration_ms }
        },
        ConversationBlockPatchFacts::ReplaceChildRef { child_ref } => {
            runtime::ConversationBlockPatchFacts::ReplaceChildRef { child_ref }
        },
        ConversationBlockPatchFacts::SetTruncated { truncated } => {
            runtime::ConversationBlockPatchFacts::SetTruncated { truncated }
        },
        ConversationBlockPatchFacts::SetStatus { status } => {
            runtime::ConversationBlockPatchFacts::SetStatus {
                status: into_runtime_block_status(status),
            }
        },
    }
}

fn into_runtime_block(block: ConversationBlockFacts) -> runtime::ConversationBlockFacts {
    match block {
        ConversationBlockFacts::User(block) => {
            runtime::ConversationBlockFacts::User(runtime::ConversationUserBlockFacts {
                id: block.id,
                turn_id: block.turn_id,
                markdown: block.markdown,
            })
        },
        ConversationBlockFacts::Assistant(block) => {
            runtime::ConversationBlockFacts::Assistant(runtime::ConversationAssistantBlockFacts {
                id: block.id,
                turn_id: block.turn_id,
                status: into_runtime_block_status(block.status),
                markdown: block.markdown,
            })
        },
        ConversationBlockFacts::Thinking(block) => {
            runtime::ConversationBlockFacts::Thinking(runtime::ConversationThinkingBlockFacts {
                id: block.id,
                turn_id: block.turn_id,
                status: into_runtime_block_status(block.status),
                markdown: block.markdown,
            })
        },
        ConversationBlockFacts::Plan(block) => {
            runtime::ConversationBlockFacts::Plan(Box::new(runtime::ConversationPlanBlockFacts {
                id: block.id,
                turn_id: block.turn_id,
                tool_call_id: block.tool_call_id,
                event_kind: into_runtime_plan_event_kind(block.event_kind),
                title: block.title,
                plan_path: block.plan_path,
                summary: block.summary,
                status: block.status,
                slug: block.slug,
                updated_at: block.updated_at,
                content: block.content,
                review: block
                    .review
                    .map(|review| runtime::ConversationPlanReviewFacts {
                        kind: into_runtime_plan_review_kind(review.kind),
                        checklist: review.checklist,
                    }),
                blockers: runtime::ConversationPlanBlockersFacts {
                    missing_headings: block.blockers.missing_headings,
                    invalid_sections: block.blockers.invalid_sections,
                },
            }))
        },
        ConversationBlockFacts::ToolCall(block) => {
            runtime::ConversationBlockFacts::ToolCall(Box::new(runtime::ToolCallBlockFacts {
                id: block.id,
                turn_id: block.turn_id,
                tool_call_id: block.tool_call_id,
                tool_name: block.tool_name,
                status: into_runtime_block_status(block.status),
                input: block.input,
                summary: block.summary,
                error: block.error,
                duration_ms: block.duration_ms,
                truncated: block.truncated,
                metadata: block.metadata,
                child_ref: block.child_ref,
                streams: runtime::ToolCallStreamsFacts {
                    stdout: block.streams.stdout,
                    stderr: block.streams.stderr,
                },
            }))
        },
        ConversationBlockFacts::Error(block) => {
            runtime::ConversationBlockFacts::Error(runtime::ConversationErrorBlockFacts {
                id: block.id,
                turn_id: block.turn_id,
                code: into_runtime_transcript_error_kind(block.code),
                message: block.message,
            })
        },
        ConversationBlockFacts::SystemNote(block) => {
            runtime::ConversationBlockFacts::SystemNote(runtime::ConversationSystemNoteBlockFacts {
                id: block.id,
                note_kind: into_runtime_system_note_kind(block.note_kind),
                markdown: block.markdown,
                compact_trigger: block.compact_trigger,
                compact_meta: block.compact_meta,
                compact_preserved_recent_turns: block.compact_preserved_recent_turns,
            })
        },
        ConversationBlockFacts::ChildHandoff(block) => {
            runtime::ConversationBlockFacts::ChildHandoff(
                runtime::ConversationChildHandoffBlockFacts {
                    id: block.id,
                    handoff_kind: into_runtime_child_handoff_kind(block.handoff_kind),
                    child_ref: block.child_ref,
                    message: block.message,
                },
            )
        },
    }
}

fn map_block_status(status: runtime::ConversationBlockStatus) -> ConversationBlockStatus {
    match status {
        runtime::ConversationBlockStatus::Streaming => ConversationBlockStatus::Streaming,
        runtime::ConversationBlockStatus::Complete => ConversationBlockStatus::Complete,
        runtime::ConversationBlockStatus::Failed => ConversationBlockStatus::Failed,
        runtime::ConversationBlockStatus::Cancelled => ConversationBlockStatus::Cancelled,
    }
}

fn into_runtime_block_status(status: ConversationBlockStatus) -> runtime::ConversationBlockStatus {
    match status {
        ConversationBlockStatus::Streaming => runtime::ConversationBlockStatus::Streaming,
        ConversationBlockStatus::Complete => runtime::ConversationBlockStatus::Complete,
        ConversationBlockStatus::Failed => runtime::ConversationBlockStatus::Failed,
        ConversationBlockStatus::Cancelled => runtime::ConversationBlockStatus::Cancelled,
    }
}

fn map_system_note_kind(kind: runtime::ConversationSystemNoteKind) -> ConversationSystemNoteKind {
    match kind {
        runtime::ConversationSystemNoteKind::Compact => ConversationSystemNoteKind::Compact,
        runtime::ConversationSystemNoteKind::SystemNote => ConversationSystemNoteKind::SystemNote,
    }
}

fn into_runtime_system_note_kind(
    kind: ConversationSystemNoteKind,
) -> runtime::ConversationSystemNoteKind {
    match kind {
        ConversationSystemNoteKind::Compact => runtime::ConversationSystemNoteKind::Compact,
        ConversationSystemNoteKind::SystemNote => runtime::ConversationSystemNoteKind::SystemNote,
    }
}

fn map_child_handoff_kind(
    kind: runtime::ConversationChildHandoffKind,
) -> ConversationChildHandoffKind {
    match kind {
        runtime::ConversationChildHandoffKind::Delegated => ConversationChildHandoffKind::Delegated,
        runtime::ConversationChildHandoffKind::Progress => ConversationChildHandoffKind::Progress,
        runtime::ConversationChildHandoffKind::Returned => ConversationChildHandoffKind::Returned,
    }
}

fn into_runtime_child_handoff_kind(
    kind: ConversationChildHandoffKind,
) -> runtime::ConversationChildHandoffKind {
    match kind {
        ConversationChildHandoffKind::Delegated => runtime::ConversationChildHandoffKind::Delegated,
        ConversationChildHandoffKind::Progress => runtime::ConversationChildHandoffKind::Progress,
        ConversationChildHandoffKind::Returned => runtime::ConversationChildHandoffKind::Returned,
    }
}

fn map_transcript_error_kind(
    kind: runtime::ConversationTranscriptErrorKind,
) -> ConversationTranscriptErrorKind {
    match kind {
        runtime::ConversationTranscriptErrorKind::ProviderError => {
            ConversationTranscriptErrorKind::ProviderError
        },
        runtime::ConversationTranscriptErrorKind::ContextWindowExceeded => {
            ConversationTranscriptErrorKind::ContextWindowExceeded
        },
        runtime::ConversationTranscriptErrorKind::ToolFatal => {
            ConversationTranscriptErrorKind::ToolFatal
        },
        runtime::ConversationTranscriptErrorKind::RateLimit => {
            ConversationTranscriptErrorKind::RateLimit
        },
    }
}

fn into_runtime_transcript_error_kind(
    kind: ConversationTranscriptErrorKind,
) -> runtime::ConversationTranscriptErrorKind {
    match kind {
        ConversationTranscriptErrorKind::ProviderError => {
            runtime::ConversationTranscriptErrorKind::ProviderError
        },
        ConversationTranscriptErrorKind::ContextWindowExceeded => {
            runtime::ConversationTranscriptErrorKind::ContextWindowExceeded
        },
        ConversationTranscriptErrorKind::ToolFatal => {
            runtime::ConversationTranscriptErrorKind::ToolFatal
        },
        ConversationTranscriptErrorKind::RateLimit => {
            runtime::ConversationTranscriptErrorKind::RateLimit
        },
    }
}

fn map_plan_event_kind(kind: runtime::ConversationPlanEventKind) -> ConversationPlanEventKind {
    match kind {
        runtime::ConversationPlanEventKind::Saved => ConversationPlanEventKind::Saved,
        runtime::ConversationPlanEventKind::ReviewPending => {
            ConversationPlanEventKind::ReviewPending
        },
        runtime::ConversationPlanEventKind::Presented => ConversationPlanEventKind::Presented,
    }
}

fn into_runtime_plan_event_kind(
    kind: ConversationPlanEventKind,
) -> runtime::ConversationPlanEventKind {
    match kind {
        ConversationPlanEventKind::Saved => runtime::ConversationPlanEventKind::Saved,
        ConversationPlanEventKind::ReviewPending => {
            runtime::ConversationPlanEventKind::ReviewPending
        },
        ConversationPlanEventKind::Presented => runtime::ConversationPlanEventKind::Presented,
    }
}

fn map_plan_review_kind(kind: runtime::ConversationPlanReviewKind) -> ConversationPlanReviewKind {
    match kind {
        runtime::ConversationPlanReviewKind::RevisePlan => ConversationPlanReviewKind::RevisePlan,
        runtime::ConversationPlanReviewKind::FinalReview => ConversationPlanReviewKind::FinalReview,
    }
}

fn into_runtime_plan_review_kind(
    kind: ConversationPlanReviewKind,
) -> runtime::ConversationPlanReviewKind {
    match kind {
        ConversationPlanReviewKind::RevisePlan => runtime::ConversationPlanReviewKind::RevisePlan,
        ConversationPlanReviewKind::FinalReview => runtime::ConversationPlanReviewKind::FinalReview,
    }
}
