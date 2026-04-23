use astrcode_core::{
    ChildAgentRef, CompactAppliedMeta, CompactTrigger, Phase, PromptCacheDiagnostics,
    SessionEventRecord, SystemPromptLayer, ToolOutputStream,
};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationBlockStatus {
    Streaming,
    Complete,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationSystemNoteKind {
    Compact,
    SystemNote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationChildHandoffKind {
    Delegated,
    Progress,
    Returned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationTranscriptErrorKind {
    ProviderError,
    ContextWindowExceeded,
    ToolFatal,
    RateLimit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationPlanEventKind {
    Saved,
    ReviewPending,
    Presented,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationPlanReviewKind {
    RevisePlan,
    FinalReview,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ToolCallStreamsFacts {
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationUserBlockFacts {
    pub id: String,
    pub turn_id: Option<String>,
    pub markdown: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationAssistantBlockFacts {
    pub id: String,
    pub turn_id: Option<String>,
    pub status: ConversationBlockStatus,
    pub markdown: String,
    pub step_index: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationThinkingBlockFacts {
    pub id: String,
    pub turn_id: Option<String>,
    pub status: ConversationBlockStatus,
    pub markdown: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationPromptMetricsBlockFacts {
    pub id: String,
    pub turn_id: Option<String>,
    pub step_index: u32,
    pub estimated_tokens: u32,
    pub context_window: u32,
    pub effective_window: u32,
    pub threshold_tokens: u32,
    pub truncated_tool_results: u32,
    pub provider_input_tokens: Option<u32>,
    pub provider_output_tokens: Option<u32>,
    pub cache_creation_input_tokens: Option<u32>,
    pub cache_read_input_tokens: Option<u32>,
    pub provider_cache_metrics_supported: bool,
    pub prompt_cache_reuse_hits: u32,
    pub prompt_cache_reuse_misses: u32,
    pub prompt_cache_unchanged_layers: Vec<SystemPromptLayer>,
    pub prompt_cache_diagnostics: Option<PromptCacheDiagnostics>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationPlanReviewFacts {
    pub kind: ConversationPlanReviewKind,
    pub checklist: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConversationPlanBlockersFacts {
    pub missing_headings: Vec<String>,
    pub invalid_sections: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationPlanBlockFacts {
    pub id: String,
    pub turn_id: Option<String>,
    pub tool_call_id: String,
    pub event_kind: ConversationPlanEventKind,
    pub title: String,
    pub plan_path: String,
    pub summary: Option<String>,
    pub status: Option<String>,
    pub slug: Option<String>,
    pub updated_at: Option<String>,
    pub content: Option<String>,
    pub review: Option<ConversationPlanReviewFacts>,
    pub blockers: ConversationPlanBlockersFacts,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCallBlockFacts {
    pub id: String,
    pub turn_id: Option<String>,
    pub tool_call_id: String,
    pub tool_name: String,
    pub status: ConversationBlockStatus,
    pub input: Option<Value>,
    pub summary: Option<String>,
    pub error: Option<String>,
    pub duration_ms: Option<u64>,
    pub truncated: bool,
    pub metadata: Option<Value>,
    pub child_ref: Option<ChildAgentRef>,
    pub streams: ToolCallStreamsFacts,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationErrorBlockFacts {
    pub id: String,
    pub turn_id: Option<String>,
    pub code: ConversationTranscriptErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationSystemNoteBlockFacts {
    pub id: String,
    pub note_kind: ConversationSystemNoteKind,
    pub markdown: String,
    pub compact_trigger: Option<CompactTrigger>,
    pub compact_meta: Option<CompactAppliedMeta>,
    pub compact_preserved_recent_turns: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationChildHandoffBlockFacts {
    pub id: String,
    pub handoff_kind: ConversationChildHandoffKind,
    pub child_ref: ChildAgentRef,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConversationBlockFacts {
    User(ConversationUserBlockFacts),
    Assistant(ConversationAssistantBlockFacts),
    Thinking(ConversationThinkingBlockFacts),
    PromptMetrics(ConversationPromptMetricsBlockFacts),
    Plan(Box<ConversationPlanBlockFacts>),
    ToolCall(Box<ToolCallBlockFacts>),
    Error(ConversationErrorBlockFacts),
    SystemNote(ConversationSystemNoteBlockFacts),
    ChildHandoff(ConversationChildHandoffBlockFacts),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConversationBlockPatchFacts {
    AppendMarkdown {
        markdown: String,
    },
    ReplaceMarkdown {
        markdown: String,
    },
    AppendToolStream {
        stream: ToolOutputStream,
        chunk: String,
    },
    ReplaceSummary {
        summary: String,
    },
    ReplaceMetadata {
        metadata: Value,
    },
    ReplaceError {
        error: Option<String>,
    },
    ReplaceDuration {
        duration_ms: u64,
    },
    ReplaceChildRef {
        child_ref: ChildAgentRef,
    },
    SetTruncated {
        truncated: bool,
    },
    SetStatus {
        status: ConversationBlockStatus,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConversationDeltaFacts {
    AppendBlock {
        block: Box<ConversationBlockFacts>,
    },
    PatchBlock {
        block_id: String,
        patch: ConversationBlockPatchFacts,
    },
    CompleteBlock {
        block_id: String,
        status: ConversationBlockStatus,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationStepCursorFacts {
    pub turn_id: String,
    pub step_index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConversationStepProgressFacts {
    pub durable: Option<ConversationStepCursorFacts>,
    pub live: Option<ConversationStepCursorFacts>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConversationDeltaFrameFacts {
    pub cursor: String,
    pub step_progress: ConversationStepProgressFacts,
    pub delta: ConversationDeltaFacts,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConversationSnapshotFacts {
    pub cursor: Option<String>,
    pub phase: Phase,
    pub step_progress: ConversationStepProgressFacts,
    pub blocks: Vec<ConversationBlockFacts>,
}

#[derive(Debug, Clone)]
pub struct ConversationStreamReplayFacts {
    pub cursor: Option<String>,
    pub phase: Phase,
    pub seed_records: Vec<SessionEventRecord>,
    pub replay_frames: Vec<ConversationDeltaFrameFacts>,
    pub history: Vec<SessionEventRecord>,
}
