//! 终端层数据模型与投影辅助。
//!
//! 定义面向前端的事件流数据模型（`TerminalFacts`、`ConversationSlashCandidateFacts` 等）
//! 以及从 session-runtime 快照到终端视图的投影辅助函数。

use astrcode_core::{
    ChildAgentRef, ChildSessionNode, CompactAppliedMeta, CompactTrigger, ExecutionTaskStatus, Phase,
};
use astrcode_session_runtime::{
    ConversationSnapshotFacts as RuntimeConversationSnapshotFacts,
    ConversationStreamReplayFacts as RuntimeConversationStreamReplayFacts,
};
use chrono::{DateTime, Utc};

use crate::ComposerOptionKind;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ConversationFocus {
    #[default]
    Root,
    SubRun {
        sub_run_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalLastCompactMetaFacts {
    pub trigger: CompactTrigger,
    pub meta: CompactAppliedMeta,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanReferenceFacts {
    pub slug: String,
    pub path: String,
    pub status: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskItemFacts {
    pub content: String,
    pub status: ExecutionTaskStatus,
    pub active_form: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationControlSummary {
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

#[derive(Debug, Clone)]
pub struct TerminalControlFacts {
    pub phase: Phase,
    pub active_turn_id: Option<String>,
    pub manual_compact_pending: bool,
    pub compacting: bool,
    pub last_compact_meta: Option<TerminalLastCompactMetaFacts>,
    pub current_mode_id: String,
    pub active_plan: Option<PlanReferenceFacts>,
    pub active_tasks: Option<Vec<TaskItemFacts>>,
}

pub type ConversationControlFacts = TerminalControlFacts;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalChildSummaryFacts {
    pub node: ChildSessionNode,
    pub phase: Phase,
    pub title: Option<String>,
    pub display_name: Option<String>,
    pub recent_output: Option<String>,
}

pub type ConversationChildSummaryFacts = TerminalChildSummaryFacts;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationChildSummarySummary {
    pub child_session_id: String,
    pub child_agent_id: String,
    pub title: String,
    pub lifecycle: astrcode_core::AgentLifecycleStatus,
    pub latest_output_summary: Option<String>,
    pub child_ref: Option<ChildAgentRef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalSlashAction {
    CreateSession,
    OpenResume,
    RequestCompact,
    InsertText { text: String },
}

pub type ConversationSlashAction = TerminalSlashAction;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationSlashActionSummary {
    InsertText,
    ExecuteCommand,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalSlashCandidateFacts {
    pub kind: ComposerOptionKind,
    pub id: String,
    pub title: String,
    pub description: String,
    pub keywords: Vec<String>,
    pub badges: Vec<String>,
    pub action: TerminalSlashAction,
}

pub type ConversationSlashCandidateFacts = TerminalSlashCandidateFacts;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationSlashCandidateSummary {
    pub id: String,
    pub title: String,
    pub description: String,
    pub keywords: Vec<String>,
    pub action_kind: ConversationSlashActionSummary,
    pub action_value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationAuthoritativeSummary {
    pub control: ConversationControlSummary,
    pub child_summaries: Vec<ConversationChildSummarySummary>,
    pub slash_candidates: Vec<ConversationSlashCandidateSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalResumeCandidateFacts {
    pub session_id: String,
    pub title: String,
    pub display_name: String,
    pub working_dir: String,
    pub updated_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub phase: Phase,
    pub parent_session_id: Option<String>,
}

pub type ConversationResumeCandidateFacts = TerminalResumeCandidateFacts;

#[derive(Debug, Clone)]
pub struct TerminalFacts {
    pub active_session_id: String,
    pub session_title: String,
    pub transcript: RuntimeConversationSnapshotFacts,
    pub control: TerminalControlFacts,
    pub child_summaries: Vec<TerminalChildSummaryFacts>,
    pub slash_candidates: Vec<TerminalSlashCandidateFacts>,
}

pub type ConversationFacts = TerminalFacts;

#[derive(Debug)]
pub struct TerminalStreamReplayFacts {
    pub active_session_id: String,
    pub replay: RuntimeConversationStreamReplayFacts,
    pub control: TerminalControlFacts,
    pub child_summaries: Vec<TerminalChildSummaryFacts>,
    pub slash_candidates: Vec<TerminalSlashCandidateFacts>,
}

pub type ConversationStreamReplayFacts = TerminalStreamReplayFacts;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalRehydrateReason {
    CursorExpired,
}

pub type ConversationRehydrateReason = TerminalRehydrateReason;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalRehydrateFacts {
    pub session_id: String,
    pub requested_cursor: String,
    pub latest_cursor: Option<String>,
    pub reason: TerminalRehydrateReason,
}

pub type ConversationRehydrateFacts = TerminalRehydrateFacts;

#[derive(Debug)]
pub enum TerminalStreamFacts {
    Replay(Box<TerminalStreamReplayFacts>),
    RehydrateRequired(TerminalRehydrateFacts),
}

pub type ConversationStreamFacts = TerminalStreamFacts;

pub(crate) fn latest_transcript_cursor(
    snapshot: &RuntimeConversationSnapshotFacts,
) -> Option<String> {
    snapshot.cursor.clone()
}

pub fn truncate_terminal_summary(content: &str) -> String {
    const MAX_SUMMARY_CHARS: usize = 120;
    let normalized = content.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = normalized.chars();
    let truncated = chars.by_ref().take(MAX_SUMMARY_CHARS).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

pub fn summarize_conversation_control(
    control: &TerminalControlFacts,
) -> ConversationControlSummary {
    ConversationControlSummary {
        phase: control.phase,
        can_submit_prompt: matches!(
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

pub fn summarize_conversation_child_summary(
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

pub fn summarize_conversation_child_ref(
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

pub fn summarize_conversation_slash_candidate(
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

pub fn summarize_conversation_authoritative(
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
