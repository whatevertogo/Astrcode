//! # 四工具模型 — Observe 实现
//!
//! `observe` 是四工具模型（send / observe / close / interrupt）中的只读观察操作。
//! 从旧 runtime/service/agent/observe.rs 迁入，去掉对 RuntimeService 的依赖。
//!
//! 快照聚合三层：
//! 1. 从 kernel AgentControl 获取 lifecycle / turn_outcome
//! 2. 从 SessionState 获取 projected_state（对话投影）
//! 3. 从 durable 事件重放 mailbox_messages（投递内容）

use std::collections::{HashMap, HashSet};

use astrcode_core::{
    AgentLifecycleStatus, AgentMailboxEnvelope, CollaborationResult, CollaborationResultKind,
    LlmMessage, ObserveAgentResult, ObserveParams, SessionId, StorageEventPayload, StoredEvent,
    UserMessageOrigin,
};

use super::AgentOrchestrationService;

impl AgentOrchestrationService {
    /// 获取目标 child agent 的增强快照（四工具模型 observe）。
    pub async fn observe_child(
        &self,
        params: ObserveParams,
        ctx: &astrcode_core::ToolContext,
    ) -> Result<CollaborationResult, super::AgentOrchestrationError> {
        params
            .validate()
            .map_err(super::AgentOrchestrationError::from)?;

        let child = self
            .kernel
            .get_agent_handle(&params.agent_id)
            .await
            .ok_or_else(|| {
                super::AgentOrchestrationError::NotFound(format!(
                    "agent '{}' not found",
                    params.agent_id
                ))
            })?;

        self.verify_caller_owns_child(ctx, &child)?;

        let lifecycle_status = self
            .kernel
            .get_agent_lifecycle(&params.agent_id)
            .await
            .unwrap_or(AgentLifecycleStatus::Pending);

        let last_turn_outcome = self.kernel.get_agent_turn_outcome(&params.agent_id).await;

        let open_session_id = child
            .child_session_id
            .clone()
            .unwrap_or_else(|| child.session_id.clone());

        // 获取 session state
        let session_state = self
            .session_runtime
            .get_session_state(&SessionId::from(
                astrcode_session_runtime::normalize_session_id(&open_session_id),
            ))
            .await
            .map_err(|e| {
                super::AgentOrchestrationError::Internal(format!(
                    "failed to load session state: {e}"
                ))
            })?;

        let projected = session_state
            .snapshot_projected_state()
            .map_err(|error| super::AgentOrchestrationError::Internal(error.to_string()))?;

        let mailbox_projection = session_state
            .mailbox_projection_for_agent(&params.agent_id)
            .unwrap_or_default();
        let pending_message_count = mailbox_projection.pending_delivery_ids.len();

        // 从持久化事件重放 mailbox 信封
        let stored_events = self
            .session_runtime
            .replay_stored_events(&SessionId::from(
                astrcode_session_runtime::normalize_session_id(&open_session_id),
            ))
            .await
            .unwrap_or_default();
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

// ── 纯函数（便于独立测试）──────────────────────────────────────

/// 从消息列表中提取最后一条 assistant 消息的输出摘要（最多 200 字符）。
pub(super) fn extract_last_output(messages: &[astrcode_core::LlmMessage]) -> Option<String> {
    messages.iter().rev().find_map(|msg| match msg {
        astrcode_core::LlmMessage::Assistant { content, .. } => {
            if content.is_empty() {
                None
            } else {
                let char_count = content.chars().count();
                if char_count > 200 {
                    Some(content.chars().take(200).collect::<String>() + "...")
                } else {
                    Some(content.clone())
                }
            }
        },
        _ => None,
    })
}

/// 推断当前正在处理的任务摘要。
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
fn pending_task_summary(
    mailbox_projection: &astrcode_core::MailboxProjection,
    mailbox_messages: &HashMap<String, AgentMailboxEnvelope>,
) -> Option<String> {
    let active_ids: HashSet<_> = mailbox_projection
        .active_delivery_ids
        .iter()
        .cloned()
        .collect();

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

/// 从持久化事件中提取目标 agent 的所有邮箱投递信封。
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
    use super::*;

    #[test]
    fn summarize_task_text_trims_and_truncates() {
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
    fn extract_last_output_ignores_empty_assistant() {
        let messages = vec![
            astrcode_core::LlmMessage::Assistant {
                content: String::new(),
                tool_calls: Vec::new(),
                reasoning: None,
            },
            astrcode_core::LlmMessage::Assistant {
                content: "最后输出".to_string(),
                tool_calls: Vec::new(),
                reasoning: None,
            },
        ];
        assert_eq!(extract_last_output(&messages), Some("最后输出".to_string()));
    }
}
