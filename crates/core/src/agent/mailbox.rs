//! # Mailbox 持久化类型
//!
//! 定义四工具协作模型下的 mailbox 消息、批次、durable 事件载荷和 observe 快照。
//!
//! 所有类型都是纯 DTO，不含运行时策略或状态机逻辑。
//! 事件载荷由 `core` 定义结构，由 `runtime` 负责实际写入 session event log。

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::lifecycle::{AgentLifecycleStatus, AgentTurnOutcome};
use crate::StoredEvent;

/// 稳定消息投递标识。
///
/// 在 at-least-once 语义下用于去重：crash 恢复后相同 delivery_id 重新出现
/// 应被视为上一轮的延续，而不是全新任务。
pub type DeliveryId = String;

/// 固定批次标识。
///
/// 每轮 snapshot drain 时分配，记录本轮接管了哪些 delivery_ids。
/// batch_id 在 turn 的 durable 生命周期内保持不变。
pub type BatchId = String;

// ── Mailbox 消息信封 ──────────────────────────────────────────────

/// 一条 durable 协作消息，是 mailbox 的最小可恢复单元。
///
/// 入队时捕获发送方的状态快照（enqueue-time snapshot），
/// 后续注入 prompt 或 observe 时继续使用这些快照值，
/// 而不是注入时现查——保证因果链可追溯。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentMailboxEnvelope {
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

// ── Durable Mailbox 事件载荷 ──────────────────────────────────────

/// `AgentMailboxQueued` 事件载荷。
///
/// 记录一条刚成功进入 mailbox 的协作消息。
/// live inbox 只能在 Queued append 成功后更新，顺序不能反过来。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MailboxQueuedPayload {
    #[serde(flatten)]
    pub envelope: AgentMailboxEnvelope,
}

/// `AgentMailboxBatchStarted` 事件载荷。
///
/// 记录某个 agent 在本轮开始时通过 snapshot drain 接管了哪些消息。
/// 必须是 mailbox-wake turn 的第一条 durable 事件，
/// 以确保 replay 时能准确恢复"本轮接管了什么"这一 durable 事实。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MailboxBatchStartedPayload {
    pub target_agent_id: String,
    pub turn_id: String,
    pub batch_id: BatchId,
    pub delivery_ids: Vec<DeliveryId>,
}

/// `AgentMailboxBatchAcked` 事件载荷。
///
/// 记录某轮在 durable turn completion 后确认处理完成。
/// 不允许在模型流结束但 turn 尚未 durable 提交时提前 ack。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MailboxBatchAckedPayload {
    pub target_agent_id: String,
    pub turn_id: String,
    pub batch_id: BatchId,
    pub delivery_ids: Vec<DeliveryId>,
}

/// `AgentMailboxDiscarded` 事件载荷。
///
/// 记录 close 时主动丢弃的 pending mailbox 消息。
/// replay 时这些消息不再重建为 pending。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MailboxDiscardedPayload {
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
        if self.agent_id.trim().is_empty() {
            return Err(crate::error::AstrError::Validation(
                "agentId 不能为空".to_string(),
            ));
        }
        if self.message.trim().is_empty() {
            return Err(crate::error::AstrError::Validation(
                "message 不能为空".to_string(),
            ));
        }
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
        if self.agent_id.trim().is_empty() {
            return Err(crate::error::AstrError::Validation(
                "agentId 不能为空".to_string(),
            ));
        }
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
        if self.agent_id.trim().is_empty() {
            return Err(crate::error::AstrError::Validation(
                "agentId 不能为空".to_string(),
            ));
        }
        Ok(())
    }
}

// ── Observe 快照结果 ──────────────────────────────────────────────

// ── Mailbox Projection（派生读模型）──────────────────────────────

/// Mailbox 的派生读模型，从 durable 事件重建。
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
pub struct MailboxProjection {
    /// 待处理消息 ID（Queued - Acked - Discarded 后剩余）。
    pub pending_delivery_ids: Vec<DeliveryId>,
    /// 当前 started-but-not-acked 的批次 ID。
    pub active_batch_id: Option<BatchId>,
    /// 当前 active batch 中的消息 ID。
    pub active_delivery_ids: Vec<DeliveryId>,
    /// 因 close 而 durable 丢弃的消息 ID。
    pub discarded_delivery_ids: Vec<DeliveryId>,
}

impl MailboxProjection {
    /// 从 durable 事件流重建指定 agent 的 MailboxProjection。
    ///
    /// 遍历所有事件，只处理与 `target_agent_id` 相关的 mailbox 事件：
    /// - `Queued` 按 `to_agent_id` 过滤（消息是发给谁的）
    /// - `BatchStarted/BatchAcked/Discarded` 按 `target_agent_id` 过滤（谁在消费/丢弃）
    pub fn replay_for_agent(events: &[StoredEvent], target_agent_id: &str) -> Self {
        let mut projection = Self::default();
        for stored in events {
            Self::apply_event_for_agent(&mut projection, stored, target_agent_id);
        }

        projection
    }

