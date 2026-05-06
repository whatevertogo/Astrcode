use astrcode_core::{
    ChildAgentRef, CompactAppliedMeta, CompactTrigger, Phase, PromptCacheDiagnostics,
    SystemPromptLayer, ToolOutputStream,
};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConversationBlockStatus {
    Streaming,
    Complete,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConversationSystemNoteKind {
    Compact,
    SystemNote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConversationChildHandoffKind {
    Delegated,
    Progress,
    Returned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConversationTranscriptErrorKind {
    ProviderError,
    ContextWindowExceeded,
    ToolFatal,
    RateLimit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConversationPlanEventKind {
    Saved,
    ReviewPending,
    Presented,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConversationPlanReviewKind {
    RevisePlan,
    FinalReview,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ToolCallStreamsFacts {
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConversationUserBlockFacts {
    pub id: String,
    pub turn_id: Option<String>,
    pub markdown: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConversationAssistantBlockFacts {
    pub id: String,
    pub turn_id: Option<String>,
    pub status: ConversationBlockStatus,
    pub markdown: String,
    pub step_index: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConversationThinkingBlockFacts {
    pub id: String,
    pub turn_id: Option<String>,
    pub status: ConversationBlockStatus,
    pub markdown: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConversationPromptMetricsBlockFacts {
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
pub(crate) struct ConversationPlanReviewFacts {
    pub kind: ConversationPlanReviewKind,
    pub checklist: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ConversationPlanBlockersFacts {
    pub missing_headings: Vec<String>,
    pub invalid_sections: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConversationPlanBlockFacts {
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
pub(crate) struct ToolCallBlockFacts {
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
pub(crate) struct ConversationErrorBlockFacts {
    pub id: String,
    pub turn_id: Option<String>,
    pub code: ConversationTranscriptErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConversationSystemNoteBlockFacts {
    pub id: String,
    pub note_kind: ConversationSystemNoteKind,
    pub markdown: String,
    pub compact_trigger: Option<CompactTrigger>,
    pub compact_meta: Option<CompactAppliedMeta>,
    pub compact_preserved_recent_turns: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConversationChildHandoffBlockFacts {
    pub id: String,
    pub handoff_kind: ConversationChildHandoffKind,
    pub child_ref: ChildAgentRef,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ConversationBlockFacts {
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
pub(crate) enum ConversationBlockPatchFacts {
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
pub(crate) enum ConversationDeltaFacts {
    Append {
        block: Box<ConversationBlockFacts>,
    },
    Patch {
        block_id: String,
        patch: ConversationBlockPatchFacts,
    },
    Complete {
        block_id: String,
        status: ConversationBlockStatus,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConversationStepCursorFacts {
    pub turn_id: String,
    pub step_index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ConversationStepProgressFacts {
    pub durable: Option<ConversationStepCursorFacts>,
    pub live: Option<ConversationStepCursorFacts>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ConversationDeltaFrameFacts {
    pub cursor: String,
    pub step_progress: ConversationStepProgressFacts,
    pub delta: ConversationDeltaFacts,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ConversationSnapshotFacts {
    pub cursor: Option<String>,
    pub phase: Phase,
    pub step_progress: ConversationStepProgressFacts,
    pub blocks: Vec<ConversationBlockFacts>,
}
