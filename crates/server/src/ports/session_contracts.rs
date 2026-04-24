//! server 自有的 session 编排合同。
//!
//! Why: server/application 只应该消费纯数据的编排摘要，
//! 不应继续把 `session-runtime` / `kernel` 的内部快照类型透传给上层。

use astrcode_core::{
    AgentLifecycleStatus, AgentTurnOutcome, ChildSessionNotification, Phase,
    ResolvedExecutionLimitsSnapshot, ResolvedSubagentContextOverrides, StoredEvent, SubRunResult,
    SubRunStorageMode,
};
use astrcode_host_session::TurnProjectionSnapshot;
use serde::{Deserialize, Serialize};

/// 应用层使用的 turn outcome 摘要。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionTurnOutcomeSummary {
    pub outcome: AgentTurnOutcome,
    pub summary: String,
    pub technical_message: String,
}

/// 应用层使用的 turn 终态快照。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionTurnTerminalState {
    pub phase: Phase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projection: Option<TurnProjectionSnapshot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<StoredEvent>,
}

/// 应用层使用的 observe 快照。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionObserveSnapshot {
    pub phase: Phase,
    pub turn_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_task: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_output_tail: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub last_turn_tail: Vec<String>,
}

/// 应用层使用的可恢复父级投递摘要。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoverableParentDelivery {
    pub delivery_id: String,
    pub parent_session_id: String,
    pub parent_turn_id: String,
    pub queued_at_ms: i64,
    pub notification: ChildSessionNotification,
}

/// server/application 使用的 durable sub-run 状态摘要。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DurableSubRunStatusSummary {
    pub sub_run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    pub agent_id: String,
    pub agent_profile: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_session_id: Option<String>,
    pub depth: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_sub_run_id: Option<String>,
    pub storage_mode: SubRunStorageMode,
    pub lifecycle: AgentLifecycleStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_turn_outcome: Option<AgentTurnOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<SubRunResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_overrides: Option<ResolvedSubagentContextOverrides>,
    pub resolved_limits: ResolvedExecutionLimitsSnapshot,
}
