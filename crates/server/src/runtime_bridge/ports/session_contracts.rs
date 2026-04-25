//! server 自有的 session 编排合同。
//!
//! Why: server/application 只应该消费纯数据的编排摘要，
//! 不应继续把 `session-runtime` / `kernel` 的内部快照类型透传给上层。

use astrcode_core::{
    AgentLifecycleStatus, AgentTurnOutcome, ChildSessionNotification, LlmMessage, Phase,
    ResolvedExecutionLimitsSnapshot, ResolvedSubagentContextOverrides, StoredEvent, SubRunResult,
    SubRunStorageMode, UserMessageOrigin,
};
use astrcode_host_session::{InputQueueProjection, TurnProjectionSnapshot};
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

impl SessionObserveSnapshot {
    pub fn from_projected_state(
        lifecycle_status: AgentLifecycleStatus,
        projected: &astrcode_host_session::AgentState,
        input_queue_projection: &InputQueueProjection,
    ) -> Self {
        Self {
            phase: projected.phase,
            turn_count: projected.turn_count as u32,
            active_task: active_task_summary(lifecycle_status, projected, input_queue_projection),
            last_output_tail: extract_last_output(&projected.messages),
            last_turn_tail: extract_last_turn_tail(&projected.messages),
        }
    }
}

fn active_task_summary(
    lifecycle_status: AgentLifecycleStatus,
    projected: &astrcode_host_session::AgentState,
    input_queue_projection: &InputQueueProjection,
) -> Option<String> {
    if !input_queue_projection.active_delivery_ids.is_empty() {
        return extract_last_turn_tail(&projected.messages)
            .into_iter()
            .next();
    }
    if matches!(
        lifecycle_status,
        AgentLifecycleStatus::Pending | AgentLifecycleStatus::Running
    ) {
        return projected
            .messages
            .iter()
            .rev()
            .find_map(|message| match message {
                LlmMessage::User {
                    content,
                    origin: UserMessageOrigin::User,
                } => summarize_inline_text(content, 120),
                _ => None,
            });
    }
    None
}

fn extract_last_output(messages: &[LlmMessage]) -> Option<String> {
    messages.iter().rev().find_map(|message| match message {
        LlmMessage::Assistant { content, .. } if !content.trim().is_empty() => {
            Some(truncate_text(content, 200))
        },
        _ => None,
    })
}

fn extract_last_turn_tail(messages: &[LlmMessage]) -> Vec<String> {
    messages
        .iter()
        .rev()
        .filter_map(|message| match message {
            LlmMessage::User { content, .. }
            | LlmMessage::Assistant { content, .. }
            | LlmMessage::Tool { content, .. } => summarize_inline_text(content, 120),
        })
        .take(3)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn summarize_inline_text(content: &str, limit: usize) -> Option<String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(truncate_text(trimmed, limit))
}

fn truncate_text(content: &str, limit: usize) -> String {
    if content.chars().count() <= limit {
        return content.to_string();
    }
    let prefix = content.chars().take(limit).collect::<String>();
    format!("{prefix}...")
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