    /// 从完整 durable 事件流重建按目标 agent 组织的 mailbox 投影索引。
    pub fn replay_index(events: &[StoredEvent]) -> HashMap<String, MailboxProjection> {
        let mut index = HashMap::new();
        for stored in events {
            match &stored.event.payload {
                crate::StorageEventPayload::AgentMailboxQueued { payload } => {
                    let projection = index
                        .entry(payload.envelope.to_agent_id.clone())
                        .or_insert_with(MailboxProjection::default);
                    Self::apply_event_for_agent(projection, stored, &payload.envelope.to_agent_id);
                },
                crate::StorageEventPayload::AgentMailboxBatchStarted { payload } => {
                    let projection = index
                        .entry(payload.target_agent_id.clone())
                        .or_insert_with(MailboxProjection::default);
                    Self::apply_event_for_agent(projection, stored, &payload.target_agent_id);
                },
                crate::StorageEventPayload::AgentMailboxBatchAcked { payload } => {
                    let projection = index
                        .entry(payload.target_agent_id.clone())
                        .or_insert_with(MailboxProjection::default);
                    Self::apply_event_for_agent(projection, stored, &payload.target_agent_id);
                },
                crate::StorageEventPayload::AgentMailboxDiscarded { payload } => {
                    let projection = index
                        .entry(payload.target_agent_id.clone())
                        .or_insert_with(MailboxProjection::default);
                    Self::apply_event_for_agent(projection, stored, &payload.target_agent_id);
                },
                _ => {},
            }
        }
        index
    }

    /// 将单条 durable mailbox 事件应用到指定目标 agent 的投影。
    pub fn apply_event_for_agent(
        projection: &mut MailboxProjection,
        stored: &StoredEvent,
        target_agent_id: &str,
    ) {
        use crate::StorageEventPayload;

        match &stored.event.payload {
            StorageEventPayload::AgentMailboxQueued { payload } => {
                if payload.envelope.to_agent_id != target_agent_id {
                    return;
                }
                let id = &payload.envelope.delivery_id;
                if !projection.discarded_delivery_ids.contains(id)
                    && !projection.pending_delivery_ids.contains(id)
                {
                    projection.pending_delivery_ids.push(id.clone());
                }
            },
            StorageEventPayload::AgentMailboxBatchStarted { payload } => {
                if payload.target_agent_id != target_agent_id {
                    return;
                }
                projection.active_batch_id = Some(payload.batch_id.clone());
                projection.active_delivery_ids = payload.delivery_ids.clone();
            },
            StorageEventPayload::AgentMailboxBatchAcked { payload } => {
                if payload.target_agent_id != target_agent_id {
                    return;
                }
                let acked_set: std::collections::HashSet<_> = payload.delivery_ids.iter().collect();
                projection.pending_delivery_ids.retain(|id| {
                    !acked_set.contains(id) && !projection.discarded_delivery_ids.contains(id)
                });
                if projection.active_batch_id.as_deref() == Some(&payload.batch_id) {
                    projection.active_batch_id = None;
                    projection.active_delivery_ids.clear();
                }
            },
            StorageEventPayload::AgentMailboxDiscarded { payload } => {
                if payload.target_agent_id != target_agent_id {
                    return;
                }
                for id in &payload.delivery_ids {
                    if !projection.discarded_delivery_ids.contains(id) {
                        projection.discarded_delivery_ids.push(id.clone());
                    }
                }
                projection
                    .pending_delivery_ids
                    .retain(|id| !projection.discarded_delivery_ids.contains(id));
                let discarded_set: std::collections::HashSet<_> =
                    projection.discarded_delivery_ids.iter().collect();
                if projection
                    .active_delivery_ids
                    .iter()
                    .any(|id| discarded_set.contains(id))
                {
                    projection.active_batch_id = None;
                    projection.active_delivery_ids.clear();
                }
            },
            _ => {},
        }
    }

    /// 返回当前待处理消息数量。
    pub fn pending_message_count(&self) -> usize {
        self.pending_delivery_ids.len()
    }
}

// ── Observe 快照结果 ──────────────────────────────────────────────

