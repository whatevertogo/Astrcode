use astrcode_application::RuntimeObservabilitySnapshot;
use astrcode_core::{
    AgentLifecycleStatus, AgentTurnOutcome, ChildSessionLineageKind, ChildSessionStatusSource,
    Phase,
};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct RuntimeDebugOverview {
    pub collected_at: DateTime<Utc>,
    pub metrics: RuntimeObservabilitySnapshot,
    pub spawn_rejection_ratio_bps: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeDebugTimelineSample {
    pub collected_at: DateTime<Utc>,
    pub spawn_rejection_ratio_bps: Option<u64>,
    pub observe_to_action_ratio_bps: Option<u64>,
    pub child_reuse_ratio_bps: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeDebugTimeline {
    pub window_started_at: DateTime<Utc>,
    pub window_ended_at: DateTime<Utc>,
    pub samples: Vec<RuntimeDebugTimelineSample>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionDebugTraceItemKind {
    ToolCall,
    ToolResult,
    PromptMetrics,
    SubRunStarted,
    SubRunFinished,
    ChildNotification,
    CollaborationFact,
    MailboxQueued,
    MailboxBatchStarted,
    MailboxBatchAcked,
    MailboxDiscarded,
    TurnDone,
    Error,
}

#[derive(Debug, Clone)]
pub struct SessionDebugTraceItem {
    pub id: String,
    pub storage_seq: u64,
    pub turn_id: Option<String>,
    pub recorded_at: Option<DateTime<Utc>>,
    pub kind: SessionDebugTraceItemKind,
    pub title: String,
    pub summary: String,
    pub agent_id: Option<String>,
    pub sub_run_id: Option<String>,
    pub child_agent_id: Option<String>,
    pub delivery_id: Option<String>,
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
    pub lifecycle: Option<AgentLifecycleStatus>,
    pub last_turn_outcome: Option<AgentTurnOutcome>,
}

#[derive(Debug, Clone)]
pub struct SessionDebugTrace {
    pub session_id: String,
    pub title: String,
    pub phase: Phase,
    pub parent_session_id: Option<String>,
    pub items: Vec<SessionDebugTraceItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugAgentNodeKind {
    SessionRoot,
    ChildAgent,
}

#[derive(Debug, Clone)]
pub struct SessionDebugAgentNode {
    pub node_id: String,
    pub kind: DebugAgentNodeKind,
    pub title: String,
    pub agent_id: String,
    pub session_id: String,
    pub child_session_id: Option<String>,
    pub sub_run_id: Option<String>,
    pub parent_agent_id: Option<String>,
    pub parent_session_id: Option<String>,
    pub depth: usize,
    pub lifecycle: AgentLifecycleStatus,
    pub last_turn_outcome: Option<AgentTurnOutcome>,
    pub status_source: Option<ChildSessionStatusSource>,
    pub lineage_kind: Option<ChildSessionLineageKind>,
}

#[derive(Debug, Clone)]
pub struct SessionDebugAgents {
    pub session_id: String,
    pub title: String,
    pub nodes: Vec<SessionDebugAgentNode>,
}
