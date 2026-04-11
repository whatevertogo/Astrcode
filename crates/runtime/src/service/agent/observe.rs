//! # 四工具模型 — Observe 实现
//!
//! `observe` 是四工具模型（send / observe / close / interrupt）中的只读观察操作，
//! 允许父 agent 查看子 agent 的当前状态快照，包括：
//!
//! - 生命周期状态（Pending / Running / Idle / Terminated）
//! - 最近一轮执行结果（Completed / Failed / Cancelled / TokenExceeded）
//! - 当前正在处理的任务摘要（active_task）
//! - 下一条待处理任务摘要（pending_task）
//! - 最近 assistant 输出摘要（last_output）
//! - 待处理邮箱消息数量
//!
//! ## 快照聚合流程
//!
//! ```text
//! observe_child(params)
//!   ├─ 校验参数 & 鉴权（父 agent 必须拥有该 child）
//!   ├─ 从 agent_control 获取 lifecycle / turn_outcome
//!   ├─ 从 session_state 获取 projected_state（对话投影）
//!   ├─ 从 session_state 获取 mailbox_projection（邮箱投影）
//!   ├─ 从持久化事件重放 mailbox_messages（投递内容）
//!   └─ 聚合为 ObserveAgentResult → 返回 CollaborationResult
//! ```

use std::collections::{HashMap, HashSet};

use astrcode_core::{
    AgentLifecycleStatus, AgentMailboxEnvelope, CollaborationResult, CollaborationResultKind,
    LlmMessage, ObserveAgentResult, ObserveParams, StorageEventPayload, StoredEvent,
    UserMessageOrigin,
};

use super::AgentServiceHandle;
use crate::service::{ServiceError, ServiceResult};

impl AgentServiceHandle {
    /// 获取目标 child agent 的增强快照（四工具模型 observe）。
    ///
    /// 返回的 `CollaborationResult.summary` 中包含 JSON 序列化的 `ObserveAgentResult`，
    /// 上层可反序列化后展示子 agent 的完整状态。
    pub async fn observe_child(
        &self,
        params: ObserveParams,
        ctx: &astrcode_core::ToolContext,
    ) -> ServiceResult<CollaborationResult> {
        params.validate().map_err(ServiceError::from)?;

        let child = self
            .runtime
            .agent_control
            .get(&params.agent_id)
            .await
            .ok_or_else(|| {
                ServiceError::NotFound(format!("agent '{}' not found", params.agent_id))
            })?;

        self.verify_caller_owns_child(ctx, &child)?;

        let lifecycle_status = self
            .runtime
            .agent_control
            .get_lifecycle(&params.agent_id)
            .await
            .unwrap_or(AgentLifecycleStatus::Pending);

        let last_turn_outcome = self
            .runtime
            .agent_control
            .get_turn_outcome(&params.agent_id)
            .await
            .flatten();

        let open_session_id = child
            .child_session_id
            .clone()
            .unwrap_or_else(|| child.session_id.clone());
        let session_state = self.runtime.ensure_session_loaded(&open_session_id).await?;
        let projected = session_state.snapshot_projected_state().map_err(|error| {
            ServiceError::Internal(astrcode_core::AstrError::Internal(error.to_string()))
        })?;

        let mailbox_projection = session_state
            .mailbox_projection_for_agent(&params.agent_id)
            .unwrap_or_default();
        let pending_message_count = mailbox_projection.pending_message_count();
        let stored_events = crate::service::session::load_events(
            self.runtime.session_manager.clone(),
            &open_session_id,
        )
        .await?;
        let mailbox_messages = collect_mailbox_messages(&stored_events, &params.agent_id);

        let observe_result = ObserveAgentResult {
            agent_id: child.agent_id.clone(),
            sub_run_id: child.sub_run_id.clone(),
            session_id: child.session_id.clone(),
            open_session_id,
            parent_agent_id: child.parent_agent_id.clone().unwrap_or_default(),
            lifecycle_status,
            last_turn_outcome,
            phase: format!("{:?}", projected.phase),
            turn_count: projected.turn_count as u32,
            pending_message_count,
            active_task: active_task_summary(
                lifecycle_status,
                &projected.messages,
                &mailbox_projection,
                &mailbox_messages,
            ),
            pending_task: pending_task_summary(&mailbox_projection, &mailbox_messages),
            last_output: extract_last_output(&projected.messages),
        };

        log::info!(
            "observe: snapshot for child agent '{}' (lifecycle={:?}, pending={})",
            params.agent_id,
            lifecycle_status,
            pending_message_count
        );

        Ok(CollaborationResult {
            accepted: true,
            kind: CollaborationResultKind::Observed,
            agent_ref: Some(
                self.project_child_ref_status(self.build_child_ref_from_handle(&child).await)
                    .await,
            ),
            delivery_id: None,
            summary: Some(serde_json::to_string(&observe_result).unwrap_or_default()),
            cascade: None,
            closed_root_agent_id: None,
            failure: None,
        })
    }
}

