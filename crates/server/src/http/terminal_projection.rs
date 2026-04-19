use std::collections::HashMap;

use astrcode_application::terminal::{
    ConversationChildSummarySummary, ConversationControlSummary, ConversationSlashActionSummary,
    ConversationSlashCandidateSummary, TerminalChildSummaryFacts, TerminalFacts,
    TerminalRehydrateFacts, TerminalSlashCandidateFacts, summarize_conversation_child_ref,
    summarize_conversation_child_summary, summarize_conversation_control,
    summarize_conversation_slash_candidate,
};
use astrcode_core::ChildAgentRef;
use astrcode_protocol::http::{
    ChildAgentRefDto, ConversationAssistantBlockDto, ConversationBannerDto,
    ConversationBannerErrorCodeDto, ConversationBlockDto, ConversationBlockPatchDto,
    ConversationBlockStatusDto, ConversationChildHandoffBlockDto, ConversationChildHandoffKindDto,
    ConversationChildSummaryDto, ConversationControlStateDto, ConversationCursorDto,
    ConversationDeltaDto, ConversationErrorBlockDto, ConversationErrorEnvelopeDto,
    ConversationLastCompactMetaDto, ConversationPlanBlockDto, ConversationPlanBlockersDto,
    ConversationPlanEventKindDto, ConversationPlanReferenceDto, ConversationPlanReviewDto,
    ConversationPlanReviewKindDto, ConversationSlashActionKindDto, ConversationSlashCandidateDto,
    ConversationSlashCandidatesResponseDto, ConversationSnapshotResponseDto,
    ConversationStreamEnvelopeDto, ConversationSystemNoteBlockDto, ConversationSystemNoteKindDto,
    ConversationTaskItemDto, ConversationTaskStatusDto, ConversationThinkingBlockDto,
    ConversationToolCallBlockDto, ConversationToolStreamsDto, ConversationTranscriptErrorCodeDto,
    ConversationUserBlockDto,
};
use astrcode_session_runtime::{
    ConversationBlockFacts, ConversationBlockPatchFacts, ConversationBlockStatus,
    ConversationChildHandoffBlockFacts, ConversationChildHandoffKind, ConversationDeltaFacts,
    ConversationDeltaFrameFacts, ConversationPlanBlockFacts, ConversationPlanEventKind,
    ConversationPlanReviewKind, ConversationSystemNoteKind, ConversationTranscriptErrorKind,
    ToolCallBlockFacts,
};

pub(crate) fn project_conversation_snapshot(
    facts: &TerminalFacts,
) -> ConversationSnapshotResponseDto {
    let child_lookup = child_summary_lookup(&facts.child_summaries);

    ConversationSnapshotResponseDto {
        session_id: facts.active_session_id.clone(),
        session_title: facts.session_title.clone(),
        cursor: ConversationCursorDto(
            facts
                .transcript
                .cursor
                .clone()
                .unwrap_or_else(|| "0.0".to_string()),
        ),
        phase: facts.control.phase,
        control: to_conversation_control_state_dto(summarize_conversation_control(&facts.control)),
        blocks: facts
            .transcript
            .blocks
            .iter()
            .map(|block| project_block(block, &child_lookup))
            .collect(),
        child_summaries: facts
            .child_summaries
            .iter()
            .map(summarize_conversation_child_summary)
            .map(to_conversation_child_summary_dto)
            .collect(),
        slash_candidates: facts
            .slash_candidates
            .iter()
            .map(summarize_conversation_slash_candidate)
            .map(to_conversation_slash_candidate_dto)
            .collect(),
        banner: None,
    }
}

pub(crate) fn project_conversation_frame(
    session_id: &str,
    frame: ConversationDeltaFrameFacts,
    child_lookup: &HashMap<String, ConversationChildSummaryDto>,
) -> ConversationStreamEnvelopeDto {
    ConversationStreamEnvelopeDto {
        session_id: session_id.to_string(),
        cursor: ConversationCursorDto(frame.cursor),
        delta: project_delta(frame.delta, child_lookup),
    }
}

