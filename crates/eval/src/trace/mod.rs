pub mod extractor;

use astrcode_core::{
    AgentCollaborationActionKind, AgentCollaborationOutcomeKind, AgentEventContext,
    CompactAppliedMeta, CompactTrigger, ExecutionContinuation, InvocationKind, PersistedToolOutput,
    PromptMetricsPayload, ResolvedExecutionLimitsSnapshot, ResolvedSubagentContextOverrides,
    SubRunResult, SubRunStorageMode, action::ToolOutputStream,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct StorageSeqRange {
    pub start: u64,
    pub end: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SessionTrace {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "astrcode_core::local_rfc3339_option"
    )]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_storage_seq: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub turns: Vec<TurnTrace>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agent_lineage: Vec<AgentLineageEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TurnTrace {
    pub turn_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_input: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_reasoning: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub thinking_deltas: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCallRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prompt_metrics: Vec<PromptMetricsSnapshot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compactions: Vec<CompactTrace>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sub_runs: Vec<SubRunTrace>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub collaboration_facts: Vec<CollaborationFactSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<ErrorTrace>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub timeline: Vec<TurnTimelineEvent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agent_lineage: Vec<AgentLineageEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_seq_range: Option<StorageSeqRange>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "astrcode_core::local_rfc3339_option"
    )]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_reason: Option<String>,
    #[serde(default)]
    pub incomplete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallRecord {
    pub tool_call_id: String,
    pub tool_name: String,
    pub args: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation: Option<ExecutionContinuation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_storage_seq: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_storage_seq: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stream_deltas: Vec<ToolCallDeltaRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persisted_reference: Option<ToolResultReferenceTrace>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallDeltaRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_seq: Option<u64>,
    pub stream: ToolOutputStream,
    pub delta: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolResultReferenceTrace {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_seq: Option<u64>,
    pub persisted_output: PersistedToolOutput,
    pub replacement: String,
    pub original_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PromptMetricsSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_seq: Option<u64>,
    pub agent: AgentEventContext,
    #[serde(flatten)]
    pub metrics: PromptMetricsPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompactTrace {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_seq: Option<u64>,
    pub agent: AgentEventContext,
    pub trigger: CompactTrigger,
    pub summary: String,
    pub meta: CompactAppliedMeta,
    pub preserved_recent_turns: u32,
    pub pre_tokens: u32,
    pub post_tokens_estimate: u32,
    pub messages_removed: u32,
    pub tokens_freed: u32,
    #[serde(with = "astrcode_core::local_rfc3339")]
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SubRunTrace {
    pub sub_run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_sub_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_mode: Option<SubRunStorageMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_overrides: Option<ResolvedSubagentContextOverrides>,
    pub resolved_limits: ResolvedExecutionLimitsSnapshot,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "astrcode_core::local_rfc3339_option"
    )]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "astrcode_core::local_rfc3339_option"
    )]
    pub finished_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<SubRunResult>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub collaboration_facts: Vec<CollaborationFactSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_seq_range: Option<StorageSeqRange>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CollaborationFactSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_seq: Option<u64>,
    pub fact_id: String,
    pub action: AgentCollaborationActionKind,
    pub outcome: AgentCollaborationOutcomeKind,
    pub parent_session_id: String,
    pub turn_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_sub_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ErrorTrace {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_seq: Option<u64>,
    pub agent: AgentEventContext,
    pub message: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "astrcode_core::local_rfc3339_option"
    )]
    pub timestamp: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentLineageEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_sub_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation_kind: Option<InvocationKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_mode: Option<SubRunStorageMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_session_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TurnTimelineEventKind {
    UserMessage,
    AssistantDelta,
    ThinkingDelta,
    AssistantFinal,
    ToolCall,
    ToolCallDelta,
    ToolResult,
    ToolResultReferenceApplied,
    PromptMetrics,
    CompactApplied,
    SubRunStarted,
    SubRunFinished,
    AgentCollaborationFact,
    Error,
    TurnDone,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TurnTimelineEvent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_seq: Option<u64>,
    pub kind: TurnTimelineEventKind,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "astrcode_core::local_rfc3339_option"
    )]
    pub timestamp: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[cfg(test)]
mod tests {
    use astrcode_core::{AgentEventContext, PersistedToolOutput, action::ToolOutputStream};
    use chrono::{TimeZone, Utc};
    use serde_json::json;