/// 从消息列表中提取最后一条 assistant 消息的输出摘要（最多 200 字符）。
pub(super) fn extract_last_output(messages: &[astrcode_core::LlmMessage]) -> Option<String> {
    messages.iter().rev().find_map(|msg| match msg {
        astrcode_core::LlmMessage::Assistant { content, .. } => {
            if content.is_empty() {
                None
            } else if content.len() > 200 {
                Some(format!("{}...", &content[..200]))
            } else {
                Some(content.clone())
            }
        },
        _ => None,
    })
}

/// 推断当前正在处理的任务摘要。
///
/// 优先级：活跃投递的消息摘要 > Pending/Running 状态下最近用户消息摘要。
fn active_task_summary(
    lifecycle_status: AgentLifecycleStatus,
    messages: &[LlmMessage],
    mailbox_projection: &astrcode_core::MailboxProjection,
    mailbox_messages: &HashMap<String, AgentMailboxEnvelope>,
) -> Option<String> {
    if let Some(summary) = first_delivery_summary(
        mailbox_projection.active_delivery_ids.iter(),
        mailbox_messages,
    ) {
        return Some(summary);
    }

    if matches!(
        lifecycle_status,
        AgentLifecycleStatus::Pending | AgentLifecycleStatus::Running
    ) {
        return latest_user_task_summary(messages);
    }

    None
}

/// 推断下一条待处理任务摘要。
///
/// 从 pending_delivery_ids 中排除已处于 active 的投递，取第一条的消息摘要。
fn pending_task_summary(
    mailbox_projection: &astrcode_core::MailboxProjection,
    mailbox_messages: &HashMap<String, AgentMailboxEnvelope>,
) -> Option<String> {
    let active_ids = mailbox_projection
        .active_delivery_ids
        .iter()
        .cloned()
        .collect::<HashSet<_>>();

    first_delivery_summary(
        mailbox_projection
            .pending_delivery_ids
            .iter()
            .filter(|delivery_id| !active_ids.contains(*delivery_id)),
        mailbox_messages,
    )
}

/// 从投递 ID 列表中找到第一条有内容的邮箱消息摘要。
fn first_delivery_summary<'a>(
    delivery_ids: impl IntoIterator<Item = &'a String>,
    mailbox_messages: &HashMap<String, AgentMailboxEnvelope>,
) -> Option<String> {
    delivery_ids.into_iter().find_map(|delivery_id| {
        mailbox_messages
            .get(delivery_id)
            .and_then(|envelope| summarize_task_text(&envelope.message))
    })
}

/// 从消息列表倒序查找最近一条 User 消息的任务摘要。
fn latest_user_task_summary(messages: &[LlmMessage]) -> Option<String> {
    messages.iter().rev().find_map(|message| match message {
        LlmMessage::User { content, origin } if *origin == UserMessageOrigin::User => {
            summarize_task_text(content)
        },
        _ => None,
    })
}

/// 从持久化事件中提取目标 agent 的所有邮箱投递信封（delivery_id → envelope）。
fn collect_mailbox_messages(
    stored_events: &[StoredEvent],
    target_agent_id: &str,
) -> HashMap<String, AgentMailboxEnvelope> {
    let mut messages = HashMap::new();

    for stored in stored_events {
        if let StorageEventPayload::AgentMailboxQueued { payload } = &stored.event.payload {
            if payload.envelope.to_agent_id == target_agent_id {
                messages.insert(
                    payload.envelope.delivery_id.clone(),
                    payload.envelope.clone(),
                );
            }
        }
    }

    messages
}

/// 将任务文本标准化并截断为最多 120 字符的摘要。
fn summarize_task_text(text: &str) -> Option<String> {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = normalized.trim();
    if trimmed.is_empty() {
        return None;
    }

    const MAX_TASK_SUMMARY_CHARS: usize = 120;
    let char_count = trimmed.chars().count();
    if char_count <= MAX_TASK_SUMMARY_CHARS {
        return Some(trimmed.to_string());
    }

    Some(
        trimmed
            .chars()
            .take(MAX_TASK_SUMMARY_CHARS)
            .collect::<String>()
            + "...",
    )
}

#[cfg(test)]
mod tests {
    use astrcode_core::{
        AgentLifecycleStatus, AgentMailboxEnvelope, LlmMessage, MailboxProjection, StoredEvent,
        UserMessageOrigin,
    };

    use super::{
        active_task_summary, collect_mailbox_messages, extract_last_output, pending_task_summary,
        summarize_task_text,
    };

    #[test]
    fn summarize_task_text_trims_whitespace_and_truncates() {
        assert_eq!(
            summarize_task_text("  review   the   mailbox state  "),
            Some("review the mailbox state".to_string())
        );
        assert!(summarize_task_text("   ").is_none());
        let long = "a".repeat(150);
        assert_eq!(
            summarize_task_text(&long),
            Some(format!("{}...", "a".repeat(120)))
        );
    }