pub(crate) fn project_conversation_rehydrate_banner(
    rehydrate: &TerminalRehydrateFacts,
) -> ConversationBannerDto {
    ConversationBannerDto {
        error: ConversationErrorEnvelopeDto {
            code: ConversationBannerErrorCodeDto::CursorExpired,
            message: format!(
                "cursor '{}' is no longer valid for session '{}'",
                rehydrate.requested_cursor, rehydrate.session_id
            ),
            rehydrate_required: true,
            details: Some(serde_json::json!({
                "requestedCursor": rehydrate.requested_cursor,
                "latestCursor": rehydrate.latest_cursor,
                "reason": format!("{:?}", rehydrate.reason),
            })),
        },
    }
}

pub(crate) fn project_conversation_rehydrate_envelope(
    rehydrate: &TerminalRehydrateFacts,
) -> ConversationStreamEnvelopeDto {
    ConversationStreamEnvelopeDto {
        session_id: rehydrate.session_id.clone(),
        cursor: ConversationCursorDto(
            rehydrate
                .latest_cursor
                .clone()
                .unwrap_or_else(|| rehydrate.requested_cursor.clone()),
        ),
        delta: ConversationDeltaDto::RehydrateRequired {
            error: project_conversation_rehydrate_banner(rehydrate).error,
        },
    }
}

pub(crate) fn project_conversation_slash_candidates(
    candidates: &[TerminalSlashCandidateFacts],
) -> ConversationSlashCandidatesResponseDto {
    ConversationSlashCandidatesResponseDto {
        items: candidates
            .iter()
            .map(summarize_conversation_slash_candidate)
            .map(to_conversation_slash_candidate_dto)
            .collect(),
    }
}

pub(crate) fn project_conversation_child_summary_summary_deltas(
    previous: &[ConversationChildSummarySummary],
    current: &[ConversationChildSummarySummary],
) -> Vec<ConversationDeltaDto> {
    let previous_by_id = previous
        .iter()
        .map(|summary| {
            (
                summary.child_session_id.clone(),
                to_conversation_child_summary_dto(summary.clone()),
            )
        })
        .collect::<HashMap<_, _>>();
    let current_by_id = current
        .iter()
        .map(|summary| {
            (
                summary.child_session_id.clone(),
                to_conversation_child_summary_dto(summary.clone()),
            )
        })
        .collect::<HashMap<_, _>>();

    let mut deltas = Vec::new();
    let mut removed_ids = previous_by_id
        .keys()
        .filter(|child_session_id| !current_by_id.contains_key(*child_session_id))
        .cloned()
        .collect::<Vec<_>>();
    removed_ids.sort();
    for child_session_id in removed_ids {
        deltas.push(ConversationDeltaDto::RemoveChildSummary {
            child_session_id: child_session_id.to_string(),
        });
    }

    let mut current_ids = current_by_id.keys().cloned().collect::<Vec<_>>();
    current_ids.sort();
    for child_session_id in current_ids {
        let current_child = current_by_id
            .get(&child_session_id)
            .expect("current child summary should exist");
        if previous_by_id.get(&child_session_id) != Some(current_child) {
            deltas.push(ConversationDeltaDto::UpsertChildSummary {
                child: current_child.clone(),
            });
        }
    }

    deltas
}

pub(crate) fn project_conversation_control_summary_delta(
    summary: &ConversationControlSummary,
) -> ConversationDeltaDto {
    ConversationDeltaDto::UpdateControlState {
        control: to_conversation_control_state_dto(summary.clone()),
    }
}

pub(crate) fn project_conversation_slash_candidate_summaries(
    candidates: &[ConversationSlashCandidateSummary],
) -> ConversationSlashCandidatesResponseDto {
    ConversationSlashCandidatesResponseDto {
        items: candidates
            .iter()
            .cloned()
            .map(to_conversation_slash_candidate_dto)
            .collect(),
    }
}

fn project_delta(
    delta: ConversationDeltaFacts,
    child_lookup: &HashMap<String, ConversationChildSummaryDto>,
) -> ConversationDeltaDto {
    match delta {
        ConversationDeltaFacts::AppendBlock { block } => ConversationDeltaDto::AppendBlock {
            block: project_block(block.as_ref(), child_lookup),
        },
        ConversationDeltaFacts::PatchBlock { block_id, patch } => {
            ConversationDeltaDto::PatchBlock {
                block_id,
                patch: project_patch(patch),
            }
        },
        ConversationDeltaFacts::CompleteBlock { block_id, status } => {
            ConversationDeltaDto::CompleteBlock {
                block_id,
                status: to_block_status_dto(status),
            }
        },
    }
}

