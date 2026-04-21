//! # Input Queue 持久化类型
//!
//! 定义四工具协作模型下的 input queue 消息、批次、durable 事件载荷和 observe 快照。
//!
//! 所有类型都是纯 DTO，不含运行时策略或状态机逻辑。
//! 事件载荷由 `core` 定义结构，由 `runtime` 负责实际写入 session event log。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{
    lifecycle::{AgentLifecycleStatus, AgentTurnOutcome},
    require_non_empty_trimmed,
};
/// 稳定消息投递标识。
///
/// 在 at-least-once 语义下用于去重：crash 恢复后相同 delivery_id 重新出现
/// 应被视为上一轮的延续，而不是全新任务。
pub type DeliveryId = crate::ids::DeliveryId;

/// 固定批次标识。
///
/// 每轮 snapshot drain 时分配，记录本轮接管了哪些 delivery_ids。
/// batch_id 在 turn 的 durable 生命周期内保持不变。
pub type BatchId = String;

// ── Input Queue 消息信封 ──────────────────────────────────────────

/// 一条 durable 协作消息，是 input queue 的最小可恢复单元。
///
/// 入队时捕获发送方的状态快照（enqueue-time snapshot），
/// 后续注入 prompt 或 observe 时继续使用这些快照值，
/// 而不是注入时现查——保证因果链可追溯。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QueuedInputEnvelope {
    pub delivery_id: DeliveryId,
    pub from_agent_id: String,
    pub to_agent_id: String,
    pub message: String,
    pub queued_at: DateTime<Utc>,
    /// 入队时发送方生命周期快照。
    pub sender_lifecycle_status: AgentLifecycleStatus,
    /// 入队时发送方最近一轮结果快照。
    pub sender_last_turn_outcome: Option<AgentTurnOutcome>,
    /// 入队时发送方可打开会话目标。
    pub sender_open_session_id: String,
}

// ── Durable input queue 事件载荷 ──────────────────────────────────────

/// `AgentInputQueued` 事件载荷。
///
/// 记录一条刚成功进入 input queue 的协作消息。
/// live inbox 只能在 Queued append 成功后更新，顺序不能反过来。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InputQueuedPayload {
    #[serde(flatten)]
    pub envelope: QueuedInputEnvelope,
}

/// `AgentInputBatchStarted` 事件载荷。
///
/// 记录某个 agent 在本轮开始时通过 snapshot drain 接管了哪些消息。
/// 必须是 input-queue drain turn 的第一条 durable 事件，
/// 以确保 replay 时能准确恢复"本轮接管了什么"这一 durable 事实。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InputBatchStartedPayload {
    pub target_agent_id: String,
    pub turn_id: String,
    pub batch_id: BatchId,
    pub delivery_ids: Vec<DeliveryId>,
}

/// `AgentInputBatchAcked` 事件载荷。
///
/// 记录某轮在 durable turn completion 后确认处理完成。
/// 不允许在模型流结束但 turn 尚未 durable 提交时提前 ack。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InputBatchAckedPayload {
    pub target_agent_id: String,
    pub turn_id: String,
    pub batch_id: BatchId,
    pub delivery_ids: Vec<DeliveryId>,
}

/// `AgentInputDiscarded` 事件载荷。
///
/// 记录 close 时主动丢弃的 pending input queue 消息。
/// replay 时这些消息不再重建为 pending。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InputDiscardedPayload {
    pub target_agent_id: String,
    pub delivery_ids: Vec<DeliveryId>,
}

// ── 四工具参数 ────────────────────────────────────────────────────

/// `send` 工具参数。
///
/// 向直接父或直接子发送一条 durable 协作消息。
/// 仅允许直接父↔直接子，禁止兄弟、越级、跨树。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SendParams {
    /// 目标 agent 稳定 ID。
    pub agent_id: String,
    /// 协作消息正文。
    pub message: String,
}

impl SendParams {
    pub fn validate(&self) -> crate::error::Result<()> {
        require_non_empty_trimmed("agentId", &self.agent_id)?;
        require_non_empty_trimmed("message", &self.message)?;
        Ok(())
    }
}