    #[test]
    fn observe_task_summaries_prefer_active_batch_then_pending_queue() {
        let delivery_active = "delivery-active".to_string();
        let delivery_pending = "delivery-pending".to_string();
        let mailbox_messages = HashMap::from([
            (
                delivery_active.clone(),
                AgentMailboxEnvelope {
                    delivery_id: delivery_active.clone(),
                    from_agent_id: "parent".to_string(),
                    to_agent_id: "child".to_string(),
                    message: "继续修复 runtime observe 缺口".to_string(),
                    queued_at: chrono::Utc::now(),
                    sender_lifecycle_status: AgentLifecycleStatus::Idle,
                    sender_last_turn_outcome: None,
                    sender_open_session_id: "session-parent".to_string(),
                },
            ),
            (
                delivery_pending.clone(),
                AgentMailboxEnvelope {
                    delivery_id: delivery_pending.clone(),
                    from_agent_id: "parent".to_string(),
                    to_agent_id: "child".to_string(),
                    message: "然后补前端类型和测试".to_string(),
                    queued_at: chrono::Utc::now(),
                    sender_lifecycle_status: AgentLifecycleStatus::Idle,
                    sender_last_turn_outcome: None,
                    sender_open_session_id: "session-parent".to_string(),
                },
            ),
        ]);
        let mailbox_projection = MailboxProjection {
            pending_delivery_ids: vec![delivery_active.clone(), delivery_pending.clone()],
            active_batch_id: Some("batch-1".to_string()),
            active_delivery_ids: vec![delivery_active],
            discarded_delivery_ids: Vec::new(),
        };

        let messages = vec![LlmMessage::User {
            content: "最初任务".to_string(),
            origin: UserMessageOrigin::User,
        }];

        assert_eq!(
            active_task_summary(
                AgentLifecycleStatus::Running,
                &messages,
                &mailbox_projection,
                &mailbox_messages,
            ),
            Some("继续修复 runtime observe 缺口".to_string())
        );
        assert_eq!(
            pending_task_summary(&mailbox_projection, &mailbox_messages),
            Some("然后补前端类型和测试".to_string())
        );
    }

    #[test]
    fn running_observe_without_active_batch_falls_back_to_latest_user_task() {
        let messages = vec![
            LlmMessage::User {
                content: "内部唤醒".to_string(),
                origin: UserMessageOrigin::ReactivationPrompt,
            },
            LlmMessage::User {
                content: "整理 close 幂等性文档".to_string(),
                origin: UserMessageOrigin::User,
            },
        ];

        assert_eq!(
            active_task_summary(
                AgentLifecycleStatus::Running,
                &messages,
                &MailboxProjection::default(),
                &HashMap::new(),
            ),
            Some("整理 close 幂等性文档".to_string())
        );
    }

    #[test]
    fn collect_mailbox_messages_only_keeps_target_agent_envelopes() {
        let stored_events = vec![
            StoredEvent {
                storage_seq: 1,
                event: astrcode_core::StorageEvent {
                    turn_id: Some("turn-1".to_string()),
                    agent: astrcode_core::AgentEventContext::default(),
                    payload: astrcode_core::StorageEventPayload::AgentMailboxQueued {
                        payload: astrcode_core::MailboxQueuedPayload {
                            envelope: AgentMailboxEnvelope {
                                delivery_id: "delivery-child".to_string(),
                                from_agent_id: "parent".to_string(),
                                to_agent_id: "child".to_string(),
                                message: "给 child".to_string(),
                                queued_at: chrono::Utc::now(),
                                sender_lifecycle_status: AgentLifecycleStatus::Idle,
                                sender_last_turn_outcome: None,
                                sender_open_session_id: "session-parent".to_string(),
                            },
                        },
                    },
                },
            },
            StoredEvent {
                storage_seq: 2,
                event: astrcode_core::StorageEvent {
                    turn_id: Some("turn-2".to_string()),
                    agent: astrcode_core::AgentEventContext::default(),
                    payload: astrcode_core::StorageEventPayload::AgentMailboxQueued {
                        payload: astrcode_core::MailboxQueuedPayload {
                            envelope: AgentMailboxEnvelope {
                                delivery_id: "delivery-other".to_string(),
                                from_agent_id: "parent".to_string(),
                                to_agent_id: "other".to_string(),
                                message: "给 other".to_string(),
                                queued_at: chrono::Utc::now(),
                                sender_lifecycle_status: AgentLifecycleStatus::Idle,
                                sender_last_turn_outcome: None,
                                sender_open_session_id: "session-parent".to_string(),
                            },
                        },
                    },
                },
            },
        ];

        let messages = collect_mailbox_messages(&stored_events, "child");
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages
                .get("delivery-child")
                .map(|envelope| envelope.message.as_str()),
            Some("给 child")
        );
    }

    #[test]
    fn extract_last_output_ignores_empty_assistant_messages() {
        let messages = vec![
            LlmMessage::Assistant {
                content: "".to_string(),
                tool_calls: Vec::new(),
                reasoning: None,
            },
            LlmMessage::Assistant {
                content: "最后输出".to_string(),
                tool_calls: Vec::new(),
                reasoning: None,
            },
        ];

        assert_eq!(extract_last_output(&messages), Some("最后输出".to_string()));
    }
}