fn project_patch(patch: ConversationBlockPatchFacts) -> ConversationBlockPatchDto {
    match patch {
        ConversationBlockPatchFacts::AppendMarkdown { markdown } => {
            ConversationBlockPatchDto::AppendMarkdown { markdown }
        },
        ConversationBlockPatchFacts::ReplaceMarkdown { markdown } => {
            ConversationBlockPatchDto::ReplaceMarkdown { markdown }
        },
        ConversationBlockPatchFacts::AppendToolStream { stream, chunk } => {
            ConversationBlockPatchDto::AppendToolStream { stream, chunk }
        },
        ConversationBlockPatchFacts::ReplaceSummary { summary } => {
            ConversationBlockPatchDto::ReplaceSummary { summary }
        },
        ConversationBlockPatchFacts::ReplaceMetadata { metadata } => {
            ConversationBlockPatchDto::ReplaceMetadata { metadata }
        },
        ConversationBlockPatchFacts::ReplaceError { error } => {
            ConversationBlockPatchDto::ReplaceError { error }
        },
        ConversationBlockPatchFacts::ReplaceDuration { duration_ms } => {
            ConversationBlockPatchDto::ReplaceDuration { duration_ms }
        },
        ConversationBlockPatchFacts::ReplaceChildRef { child_ref } => {
            ConversationBlockPatchDto::ReplaceChildRef {
                child_ref: to_child_ref_dto(child_ref),
            }
        },
        ConversationBlockPatchFacts::SetTruncated { truncated } => {
            ConversationBlockPatchDto::SetTruncated { truncated }
        },
        ConversationBlockPatchFacts::SetStatus { status } => ConversationBlockPatchDto::SetStatus {
            status: to_block_status_dto(status),
        },
    }
}

fn project_block(
    block: &ConversationBlockFacts,
    child_lookup: &HashMap<String, ConversationChildSummaryDto>,
) -> ConversationBlockDto {
    match block {
        ConversationBlockFacts::User(block) => {
            ConversationBlockDto::User(ConversationUserBlockDto {
                id: block.id.clone(),
                turn_id: block.turn_id.clone(),
                markdown: block.markdown.clone(),
            })
        },
        ConversationBlockFacts::Assistant(block) => {
            ConversationBlockDto::Assistant(ConversationAssistantBlockDto {
                id: block.id.clone(),
                turn_id: block.turn_id.clone(),
                status: to_block_status_dto(block.status),
                markdown: block.markdown.clone(),
            })
        },
        ConversationBlockFacts::Thinking(block) => {
            ConversationBlockDto::Thinking(ConversationThinkingBlockDto {
                id: block.id.clone(),
                turn_id: block.turn_id.clone(),
                status: to_block_status_dto(block.status),
                markdown: block.markdown.clone(),
            })
        },
        ConversationBlockFacts::Plan(block) => {
            ConversationBlockDto::Plan(project_plan_block(block.as_ref()))
        },
        ConversationBlockFacts::ToolCall(block) => {
            ConversationBlockDto::ToolCall(project_tool_call_block(block))
        },
        ConversationBlockFacts::Error(block) => {
            ConversationBlockDto::Error(ConversationErrorBlockDto {
                id: block.id.clone(),
                turn_id: block.turn_id.clone(),
                code: to_error_code_dto(block.code),
                message: block.message.clone(),
            })
        },
        ConversationBlockFacts::SystemNote(block) => {
            ConversationBlockDto::SystemNote(ConversationSystemNoteBlockDto {
                id: block.id.clone(),
                note_kind: match block.note_kind {
                    ConversationSystemNoteKind::Compact => ConversationSystemNoteKindDto::Compact,
                    ConversationSystemNoteKind::SystemNote => {
                        ConversationSystemNoteKindDto::SystemNote
                    },
                },
                markdown: block.markdown.clone(),
                compact_meta: block.compact_meta.as_ref().and_then(|meta| {
                    block
                        .compact_trigger
                        .map(|trigger| ConversationLastCompactMetaDto {
                            trigger,
                            meta: meta.clone(),
                        })
                }),
                preserved_recent_turns: block.compact_preserved_recent_turns,
            })
        },
        ConversationBlockFacts::ChildHandoff(block) => {
            ConversationBlockDto::ChildHandoff(project_child_handoff_block(block, child_lookup))
        },
    }
}

