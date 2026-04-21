//! application 自有的 session 编排合同。
//!
//! Why: `application` 只应该消费纯数据的编排摘要，
//! 不应继续把 `session-runtime` / `kernel` 的内部快照类型透传给上层。

use astrcode_core::{
    AgentTurnOutcome, ChildSessionNotification, Phase, StoredEvent, TurnProjectionSnapshot,
};
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