/// `observe` 工具参数。
///
/// 获取目标 child agent 的增强快照。
/// 仅直接父可调用，非直接父、兄弟、跨树调用被拒绝。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ObserveParams {
    /// 被观测的 child agent 稳定 ID。
    pub agent_id: String,
}

impl ObserveParams {
    pub fn validate(&self) -> crate::error::Result<()> {
        require_non_empty_trimmed("agentId", &self.agent_id)?;
        Ok(())
    }
}

/// `close` 工具参数。
///
/// 终止目标 child agent 及其后代，是唯一公开终止手段。
/// 统一使用 subtree close，不支持仅关闭单节点。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CloseParams {
    /// 目标 child agent 稳定 ID。
    pub agent_id: String,
}

impl CloseParams {
    pub fn validate(&self) -> crate::error::Result<()> {
        require_non_empty_trimmed("agentId", &self.agent_id)?;
        Ok(())
    }
}

// ── Observe 快照结果 ──────────────────────────────────────────────

// ── Input Queue Projection（派生读模型）───────────────────────────

/// Input queue 的派生读模型，从 durable 事件重建。
///
/// 唯一 durable 真相仍是 event log，此结构只是 replay 后的缓存视图。
/// 用于 `observe`、wake 调度决策和恢复。
///
/// Replay 规则：
/// - `Queued` → 增加 pending
/// - `BatchStarted` → 标记 active batch（不等于已 ack）
/// - `BatchAcked` → 移出 pending/active
/// - `Discarded` → 标记为丢弃，停止重建
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InputQueueProjection {
    /// 待处理消息 ID（Queued - Acked - Discarded 后剩余）。
    pub pending_delivery_ids: Vec<DeliveryId>,
    /// 当前 started-but-not-acked 的批次 ID。
    pub active_batch_id: Option<BatchId>,
    /// 当前 active batch 中的消息 ID。
    pub active_delivery_ids: Vec<DeliveryId>,
    /// 因 close 而 durable 丢弃的消息 ID。
    pub discarded_delivery_ids: Vec<DeliveryId>,
}

impl InputQueueProjection {
    /// 返回当前待处理消息数量。
    pub fn pending_input_count(&self) -> usize {
        self.pending_delivery_ids.len()
    }
}

// ── Observe 快照结果 ──────────────────────────────────────────────

/// `observe` 工具返回的目标 Agent 查询结果。
///
/// 融合 live control state 与对话投影。
/// 是读模型而非领域实体。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ObserveSnapshot {
    pub agent_id: String,
    pub session_id: String,
    /// 当前生命周期状态。
    pub lifecycle_status: AgentLifecycleStatus,
    /// 最近一轮执行结果。
    pub last_turn_outcome: Option<AgentTurnOutcome>,
    /// 对话阶段（来自现有 AgentStateProjector）。
    pub phase: String,
    /// 当前轮次数。
    pub turn_count: u32,
    /// 当前正在处理的任务摘要。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_task: Option<String>,
    /// 最近 assistant 输出尾部。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_output_tail: Option<String>,
    /// 最后一个 turn 的尾部内容。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub last_turn_tail: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_params_validation_rejects_empty() {
        let err = SendParams {
            agent_id: "  ".to_string(),
            message: "hello".to_string(),
        }
        .validate()
        .expect_err("empty agent_id should be rejected");
        assert!(err.to_string().contains("agentId"));

        let err = SendParams {
            agent_id: "agent-1".to_string(),
            message: "  ".to_string(),
        }
        .validate()
        .expect_err("empty message should be rejected");
        assert!(err.to_string().contains("message"));
    }

    #[test]
    fn observe_params_validation_rejects_empty() {
        let err = ObserveParams {
            agent_id: String::new(),
        }
        .validate()
        .expect_err("empty agent_id should be rejected");
        assert!(err.to_string().contains("agentId"));
    }

    #[test]
    fn close_params_validation_rejects_empty() {
        let err = CloseParams {
            agent_id: "   ".to_string(),
        }
        .validate()
        .expect_err("empty agent_id should be rejected");
        assert!(err.to_string().contains("agentId"));
    }
}