fn project_plan_block(block: &ConversationPlanBlockFacts) -> ConversationPlanBlockDto {
    ConversationPlanBlockDto {
        id: block.id.clone(),
        turn_id: block.turn_id.clone(),
        tool_call_id: block.tool_call_id.clone(),
        event_kind: match block.event_kind {
            ConversationPlanEventKind::Saved => ConversationPlanEventKindDto::Saved,
            ConversationPlanEventKind::ReviewPending => ConversationPlanEventKindDto::ReviewPending,
            ConversationPlanEventKind::Presented => ConversationPlanEventKindDto::Presented,
        },
        title: block.title.clone(),
        plan_path: block.plan_path.clone(),
        summary: block.summary.clone(),
        status: block.status.clone(),
        slug: block.slug.clone(),
        updated_at: block.updated_at.clone(),
        content: block.content.clone(),
        review: block
            .review
            .as_ref()
            .map(|review| ConversationPlanReviewDto {
                kind: match review.kind {
                    ConversationPlanReviewKind::RevisePlan => {
                        ConversationPlanReviewKindDto::RevisePlan
                    },
                    ConversationPlanReviewKind::FinalReview => {
                        ConversationPlanReviewKindDto::FinalReview
                    },
                },
                checklist: review.checklist.clone(),
            }),
        blockers: ConversationPlanBlockersDto {
            missing_headings: block.blockers.missing_headings.clone(),
            invalid_sections: block.blockers.invalid_sections.clone(),
        },
    }
}

fn project_tool_call_block(block: &ToolCallBlockFacts) -> ConversationToolCallBlockDto {
    ConversationToolCallBlockDto {
        id: block.id.clone(),
        turn_id: block.turn_id.clone(),
        tool_call_id: block.tool_call_id.clone(),
        tool_name: block.tool_name.clone(),
        status: to_block_status_dto(block.status),
        input: block.input.clone(),
        summary: block.summary.clone(),
        error: block.error.clone(),
        duration_ms: block.duration_ms,
        truncated: block.truncated,
        metadata: block.metadata.clone(),
        child_ref: block.child_ref.clone().map(to_child_ref_dto),
        streams: ConversationToolStreamsDto {
            stdout: block.streams.stdout.clone(),
            stderr: block.streams.stderr.clone(),
        },
    }
}

fn project_child_handoff_block(
    block: &ConversationChildHandoffBlockFacts,
    child_lookup: &HashMap<String, ConversationChildSummaryDto>,
) -> ConversationChildHandoffBlockDto {
    let child = child_lookup
        .get(block.child_ref.open_session_id.as_str())
        .cloned()
        .or_else(|| {
            child_lookup
                .get(block.child_ref.session_id().as_str())
                .cloned()
        })
        .unwrap_or_else(|| {
            to_conversation_child_summary_dto(summarize_conversation_child_ref(&block.child_ref))
        });

    ConversationChildHandoffBlockDto {
        id: block.id.clone(),
        handoff_kind: match block.handoff_kind {
            ConversationChildHandoffKind::Delegated => ConversationChildHandoffKindDto::Delegated,
            ConversationChildHandoffKind::Progress => ConversationChildHandoffKindDto::Progress,
            ConversationChildHandoffKind::Returned => ConversationChildHandoffKindDto::Returned,
        },
        child,
        message: block.message.clone(),
    }
}

fn child_summary_lookup(
    summaries: &[TerminalChildSummaryFacts],
) -> HashMap<String, ConversationChildSummaryDto> {
    child_summary_summary_lookup(
        &summaries
            .iter()
            .map(summarize_conversation_child_summary)
            .collect::<Vec<_>>(),
    )
}

pub(crate) fn child_summary_summary_lookup(
    summaries: &[ConversationChildSummarySummary],
) -> HashMap<String, ConversationChildSummaryDto> {
    let mut lookup = HashMap::new();
    for summary in summaries {
        let dto = to_conversation_child_summary_dto(summary.clone());
        lookup.insert(summary.child_session_id.clone(), dto.clone());
        if let Some(child_ref) = &dto.child_ref {
            lookup.insert(child_ref.open_session_id.clone(), dto.clone());
            lookup.insert(child_ref.session_id.clone(), dto.clone());
        }
    }
    lookup
}

