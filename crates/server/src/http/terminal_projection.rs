use std::collections::HashMap;

use astrcode_application::{
    TerminalChildSummaryFacts, TerminalControlFacts, TerminalFacts, TerminalRehydrateFacts,
    TerminalSlashAction, TerminalSlashCandidateFacts,
};
use astrcode_core::{
    AgentLifecycleStatus, ChildAgentRef, ChildSessionLineageKind, ToolOutputStream,
};
use astrcode_protocol::http::{
    AgentLifecycleDto, ChildAgentRefDto, ChildSessionLineageKindDto, ConversationAssistantBlockDto,
    ConversationBannerDto, ConversationBannerErrorCodeDto, ConversationBlockDto,
    ConversationBlockPatchDto, ConversationBlockStatusDto, ConversationChildHandoffBlockDto,
    ConversationChildHandoffKindDto, ConversationChildSummaryDto, ConversationControlStateDto,
    ConversationCursorDto, ConversationDeltaDto, ConversationErrorBlockDto,
    ConversationErrorEnvelopeDto, ConversationLastCompactMetaDto, ConversationSlashActionKindDto,
    ConversationSlashCandidateDto, ConversationSlashCandidatesResponseDto,
    ConversationSnapshotResponseDto, ConversationStreamEnvelopeDto, ConversationSystemNoteBlockDto,
    ConversationSystemNoteKindDto, ConversationThinkingBlockDto, ConversationToolCallBlockDto,
    ConversationToolStreamsDto, ConversationTranscriptErrorCodeDto, ConversationUserBlockDto,
    PhaseDto, ToolOutputStreamDto,
};
use astrcode_session_runtime::{
    ConversationBlockFacts, ConversationBlockPatchFacts, ConversationBlockStatus,
    ConversationChildHandoffBlockFacts, ConversationChildHandoffKind, ConversationDeltaFacts,
    ConversationDeltaFrameFacts, ConversationSystemNoteKind, ConversationTranscriptErrorKind,
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
        phase: to_phase_dto(facts.control.phase),
        control: project_control_state(&facts.control),
        blocks: facts
            .transcript
            .blocks
            .iter()
            .map(|block| project_block(block, &child_lookup))
            .collect(),
        child_summaries: facts
            .child_summaries
            .iter()
            .map(project_child_summary)
            .collect(),
        slash_candidates: facts
            .slash_candidates
            .iter()
            .map(project_slash_candidate)
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

pub(crate) fn project_conversation_control_delta(
    control: &TerminalControlFacts,
) -> ConversationDeltaDto {
    ConversationDeltaDto::UpdateControlState {
        control: project_control_state(control),
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
        items: candidates.iter().map(project_slash_candidate).collect(),
    }
}

pub(crate) fn project_conversation_child_summary_deltas(
    previous: &[TerminalChildSummaryFacts],
    current: &[TerminalChildSummaryFacts],
) -> Vec<ConversationDeltaDto> {
    let previous_by_id = previous
        .iter()
        .map(|summary| {
            (
                summary.node.child_session_id.clone(),
                project_child_summary(summary),
            )
        })
        .collect::<HashMap<_, _>>();
    let current_by_id = current
        .iter()
        .map(|summary| {
            (
                summary.node.child_session_id.clone(),
                project_child_summary(summary),
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
            ConversationBlockPatchDto::AppendToolStream {
                stream: to_stream_dto(stream),
                chunk,
            }
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
            })
        },
        ConversationBlockFacts::ChildHandoff(block) => {
            ConversationBlockDto::ChildHandoff(project_child_handoff_block(block, child_lookup))
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
        .unwrap_or_else(|| fallback_child_summary(&block.child_ref));

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

fn fallback_child_summary(child_ref: &ChildAgentRef) -> ConversationChildSummaryDto {
    ConversationChildSummaryDto {
        child_session_id: child_ref.open_session_id.to_string(),
        child_agent_id: child_ref.agent_id().to_string(),
        title: child_ref.agent_id().to_string(),
        lifecycle: to_lifecycle_dto(child_ref.status),
        latest_output_summary: None,
        child_ref: Some(to_child_ref_dto(child_ref.clone())),
    }
}

fn child_summary_lookup(
    summaries: &[TerminalChildSummaryFacts],
) -> HashMap<String, ConversationChildSummaryDto> {
    let mut lookup = HashMap::new();
    for summary in summaries {
        let dto = project_child_summary(summary);
        lookup.insert(summary.node.child_session_id.to_string(), dto.clone());
        if let Some(child_ref) = &dto.child_ref {
            lookup.insert(child_ref.open_session_id.clone(), dto.clone());
            lookup.insert(child_ref.session_id.clone(), dto.clone());
        }
    }
    lookup
}

pub(crate) fn project_child_summary(
    summary: &TerminalChildSummaryFacts,
) -> ConversationChildSummaryDto {
    ConversationChildSummaryDto {
        child_session_id: summary.node.child_session_id.to_string(),
        child_agent_id: summary.node.agent_id().to_string(),
        title: summary
            .title
            .clone()
            .or_else(|| summary.display_name.clone())
            .unwrap_or_else(|| summary.node.child_session_id.to_string()),
        lifecycle: to_lifecycle_dto(summary.node.status),
        latest_output_summary: summary.recent_output.clone(),
        child_ref: Some(to_child_ref_dto(summary.node.child_ref())),
    }
}

fn project_control_state(control: &TerminalControlFacts) -> ConversationControlStateDto {
    let can_submit_prompt = matches!(
        control.phase,
        astrcode_core::Phase::Idle | astrcode_core::Phase::Done | astrcode_core::Phase::Interrupted
    );
    ConversationControlStateDto {
        phase: to_phase_dto(control.phase),
        can_submit_prompt,
        can_request_compact: !control.manual_compact_pending && !control.compacting,
        compact_pending: control.manual_compact_pending,
        compacting: control.compacting,
        active_turn_id: control.active_turn_id.clone(),
        last_compact_meta: control
            .last_compact_meta
            .as_ref()
            .map(project_last_compact_meta),
    }
}

fn project_last_compact_meta(
    facts: &astrcode_application::terminal::TerminalLastCompactMetaFacts,
) -> ConversationLastCompactMetaDto {
    ConversationLastCompactMetaDto {
        trigger: facts.trigger,
        meta: facts.meta.clone(),
    }
}

fn project_slash_candidate(
    candidate: &TerminalSlashCandidateFacts,
) -> ConversationSlashCandidateDto {
    let (action_kind, action_value) = match &candidate.action {
        TerminalSlashAction::CreateSession => (
            ConversationSlashActionKindDto::ExecuteCommand,
            "/new".to_string(),
        ),
        TerminalSlashAction::OpenResume => (
            ConversationSlashActionKindDto::ExecuteCommand,
            "/resume".to_string(),
        ),
        TerminalSlashAction::RequestCompact => (
            ConversationSlashActionKindDto::ExecuteCommand,
            "/compact".to_string(),
        ),
        TerminalSlashAction::OpenSkillPalette => (
            ConversationSlashActionKindDto::ExecuteCommand,
            "/skill".to_string(),
        ),
        TerminalSlashAction::InsertText { text } => {
            (ConversationSlashActionKindDto::InsertText, text.clone())
        },
    };

    ConversationSlashCandidateDto {
        id: candidate.id.clone(),
        title: candidate.title.clone(),
        description: candidate.description.clone(),
        keywords: candidate.keywords.clone(),
        action_kind,
        action_value,
    }
}

fn to_phase_dto(phase: astrcode_core::Phase) -> PhaseDto {
    match phase {
        astrcode_core::Phase::Idle => PhaseDto::Idle,
        astrcode_core::Phase::Thinking => PhaseDto::Thinking,
        astrcode_core::Phase::CallingTool => PhaseDto::CallingTool,
        astrcode_core::Phase::Streaming => PhaseDto::Streaming,
        astrcode_core::Phase::Interrupted => PhaseDto::Interrupted,
        astrcode_core::Phase::Done => PhaseDto::Done,
    }
}

fn to_lifecycle_dto(status: AgentLifecycleStatus) -> AgentLifecycleDto {
    match status {
        AgentLifecycleStatus::Pending => AgentLifecycleDto::Pending,
        AgentLifecycleStatus::Running => AgentLifecycleDto::Running,
        AgentLifecycleStatus::Idle => AgentLifecycleDto::Idle,
        AgentLifecycleStatus::Terminated => AgentLifecycleDto::Terminated,
    }
}

fn to_stream_dto(stream: ToolOutputStream) -> ToolOutputStreamDto {
    match stream {
        ToolOutputStream::Stdout => ToolOutputStreamDto::Stdout,
        ToolOutputStream::Stderr => ToolOutputStreamDto::Stderr,
    }
}

fn to_child_ref_dto(child_ref: ChildAgentRef) -> ChildAgentRefDto {
    ChildAgentRefDto {
        agent_id: child_ref.agent_id().to_string(),
        session_id: child_ref.session_id().to_string(),
        sub_run_id: child_ref.sub_run_id().to_string(),
        parent_agent_id: child_ref.parent_agent_id().map(|id| id.to_string()),
        parent_sub_run_id: child_ref.parent_sub_run_id().map(|id| id.to_string()),
        lineage_kind: match child_ref.lineage_kind {
            ChildSessionLineageKind::Spawn => ChildSessionLineageKindDto::Spawn,
            ChildSessionLineageKind::Fork => ChildSessionLineageKindDto::Fork,
            ChildSessionLineageKind::Resume => ChildSessionLineageKindDto::Resume,
        },
        status: to_lifecycle_dto(child_ref.status),
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