/// `observe` 工具返回的目标 Agent 查询结果。
///
/// 融合 live control state、对话投影和 mailbox 派生信息。
/// 是读模型而非领域实体。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ObserveAgentResult {
    pub agent_id: String,
    pub sub_run_id: String,
    pub session_id: String,
    pub open_session_id: String,
    pub parent_agent_id: String,
    /// 当前生命周期状态。
    pub lifecycle_status: AgentLifecycleStatus,
    /// 最近一轮执行结果。
    pub last_turn_outcome: Option<AgentTurnOutcome>,
    /// 对话阶段（来自现有 AgentStateProjector）。
    pub phase: String,
    /// 当前轮次数。
    pub turn_count: u32,
    /// durable replay 为准的待处理消息数量。
    pub pending_message_count: usize,
    /// 当前正在处理的任务摘要。
    pub active_task: Option<String>,
    /// 下一条待处理任务摘要。
    pub pending_task: Option<String>,
    /// 最近几条 mailbox 消息摘要，仅用于帮助判断最近协作上下文。
    ///
    /// 这是 tail view，不是全量 mailbox dump，避免 observe 结果过长。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_mailbox_messages: Vec<String>,
    /// 最近 assistant 输出摘要。
    pub last_output: Option<String>,
    /// 面向下一步决策的建议动作。
    ///
    /// 这是 advisory projection，不是新的业务真相；
    /// 调用方仍应以 lifecycle/outcome 等原始事实为准。
    pub recommended_next_action: String,
    /// 对建议动作的简短说明。
    pub recommended_reason: String,
    /// 交付新鲜度投影，帮助调用方判断是继续等待还是立即处理。
    pub delivery_freshness: String,
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

    #[test]
    fn mailbox_projection_replay_tracks_full_lifecycle() {
        use crate::{StorageEvent, StorageEventPayload, StoredEvent};

        let agent = crate::AgentEventContext::default();
        let events = vec![
            StoredEvent {
                storage_seq: 1,
                event: StorageEvent {
                    turn_id: Some("t1".into()),
                    agent: agent.clone(),
                    payload: StorageEventPayload::AgentMailboxQueued {
                        payload: MailboxQueuedPayload {
                            envelope: AgentMailboxEnvelope {
                                delivery_id: "d1".into(),
                                from_agent_id: "parent".into(),
                                to_agent_id: "child".into(),
                                message: "hello".into(),
                                queued_at: chrono::Utc::now(),
                                sender_lifecycle_status:
                                    crate::agent::lifecycle::AgentLifecycleStatus::Running,
                                sender_last_turn_outcome: None,
                                sender_open_session_id: "s-parent".into(),
                            },
                        },
                    },
                },
            },
            StoredEvent {
                storage_seq: 2,
                event: StorageEvent {
                    turn_id: Some("t2".into()),
                    agent: agent.clone(),
                    payload: StorageEventPayload::AgentMailboxBatchStarted {
                        payload: MailboxBatchStartedPayload {
                            target_agent_id: "child".into(),
                            turn_id: "t2".into(),
                            batch_id: "b1".into(),
                            delivery_ids: vec!["d1".into()],
                        },
                    },
                },
            },
            StoredEvent {
                storage_seq: 3,
                event: StorageEvent {
                    turn_id: Some("t2".into()),
                    agent: agent.clone(),
                    payload: StorageEventPayload::AgentMailboxBatchAcked {
                        payload: MailboxBatchAckedPayload {
                            target_agent_id: "child".into(),
                            turn_id: "t2".into(),
                            batch_id: "b1".into(),
                            delivery_ids: vec!["d1".into()],
                        },
                    },
                },
            },
        ];

        let projection = MailboxProjection::replay_for_agent(&events, "child");
        assert!(projection.pending_delivery_ids.is_empty());
        assert!(projection.active_batch_id.is_none());
        assert!(projection.active_delivery_ids.is_empty());
        assert_eq!(projection.pending_message_count(), 0);
    }

    #[test]
    fn mailbox_projection_replay_tracks_discarded() {
        use crate::{StorageEvent, StorageEventPayload, StoredEvent};

        let agent = crate::AgentEventContext::default();
        let events = vec![
            StoredEvent {
                storage_seq: 1,
                event: StorageEvent {
                    turn_id: Some("t1".into()),
                    agent: agent.clone(),
                    payload: StorageEventPayload::AgentMailboxQueued {
                        payload: MailboxQueuedPayload {
                            envelope: AgentMailboxEnvelope {
                                delivery_id: "d1".into(),
                                from_agent_id: "parent".into(),
                                to_agent_id: "child".into(),
                                message: "hello".into(),
                                queued_at: chrono::Utc::now(),
                                sender_lifecycle_status:
                                    crate::agent::lifecycle::AgentLifecycleStatus::Running,
                                sender_last_turn_outcome: None,
                                sender_open_session_id: "s-parent".into(),
                            },
                        },
                    },
                },
            },
            StoredEvent {
                storage_seq: 2,
                event: StorageEvent {
                    turn_id: Some("t1".into()),
                    agent: agent.clone(),
                    payload: StorageEventPayload::AgentMailboxDiscarded {
                        payload: MailboxDiscardedPayload {
                            target_agent_id: "child".into(),
                            delivery_ids: vec!["d1".into()],
                        },
                    },
                },
            },
        ];

        let projection = MailboxProjection::replay_for_agent(&events, "child");
        assert!(projection.pending_delivery_ids.is_empty());
        assert!(
            projection
                .discarded_delivery_ids
                .contains(&"d1".to_string())
        );
    }

    #[test]
    fn mailbox_projection_started_but_not_acked_keeps_pending() {
        use crate::{StorageEvent, StorageEventPayload, StoredEvent};

        let agent = crate::AgentEventContext::default();
        let events = vec![
            StoredEvent {
                storage_seq: 1,
                event: StorageEvent {
                    turn_id: Some("t1".into()),
                    agent: agent.clone(),
                    payload: StorageEventPayload::AgentMailboxQueued {
                        payload: MailboxQueuedPayload {
                            envelope: AgentMailboxEnvelope {
                                delivery_id: "d1".into(),
                                from_agent_id: "parent".into(),
                                to_agent_id: "child".into(),
                                message: "hello".into(),
                                queued_at: chrono::Utc::now(),
                                sender_lifecycle_status:
                                    crate::agent::lifecycle::AgentLifecycleStatus::Running,
                                sender_last_turn_outcome: None,
                                sender_open_session_id: "s-parent".into(),
                            },
                        },
                    },
                },
            },
            StoredEvent {
                storage_seq: 2,
                event: StorageEvent {
                    turn_id: Some("t2".into()),
                    agent: agent.clone(),
                    payload: StorageEventPayload::AgentMailboxBatchStarted {
                        payload: MailboxBatchStartedPayload {
                            target_agent_id: "child".into(),
                            turn_id: "t2".into(),
                            batch_id: "b1".into(),
                            delivery_ids: vec!["d1".into()],
                        },
                    },
                },
            },
        ];

        let projection = MailboxProjection::replay_for_agent(&events, "child");
        // Started 但未 Acked，d1 仍在 pending 中（at-least-once 语义）
        assert!(projection.pending_delivery_ids.contains(&"d1".to_string()));
        assert_eq!(projection.active_batch_id.as_deref(), Some("b1"));
        assert_eq!(projection.pending_message_count(), 1);
    }

    #[test]
    fn mailbox_projection_per_agent_filtering_isolates_agents() {
        use crate::{StorageEvent, StorageEventPayload, StoredEvent};

        let agent = crate::AgentEventContext::default();
        // 给 agent-a 和 agent-b 各发一条消息
        let events = vec![
            StoredEvent {
                storage_seq: 1,
                event: StorageEvent {
                    turn_id: Some("t1".into()),
                    agent: agent.clone(),
                    payload: StorageEventPayload::AgentMailboxQueued {
                        payload: MailboxQueuedPayload {
                            envelope: AgentMailboxEnvelope {
                                delivery_id: "d-a".into(),
                                from_agent_id: "parent".into(),
                                to_agent_id: "agent-a".into(),
                                message: "for a".into(),
                                queued_at: chrono::Utc::now(),
                                sender_lifecycle_status:
                                    crate::agent::lifecycle::AgentLifecycleStatus::Running,
                                sender_last_turn_outcome: None,
                                sender_open_session_id: "s-parent".into(),
                            },
                        },
                    },
                },
            },
            StoredEvent {
                storage_seq: 2,
                event: StorageEvent {
                    turn_id: Some("t1".into()),
                    agent: agent.clone(),
                    payload: StorageEventPayload::AgentMailboxQueued {
                        payload: MailboxQueuedPayload {
                            envelope: AgentMailboxEnvelope {
                                delivery_id: "d-b".into(),
                                from_agent_id: "parent".into(),
                                to_agent_id: "agent-b".into(),
                                message: "for b".into(),
                                queued_at: chrono::Utc::now(),
                                sender_lifecycle_status:
                                    crate::agent::lifecycle::AgentLifecycleStatus::Running,
                                sender_last_turn_outcome: None,
                                sender_open_session_id: "s-parent".into(),
                            },
                        },
                    },
                },
            },
        ];

        let projection_a = MailboxProjection::replay_for_agent(&events, "agent-a");
        assert_eq!(projection_a.pending_delivery_ids, vec!["d-a".to_string()]);
        assert_eq!(projection_a.pending_message_count(), 1);

        let projection_b = MailboxProjection::replay_for_agent(&events, "agent-b");
        assert_eq!(projection_b.pending_delivery_ids, vec!["d-b".to_string()]);
        assert_eq!(projection_b.pending_message_count(), 1);

        let projection_c = MailboxProjection::replay_for_agent(&events, "agent-c");
        assert_eq!(projection_c.pending_message_count(), 0);
    }
}