    use super::{
        AgentLineageEntry, SessionTrace, StorageSeqRange, ToolCallRecord, ToolResultReferenceTrace,
        TurnTrace,
    };

    fn persisted_output() -> PersistedToolOutput {
        PersistedToolOutput {
            storage_kind: "file".to_string(),
            absolute_path: "D:/workspace/.astrcode/tool-results/call-1.txt".to_string(),
            relative_path: "tool-results/call-1.txt".to_string(),
            total_bytes: 100,
            preview_text: "hello".to_string(),
            preview_bytes: 5,
        }
    }

    #[test]
    fn session_trace_round_trip_serialization() {
        let trace = SessionTrace {
            session_id: Some("session-1".to_string()),
            working_dir: Some("D:/workspace".to_string()),
            started_at: Some(Utc.with_ymd_and_hms(2026, 4, 20, 8, 0, 0).unwrap()),
            parent_session_id: Some("session-parent".to_string()),
            parent_storage_seq: Some(12),
            turns: vec![TurnTrace {
                turn_id: "turn-1".to_string(),
                user_input: Some("read the file".to_string()),
                assistant_output: Some("done".to_string()),
                assistant_reasoning: Some("thinking".to_string()),
                thinking_deltas: vec!["step".to_string()],
                tool_calls: vec![ToolCallRecord {
                    tool_call_id: "call-1".to_string(),
                    tool_name: "Read".to_string(),
                    args: json!({"path": "README.md"}),
                    output: Some("hello".to_string()),
                    success: Some(true),
                    error: None,
                    metadata: None,
                    continuation: None,
                    duration_ms: Some(12),
                    started_storage_seq: Some(2),
                    finished_storage_seq: Some(4),
                    stream_deltas: vec![super::ToolCallDeltaRecord {
                        storage_seq: Some(3),
                        stream: ToolOutputStream::Stdout,
                        delta: "he".to_string(),
                    }],
                    persisted_reference: Some(ToolResultReferenceTrace {
                        storage_seq: Some(5),
                        persisted_output: persisted_output(),
                        replacement: "<persisted-output>".to_string(),
                        original_bytes: 100,
                    }),
                }],
                prompt_metrics: vec![super::PromptMetricsSnapshot {
                    storage_seq: Some(6),
                    agent: AgentEventContext::default(),
                    metrics: astrcode_core::PromptMetricsPayload {
                        step_index: 1,
                        estimated_tokens: 200,
                        context_window: 8000,
                        effective_window: 7000,
                        threshold_tokens: 6000,
                        truncated_tool_results: 0,
                        provider_input_tokens: Some(100),
                        provider_output_tokens: Some(50),
                        cache_creation_input_tokens: None,
                        cache_read_input_tokens: None,
                        provider_cache_metrics_supported: false,
                        prompt_cache_reuse_hits: 0,
                        prompt_cache_reuse_misses: 0,
                        prompt_cache_unchanged_layers: Vec::new(),
                        prompt_cache_diagnostics: None,
                    },
                }],
                compactions: Vec::new(),
                sub_runs: Vec::new(),
                collaboration_facts: Vec::new(),
                errors: Vec::new(),
                timeline: Vec::new(),
                agent_lineage: vec![AgentLineageEntry {
                    agent_id: Some("agent-root".to_string()),
                    parent_turn_id: None,
                    agent_profile: Some("default".to_string()),
                    sub_run_id: None,
                    parent_sub_run_id: None,
                    invocation_kind: Some(astrcode_core::InvocationKind::RootExecution),
                    storage_mode: None,
                    child_session_id: None,
                }],
                storage_seq_range: Some(StorageSeqRange { start: 2, end: 6 }),
                completed_at: Some(Utc.with_ymd_and_hms(2026, 4, 20, 8, 0, 10).unwrap()),
                completion_reason: Some("completed".to_string()),
                incomplete: false,
            }],
            agent_lineage: vec![AgentLineageEntry {
                agent_id: Some("agent-root".to_string()),
                parent_turn_id: None,
                agent_profile: Some("default".to_string()),
                sub_run_id: None,
                parent_sub_run_id: None,
                invocation_kind: Some(astrcode_core::InvocationKind::RootExecution),
                storage_mode: None,
                child_session_id: None,
            }],
        };

        let json = serde_json::to_string(&trace).expect("trace should serialize");
        let decoded: SessionTrace = serde_json::from_str(&json).expect("trace should deserialize");
        assert_eq!(decoded, trace);
    }
}