fn to_conversation_child_summary_dto(
    summary: ConversationChildSummarySummary,
) -> ConversationChildSummaryDto {
    ConversationChildSummaryDto {
        child_session_id: summary.child_session_id,
        child_agent_id: summary.child_agent_id,
        title: summary.title,
        lifecycle: summary.lifecycle,
        latest_output_summary: summary.latest_output_summary,
        child_ref: summary.child_ref.map(to_child_ref_dto),
    }
}

fn to_conversation_control_state_dto(
    summary: ConversationControlSummary,
) -> ConversationControlStateDto {
    ConversationControlStateDto {
        phase: summary.phase,
        can_submit_prompt: summary.can_submit_prompt,
        can_request_compact: summary.can_request_compact,
        compact_pending: summary.compact_pending,
        compacting: summary.compacting,
        current_mode_id: summary.current_mode_id,
        active_turn_id: summary.active_turn_id,
        last_compact_meta: summary
            .last_compact_meta
            .map(|meta| ConversationLastCompactMetaDto {
                trigger: meta.trigger,
                meta: meta.meta,
            }),
        active_plan: summary.active_plan.map(to_plan_reference_dto),
        active_tasks: summary.active_tasks.map(|tasks| {
            tasks
                .into_iter()
                .map(|task| ConversationTaskItemDto {
                    content: task.content,
                    status: match task.status {
                        astrcode_core::ExecutionTaskStatus::Pending => {
                            ConversationTaskStatusDto::Pending
                        },
                        astrcode_core::ExecutionTaskStatus::InProgress => {
                            ConversationTaskStatusDto::InProgress
                        },
                        astrcode_core::ExecutionTaskStatus::Completed => {
                            ConversationTaskStatusDto::Completed
                        },
                    },
                    active_form: task.active_form,
                })
                .collect()
        }),
    }
}

fn to_plan_reference_dto(
    plan: astrcode_application::terminal::PlanReferenceFacts,
) -> ConversationPlanReferenceDto {
    ConversationPlanReferenceDto {
        slug: plan.slug,
        path: plan.path,
        status: plan.status,
        title: plan.title,
    }
}

fn to_conversation_slash_candidate_dto(
    summary: ConversationSlashCandidateSummary,
) -> ConversationSlashCandidateDto {
    ConversationSlashCandidateDto {
        id: summary.id,
        title: summary.title,
        description: summary.description,
        keywords: summary.keywords,
        action_kind: match summary.action_kind {
            ConversationSlashActionSummary::InsertText => {
                ConversationSlashActionKindDto::InsertText
            },
            ConversationSlashActionSummary::ExecuteCommand => {
                ConversationSlashActionKindDto::ExecuteCommand
            },
        },
        action_value: summary.action_value,
    }
}

fn to_child_ref_dto(child_ref: ChildAgentRef) -> ChildAgentRefDto {
    ChildAgentRefDto {
        agent_id: child_ref.agent_id().to_string(),
        session_id: child_ref.session_id().to_string(),
        sub_run_id: child_ref.sub_run_id().to_string(),
        parent_agent_id: child_ref.parent_agent_id().map(|id| id.to_string()),
        parent_sub_run_id: child_ref.parent_sub_run_id().map(|id| id.to_string()),
        lineage_kind: child_ref.lineage_kind,
        status: child_ref.status,
        open_session_id: child_ref.open_session_id.to_string(),
    }
}

fn to_block_status_dto(status: ConversationBlockStatus) -> ConversationBlockStatusDto {
    match status {
        ConversationBlockStatus::Streaming => ConversationBlockStatusDto::Streaming,
        ConversationBlockStatus::Complete => ConversationBlockStatusDto::Complete,
        ConversationBlockStatus::Failed => ConversationBlockStatusDto::Failed,
        ConversationBlockStatus::Cancelled => ConversationBlockStatusDto::Cancelled,
    }
}

fn to_error_code_dto(code: ConversationTranscriptErrorKind) -> ConversationTranscriptErrorCodeDto {
    match code {
        ConversationTranscriptErrorKind::ProviderError => {
            ConversationTranscriptErrorCodeDto::ProviderError
        },
        ConversationTranscriptErrorKind::ContextWindowExceeded => {
            ConversationTranscriptErrorCodeDto::ContextWindowExceeded
        },
        ConversationTranscriptErrorKind::ToolFatal => ConversationTranscriptErrorCodeDto::ToolFatal,
        ConversationTranscriptErrorKind::RateLimit => ConversationTranscriptErrorCodeDto::RateLimit,
    }
}
