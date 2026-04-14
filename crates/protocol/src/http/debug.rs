use serde::{Deserialize, Serialize};

use super::{AgentLifecycleDto, AgentTurnOutcomeDto, PhaseDto, RuntimeMetricsDto};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeDebugOverviewDto {
    pub collected_at: String,
    pub spawn_rejection_ratio_bps: Option<u64>,
    pub metrics: RuntimeMetricsDto,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeDebugTimelineSampleDto {
    pub collected_at: String,
    pub spawn_rejection_ratio_bps: Option<u64>,
    pub observe_to_action_ratio_bps: Option<u64>,
    pub child_reuse_ratio_bps: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeDebugTimelineDto {
    pub window_started_at: String,
    pub window_ended_at: String,
    pub samples: Vec<RuntimeDebugTimelineSampleDto>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SessionDebugTraceItemKindDto {
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionDebugTraceItemDto {
    pub id: String,
    pub storage_seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_at: Option<String>,
    pub kind: SessionDebugTraceItemKindDto,
    pub title: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<AgentLifecycleDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_turn_outcome: Option<AgentTurnOutcomeDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionDebugTraceDto {
    pub session_id: String,
    pub title: String,
    pub phase: PhaseDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    pub items: Vec<SessionDebugTraceItemDto>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DebugAgentNodeKindDto {
    SessionRoot,
    ChildAgent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionDebugAgentNodeDto {
    pub node_id: String,
    pub kind: DebugAgentNodeKindDto,
    pub title: String,
    pub agent_id: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    pub depth: usize,
    pub lifecycle: AgentLifecycleDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_turn_outcome: Option<AgentTurnOutcomeDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionDebugAgentsDto {
    pub session_id: String,
    pub title: String,
    pub nodes: Vec<SessionDebugAgentNodeDto>,
}
