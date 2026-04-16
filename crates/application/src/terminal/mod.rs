use astrcode_core::{ChildSessionNode, CompactAppliedMeta, CompactTrigger, Phase};
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

#[derive(Debug, Clone)]
pub struct TerminalControlFacts {
    pub phase: Phase,
    pub active_turn_id: Option<String>,
    pub manual_compact_pending: bool,
    pub compacting: bool,
    pub last_compact_meta: Option<TerminalLastCompactMetaFacts>,
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
pub enum TerminalSlashAction {
    CreateSession,
    OpenResume,
    RequestCompact,
    OpenSkillPalette,
    InsertText { text: String },
}

pub type ConversationSlashAction = TerminalSlashAction;

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
