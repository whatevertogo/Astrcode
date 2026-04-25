use std::collections::{HashMap, HashSet};

use astrcode_core::{
    ChildAgentRef, CompactAppliedMeta, CompactTrigger, ExecutionTaskStatus, Phase,
};
use astrcode_host_session::SessionControlStateSnapshot;
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
    ConversationStepCursorDto, ConversationStepProgressDto, ConversationStreamEnvelopeDto,
    ConversationSystemNoteBlockDto, ConversationSystemNoteKindDto, ConversationTaskItemDto,
    ConversationTaskStatusDto, ConversationThinkingBlockDto, ConversationToolCallBlockDto,
    ConversationToolStreamsDto, ConversationTranscriptErrorCodeDto, ConversationUserBlockDto,
    conversation::v1::ConversationPromptMetricsBlockDto,
};

use crate::conversation_read_model::{
    ConversationBlockFacts, ConversationBlockPatchFacts, ConversationBlockStatus,
    ConversationChildHandoffBlockFacts, ConversationChildHandoffKind, ConversationDeltaFacts,
    ConversationDeltaFrameFacts, ConversationDeltaProjector, ConversationPlanBlockFacts,
    ConversationPlanEventKind, ConversationPlanReviewKind, ConversationReplayStream,
    ConversationSnapshotFacts, ConversationStepCursorFacts, ConversationStepProgressFacts,
    ConversationStreamReplayFacts, ConversationSystemNoteKind, ConversationTranscriptErrorKind,
    ToolCallBlockFacts,
};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum ConversationFocus {
    #[default]
    Root,
    SubRun {
        sub_run_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TerminalLastCompactMetaFacts {
    pub trigger: CompactTrigger,
    pub meta: CompactAppliedMeta,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlanReferenceFacts {
    pub slug: String,
    pub path: String,
    pub status: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskItemFacts {
    pub content: String,
    pub status: ExecutionTaskStatus,
    pub active_form: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConversationControlSummary {
    pub phase: Phase,
    pub can_submit_prompt: bool,
    pub can_request_compact: bool,
    pub compact_pending: bool,
    pub compacting: bool,
    pub active_turn_id: Option<String>,
    pub last_compact_meta: Option<TerminalLastCompactMetaFacts>,
    pub current_mode_id: String,
    pub active_plan: Option<PlanReferenceFacts>,
    pub active_tasks: Option<Vec<TaskItemFacts>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TerminalControlFacts {
    pub phase: Phase,
    pub active_turn_id: Option<String>,
    pub manual_compact_pending: bool,
    pub compacting: bool,
    pub last_compact_meta: Option<TerminalLastCompactMetaFacts>,
    pub current_mode_id: String,
    pub active_plan: Option<PlanReferenceFacts>,
    pub active_tasks: Option<Vec<TaskItemFacts>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TerminalChildSummaryFacts {
    pub node: astrcode_core::ChildSessionNode,
    pub phase: Phase,
    pub title: Option<String>,
    pub display_name: Option<String>,
    pub recent_output: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConversationChildSummarySummary {
    pub child_session_id: String,
    pub child_agent_id: String,
    pub title: String,
    pub lifecycle: astrcode_core::AgentLifecycleStatus,
    pub latest_output_summary: Option<String>,
    pub child_ref: Option<ChildAgentRef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TerminalSlashAction {
    CreateSession,
    OpenResume,
    RequestCompact,
    InsertText { text: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConversationSlashActionSummary {
    InsertText,
    ExecuteCommand,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TerminalSlashCandidateFacts {
    pub id: String,
    pub title: String,
    pub description: String,
    pub keywords: Vec<String>,
    pub badges: Vec<String>,
    pub action: TerminalSlashAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConversationSlashCandidateSummary {
    pub id: String,
    pub title: String,
    pub description: String,
    pub keywords: Vec<String>,
    pub action_kind: ConversationSlashActionSummary,
    pub action_value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConversationAuthoritativeSummary {
    pub control: ConversationControlSummary,
    pub child_summaries: Vec<ConversationChildSummarySummary>,
    pub slash_candidates: Vec<ConversationSlashCandidateSummary>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TerminalFacts {
    pub active_session_id: String,
    pub session_title: String,
    pub transcript: ConversationSnapshotFacts,
    pub control: TerminalControlFacts,
    pub child_summaries: Vec<TerminalChildSummaryFacts>,
    pub slash_candidates: Vec<TerminalSlashCandidateFacts>,
}

#[derive(Debug)]
pub(crate) struct TerminalStreamReplayFacts {
    pub replay: ConversationStreamReplayFacts,
    pub stream: ConversationReplayStream,
    pub control: TerminalControlFacts,
    pub child_summaries: Vec<TerminalChildSummaryFacts>,
    pub slash_candidates: Vec<TerminalSlashCandidateFacts>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TerminalRehydrateReason {
    CursorExpired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TerminalRehydrateFacts {
    pub session_id: String,
    pub requested_cursor: String,
    pub latest_cursor: Option<String>,
    pub reason: TerminalRehydrateReason,
}

#[derive(Debug)]
pub(crate) enum TerminalStreamFacts {
    Replay(Box<TerminalStreamReplayFacts>),
    RehydrateRequired(TerminalRehydrateFacts),
}

pub(crate) fn map_control_facts(control: SessionControlStateSnapshot) -> TerminalControlFacts {
    TerminalControlFacts {
        phase: control.phase,
        active_turn_id: control.active_turn_id,
        manual_compact_pending: control.manual_compact_pending,
        compacting: control.compacting,
        last_compact_meta: control
            .last_compact_meta
            .map(|meta| TerminalLastCompactMetaFacts {
                trigger: meta.trigger,
                meta: meta.meta,
            }),
        current_mode_id: control.current_mode_id.to_string(),
        active_plan: None,
        active_tasks: None,
    }
}

pub(crate) fn summarize_conversation_control(
    control: &TerminalControlFacts,
) -> ConversationControlSummary {
    ConversationControlSummary {
        phase: control.phase,
        can_submit_prompt: control.active_turn_id.is_none()
            && matches!(
                control.phase,
                Phase::Idle | Phase::Done | Phase::Interrupted
            ),
        can_request_compact: !control.manual_compact_pending && !control.compacting,
        compact_pending: control.manual_compact_pending,
        compacting: control.compacting,
        active_turn_id: control.active_turn_id.clone(),
        last_compact_meta: control.last_compact_meta.clone(),
        current_mode_id: control.current_mode_id.clone(),
        active_plan: control.active_plan.clone(),
        active_tasks: control.active_tasks.clone(),
    }
}

pub(crate) fn summarize_conversation_child_summary(
    summary: &TerminalChildSummaryFacts,
) -> ConversationChildSummarySummary {
    ConversationChildSummarySummary {
        child_session_id: summary.node.child_session_id.to_string(),
        child_agent_id: summary.node.agent_id().to_string(),
        title: summary
            .title
            .clone()
            .or_else(|| summary.display_name.clone())
            .unwrap_or_else(|| summary.node.child_session_id.to_string()),
        lifecycle: summary.node.status,
        latest_output_summary: summary.recent_output.clone(),
        child_ref: Some(summary.node.child_ref()),
    }
}

pub(crate) fn summarize_conversation_child_ref(
    child_ref: &ChildAgentRef,
) -> ConversationChildSummarySummary {
    ConversationChildSummarySummary {
        child_session_id: child_ref.open_session_id.to_string(),
        child_agent_id: child_ref.agent_id().to_string(),
        title: child_ref.agent_id().to_string(),
        lifecycle: child_ref.status,
        latest_output_summary: None,
        child_ref: Some(child_ref.clone()),
    }
}

pub(crate) fn summarize_conversation_slash_candidate(
    candidate: &TerminalSlashCandidateFacts,
) -> ConversationSlashCandidateSummary {
    let (action_kind, action_value) = match &candidate.action {
        TerminalSlashAction::CreateSession => (
            ConversationSlashActionSummary::ExecuteCommand,
            "/new".to_string(),
        ),
        TerminalSlashAction::OpenResume => (
            ConversationSlashActionSummary::ExecuteCommand,
            "/resume".to_string(),
        ),
        TerminalSlashAction::RequestCompact => (
            ConversationSlashActionSummary::ExecuteCommand,
            "/compact".to_string(),
        ),
        TerminalSlashAction::InsertText { text } => {
            (ConversationSlashActionSummary::InsertText, text.clone())
        },
    };

    ConversationSlashCandidateSummary {
        id: candidate.id.clone(),
        title: candidate.title.clone(),
        description: candidate.description.clone(),
        keywords: candidate.keywords.clone(),
        action_kind,
        action_value,
    }
}

pub(crate) fn summarize_conversation_authoritative(
    control: &TerminalControlFacts,
    child_summaries: &[TerminalChildSummaryFacts],
    slash_candidates: &[TerminalSlashCandidateFacts],
) -> ConversationAuthoritativeSummary {
    ConversationAuthoritativeSummary {
        control: summarize_conversation_control(control),
        child_summaries: child_summaries
            .iter()
            .map(summarize_conversation_child_summary)
            .collect(),
        slash_candidates: slash_candidates
            .iter()
            .map(summarize_conversation_slash_candidate)
            .collect(),
    }
}

pub(crate) fn truncate_terminal_summary(content: &str) -> String {
    const MAX_SUMMARY_CHARS: usize = 120;
    let normalized = content.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = normalized.chars();
    let truncated = chars.by_ref().take(MAX_SUMMARY_CHARS).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

pub(crate) fn latest_terminal_summary(snapshot: &ConversationSnapshotFacts) -> Option<String> {
    snapshot
        .blocks
        .iter()
        .rev()
        .find_map(summary_from_block)
        .or_else(|| latest_transcript_cursor(snapshot).map(|cursor| format!("cursor:{cursor}")))
}

pub(crate) fn build_conversation_snapshot(
    records: &[astrcode_core::SessionEventRecord],
    phase: Phase,
) -> ConversationSnapshotFacts {
    let mut projector = ConversationDeltaProjector::new();
    projector.seed(records);
    let blocks = suppress_draft_approval_plan_leakage(projector.into_blocks());
    ConversationSnapshotFacts {
        cursor: records.last().map(|record| record.event_id.clone()),
        phase,
        step_progress: durable_step_progress_from_blocks(&blocks),
        blocks,
    }
}

pub(crate) fn build_conversation_replay_frames(
    seed_records: &[astrcode_core::SessionEventRecord],
    history: &[astrcode_core::SessionEventRecord],
) -> Vec<ConversationDeltaFrameFacts> {
    let mut projector = ConversationDeltaProjector::new();
    projector.seed(seed_records);
    let mut step_progress = durable_step_progress_from_blocks(projector.blocks());
    let mut raw_frames = Vec::new();
    for record in history {
        raw_frames.extend(
            projector
                .project_record(record)
                .into_iter()
                .map(|delta| (record.event_id.clone(), delta)),
        );
    }
    let hidden_block_ids = draft_approval_leakage_hidden_block_ids(projector.blocks());

    let mut frames = Vec::new();
    for (cursor, delta) in raw_frames {
        if delta_block_id(&delta).is_some_and(|block_id| hidden_block_ids.contains(block_id)) {
            continue;
        }
        observe_durable_delta_step(&mut step_progress, &delta);
        frames.push(ConversationDeltaFrameFacts {
            cursor,
            step_progress: step_progress.clone(),
            delta,
        });
    }
    frames
}

fn latest_transcript_cursor(snapshot: &ConversationSnapshotFacts) -> Option<String> {
    snapshot.cursor.clone()
}

fn summary_from_block(block: &ConversationBlockFacts) -> Option<String> {
    match block {
        ConversationBlockFacts::Assistant(block) => summary_from_markdown(&block.markdown),
        ConversationBlockFacts::Plan(block) => summary_from_plan_block(block),
        ConversationBlockFacts::ToolCall(block) => summary_from_tool_call(block),
        ConversationBlockFacts::ChildHandoff(block) => summary_from_child_handoff(block),
        ConversationBlockFacts::Error(block) => summary_from_markdown(&block.message),
        ConversationBlockFacts::SystemNote(block) => summary_from_markdown(&block.markdown),
        ConversationBlockFacts::User(_)
        | ConversationBlockFacts::Thinking(_)
        | ConversationBlockFacts::PromptMetrics(_) => None,
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

fn summary_from_plan_block(block: &ConversationPlanBlockFacts) -> Option<String> {
    block
        .summary
        .as_deref()
        .filter(|summary| !summary.trim().is_empty())
        .map(truncate_terminal_summary)
        .or_else(|| {
            block
                .content
                .as_deref()
                .filter(|content| !content.trim().is_empty())
                .map(truncate_terminal_summary)
        })
        .or_else(|| summary_from_markdown(&block.title))
}

fn summary_from_child_handoff(block: &ConversationChildHandoffBlockFacts) -> Option<String> {
    block
        .message
        .as_deref()
        .filter(|message| !message.trim().is_empty())
        .map(truncate_terminal_summary)
}

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
        step_progress: project_conversation_step_progress(facts.transcript.step_progress.clone()),
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
        step_progress: project_conversation_step_progress(frame.step_progress),
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
        step_progress: ConversationStepProgressDto::default(),
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
        deltas.push(ConversationDeltaDto::RemoveChildSummary { child_session_id });
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

pub(crate) fn project_conversation_step_progress(
    facts: ConversationStepProgressFacts,
) -> ConversationStepProgressDto {
    ConversationStepProgressDto {
        durable: facts.durable.map(to_step_cursor_dto),
        live: facts.live.map(to_step_cursor_dto),
    }
}

fn suppress_draft_approval_plan_leakage(
    blocks: Vec<ConversationBlockFacts>,
) -> Vec<ConversationBlockFacts> {
    let hidden_block_ids = draft_approval_leakage_hidden_block_ids(&blocks);
    blocks
        .into_iter()
        .filter(|block| !hidden_block_ids.contains(block_id(block)))
        .collect()
}

fn draft_approval_leakage_hidden_block_ids(blocks: &[ConversationBlockFacts]) -> HashSet<String> {
    let mut turn_facts = HashMap::<String, (bool, bool)>::new();
    for block in blocks {
        match block {
            ConversationBlockFacts::User(block) => {
                let Some(turn_id) = block.turn_id.as_deref() else {
                    continue;
                };
                let facts = turn_facts
                    .entry(turn_id.to_string())
                    .or_insert((false, false));
                if is_approval_like_turn_text(&block.markdown) {
                    facts.0 = true;
                }
            },
            ConversationBlockFacts::Plan(block) => {
                let Some(turn_id) = block.turn_id.as_deref() else {
                    continue;
                };
                let facts = turn_facts
                    .entry(turn_id.to_string())
                    .or_insert((false, false));
                if block.status.as_deref() == Some("awaiting_approval")
                    || matches!(
                        block.event_kind,
                        ConversationPlanEventKind::Presented
                            | ConversationPlanEventKind::ReviewPending
                    )
                {
                    facts.1 = true;
                }
            },
            _ => {},
        }
    }

    blocks
        .iter()
        .filter_map(|block| {
            let turn_id = turn_id(block)?;
            let (approval_like_user, has_review_plan) = turn_facts.get(turn_id).copied()?;
            if !approval_like_user || !has_review_plan {
                return None;
            }
            matches!(
                block,
                ConversationBlockFacts::Assistant(_) | ConversationBlockFacts::Thinking(_)
            )
            .then(|| block_id(block).to_string())
        })
        .collect()
}

fn delta_block_id(delta: &ConversationDeltaFacts) -> Option<&str> {
    match delta {
        ConversationDeltaFacts::Append { block } => Some(block_id(block.as_ref())),
        ConversationDeltaFacts::Patch { block_id, .. }
        | ConversationDeltaFacts::Complete { block_id, .. } => Some(block_id.as_str()),
    }
}

fn turn_id(block: &ConversationBlockFacts) -> Option<&str> {
    match block {
        ConversationBlockFacts::User(block) => block.turn_id.as_deref(),
        ConversationBlockFacts::Assistant(block) => block.turn_id.as_deref(),
        ConversationBlockFacts::Thinking(block) => block.turn_id.as_deref(),
        ConversationBlockFacts::PromptMetrics(block) => block.turn_id.as_deref(),
        ConversationBlockFacts::Plan(block) => block.turn_id.as_deref(),
        ConversationBlockFacts::ToolCall(block) => block.turn_id.as_deref(),
        ConversationBlockFacts::Error(block) => block.turn_id.as_deref(),
        ConversationBlockFacts::SystemNote(_) | ConversationBlockFacts::ChildHandoff(_) => None,
    }
}

fn is_approval_like_turn_text(text: &str) -> bool {
    let normalized_english = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    for phrase in ["approved", "go ahead", "implement it"] {
        if normalized_english == phrase
            || (phrase != "implement it" && normalized_english.starts_with(&format!("{phrase} ")))
        {
            return true;
        }
    }

    let normalized_chinese = text
        .chars()
        .filter(|ch| {
            !ch.is_whitespace()
                && !matches!(
                    ch,
                    ',' | '.'
                        | '!'
                        | '?'
                        | ';'
                        | ':'
                        | '，'
                        | '。'
                        | '！'
                        | '？'
                        | '；'
                        | '：'
                        | '【'
                        | '】'
                        | '、'
                )
        })
        .collect::<String>();
    for phrase in ["同意", "可以", "按这个做", "开始实现"] {
        let matched = if matches!(phrase, "同意" | "可以") {
            normalized_chinese == phrase
        } else {
            normalized_chinese == phrase || normalized_chinese.starts_with(phrase)
        };
        if matched {
            return true;
        }
    }

    false
}

fn durable_step_progress_from_blocks(
    blocks: &[ConversationBlockFacts],
) -> ConversationStepProgressFacts {
    let mut step_progress = ConversationStepProgressFacts::default();
    for block in blocks {
        observe_durable_block_step(&mut step_progress, block);
    }
    step_progress
}

fn observe_durable_delta_step(
    step_progress: &mut ConversationStepProgressFacts,
    delta: &ConversationDeltaFacts,
) {
    if let ConversationDeltaFacts::Append { block } = delta {
        observe_durable_block_step(step_progress, block.as_ref());
    }
}

fn observe_durable_block_step(
    step_progress: &mut ConversationStepProgressFacts,
    block: &ConversationBlockFacts,
) {
    let step_cursor = match block {
        ConversationBlockFacts::PromptMetrics(block) => Some(ConversationStepCursorFacts {
            turn_id: block
                .turn_id
                .clone()
                .unwrap_or_else(|| "session".to_string()),
            step_index: block.step_index,
        }),
        ConversationBlockFacts::Assistant(block) => {
            block
                .step_index
                .map(|step_index| ConversationStepCursorFacts {
                    turn_id: block
                        .turn_id
                        .clone()
                        .unwrap_or_else(|| "session".to_string()),
                    step_index,
                })
        },
        _ => None,
    };

    if let Some(step_cursor) = step_cursor {
        step_progress.durable = Some(step_cursor.clone());
        if let Some(live) = step_progress.live.as_ref() {
            if live.turn_id != step_cursor.turn_id || live.step_index <= step_cursor.step_index {
                step_progress.live = None;
            }
        }
    }
}

fn project_delta(
    delta: ConversationDeltaFacts,
    child_lookup: &HashMap<String, ConversationChildSummaryDto>,
) -> ConversationDeltaDto {
    match delta {
        ConversationDeltaFacts::Append { block } => ConversationDeltaDto::AppendBlock {
            block: project_block(block.as_ref(), child_lookup),
        },
        ConversationDeltaFacts::Patch { block_id, patch } => ConversationDeltaDto::PatchBlock {
            block_id,
            patch: project_patch(patch),
        },
        ConversationDeltaFacts::Complete { block_id, status } => {
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
                step_index: block.step_index,
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
        ConversationBlockFacts::PromptMetrics(block) => {
            ConversationBlockDto::PromptMetrics(ConversationPromptMetricsBlockDto {
                id: block.id.clone(),
                turn_id: block.turn_id.clone(),
                step_index: block.step_index,
                estimated_tokens: block.estimated_tokens,
                context_window: block.context_window,
                effective_window: block.effective_window,
                threshold_tokens: block.threshold_tokens,
                truncated_tool_results: block.truncated_tool_results,
                provider_input_tokens: block.provider_input_tokens,
                provider_output_tokens: block.provider_output_tokens,
                cache_creation_input_tokens: block.cache_creation_input_tokens,
                cache_read_input_tokens: block.cache_read_input_tokens,
                provider_cache_metrics_supported: block.provider_cache_metrics_supported,
                prompt_cache_reuse_hits: block.prompt_cache_reuse_hits,
                prompt_cache_reuse_misses: block.prompt_cache_reuse_misses,
                prompt_cache_unchanged_layers: block
                    .prompt_cache_unchanged_layers
                    .iter()
                    .filter_map(|layer| {
                        serde_json::to_value(layer)
                            .ok()
                            .and_then(|value| value.as_str().map(ToString::to_string))
                    })
                    .collect(),
                prompt_cache_diagnostics: block.prompt_cache_diagnostics.clone(),
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

fn to_step_cursor_dto(facts: ConversationStepCursorFacts) -> ConversationStepCursorDto {
    ConversationStepCursorDto {
        turn_id: facts.turn_id,
        step_index: facts.step_index,
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
                        ExecutionTaskStatus::Pending => ConversationTaskStatusDto::Pending,
                        ExecutionTaskStatus::InProgress => ConversationTaskStatusDto::InProgress,
                        ExecutionTaskStatus::Completed => ConversationTaskStatusDto::Completed,
                    },
                    active_form: task.active_form,
                })
                .collect()
        }),
    }
}

fn to_plan_reference_dto(plan: PlanReferenceFacts) -> ConversationPlanReferenceDto {
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

fn block_id(block: &ConversationBlockFacts) -> &str {
    match block {
        ConversationBlockFacts::User(block) => &block.id,
        ConversationBlockFacts::Assistant(block) => &block.id,
        ConversationBlockFacts::Thinking(block) => &block.id,
        ConversationBlockFacts::PromptMetrics(block) => &block.id,
        ConversationBlockFacts::Plan(block) => &block.id,
        ConversationBlockFacts::ToolCall(block) => &block.id,
        ConversationBlockFacts::Error(block) => &block.id,
        ConversationBlockFacts::SystemNote(block) => &block.id,
        ConversationBlockFacts::ChildHandoff(block) => &block.id,
    }
}
