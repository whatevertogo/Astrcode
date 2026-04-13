//! mailbox 构造与 durable 追加辅助。
//!
//! 从旧 runtime/service/agent/mailbox.rs 迁入，去掉对 RuntimeService 的依赖，
//! 改为通过 Kernel + SessionState 完成所有操作。

use astrcode_core::{
    AgentEventContext, AgentInboxEnvelope, AgentLifecycleStatus, AgentMailboxEnvelope, AstrError,
    MailboxDiscardedPayload, MailboxQueuedPayload, Result, SessionId, SubRunHandle, ToolContext,
};

use super::AgentOrchestrationService;

/// 将待处理的 inbox 信封与新的 send 输入拼接为 resume 消息。
///
/// 纯函数，不持有状态，便于独立测试。
pub(super) fn compose_reusable_child_message(
    pending: &[astrcode_core::AgentInboxEnvelope],
    params: &astrcode_core::SendAgentParams,
) -> String {
    let mut parts = pending
        .iter()
        .filter(|envelope| {
            matches!(
                envelope.kind,
                astrcode_core::InboxEnvelopeKind::ParentMessage
            )
        })
        .map(render_parent_message_envelope)
        .collect::<Vec<_>>();
    parts.push(render_parent_message_input(
        params.message.as_str(),
        params.context.as_deref(),
    ));

    if parts.len() == 1 {
        return parts.pop().unwrap_or_default();
    }

    let enumerated = parts
        .into_iter()
        .enumerate()
        .map(|(index, part)| format!("{}. {}", index + 1, part))
        .collect::<Vec<_>>()
        .join("\n\n");
    format!("请按顺序处理以下追加要求：\n\n{enumerated}")
}

pub(super) fn render_parent_message_envelope(
    envelope: &astrcode_core::AgentInboxEnvelope,
) -> String {
    render_parent_message_input(envelope.message.as_str(), envelope.context.as_deref())
}

pub(super) fn render_parent_message_input(message: &str, context: Option<&str>) -> String {
    match context {
        Some(context) if !context.trim().is_empty() => {
            format!("{message}\n\n补充上下文：{context}")
        },
        _ => message.to_string(),
    }
}

impl AgentOrchestrationService {
    /// 向 child session 的 durable event log 追加一条 MailboxQueued 事件。
    pub(super) async fn append_durable_mailbox_queue(
        &self,
        child: &SubRunHandle,
        envelope: &AgentInboxEnvelope,
        ctx: &ToolContext,
    ) -> Result<()> {
        let target_session_id = child
            .child_session_id
            .clone()
            .unwrap_or_else(|| child.session_id.clone());
        let session_state = self
            .session_runtime
            .get_session_state(&SessionId::from(
                astrcode_session_runtime::normalize_session_id(&target_session_id),
            ))
            .await
            .map_err(|e| AstrError::Internal(e.to_string()))?;

        let sender_agent_id = ctx.agent_context().agent_id.clone().unwrap_or_default();
        let sender_lifecycle_status = if sender_agent_id.is_empty() {
            AgentLifecycleStatus::Running
        } else {
            self.kernel
                .get_agent_lifecycle(&sender_agent_id)
                .await
                .unwrap_or(AgentLifecycleStatus::Running)
        };
        let sender_last_turn_outcome = if sender_agent_id.is_empty() {
            None
        } else {
            self.kernel.get_agent_turn_outcome(&sender_agent_id).await
        };
        let sender_open_session_id = ctx
            .agent_context()
            .child_session_id
            .clone()
            .unwrap_or_else(|| ctx.session_id().to_string());

        let payload = MailboxQueuedPayload {
            envelope: AgentMailboxEnvelope {
                delivery_id: envelope.delivery_id.clone(),
                from_agent_id: envelope.from_agent_id.clone(),
                to_agent_id: envelope.to_agent_id.clone(),
                message: render_parent_message_input(
                    &envelope.message,
                    envelope.context.as_deref(),
                ),
                queued_at: chrono::Utc::now(),
                sender_lifecycle_status,
                sender_last_turn_outcome,
                sender_open_session_id,
            },
        };

        let event_agent = AgentEventContext::sub_run(
            child.agent_id.clone(),
            child.parent_turn_id.clone(),
            child.agent_profile.clone(),
            child.sub_run_id.clone(),
            child.parent_sub_run_id.clone(),
            child.storage_mode,
            child.child_session_id.clone(),
        );
        let mut translator = astrcode_core::EventTranslator::new(session_state.current_phase()?);
        astrcode_session_runtime::append_mailbox_queued(
            &session_state,
            ctx.turn_id().unwrap_or(&child.parent_turn_id),
            event_agent,
            payload,
            &mut translator,
        )
        .await
        .map_err(|error| AstrError::Internal(error.to_string()))?;
        Ok(())
    }

    /// 批量追加 MailboxDiscarded 事件。
    pub(super) async fn append_durable_mailbox_discard_batch(
        &self,
        handles: &[SubRunHandle],
        ctx: &ToolContext,
    ) -> Result<()> {
        for handle in handles {
            self.append_durable_mailbox_discard(handle, ctx).await?;
        }
        Ok(())
    }

    /// 向 child session 的 durable event log 追加一条 MailboxDiscarded 事件。
    async fn append_durable_mailbox_discard(
        &self,
        handle: &SubRunHandle,
        ctx: &ToolContext,
    ) -> Result<()> {
        let target_session_id = handle
            .child_session_id
            .clone()
            .unwrap_or_else(|| handle.session_id.clone());
        let session_state = self
            .session_runtime
            .get_session_state(&SessionId::from(
                astrcode_session_runtime::normalize_session_id(&target_session_id),
            ))
            .await
            .map_err(|e| AstrError::Internal(e.to_string()))?;

        let projection = session_state
            .mailbox_projection_for_agent(&handle.agent_id)
            .map_err(|error| AstrError::Internal(error.to_string()))?;
        if projection.pending_delivery_ids.is_empty() {
            return Ok(());
        }

        let payload = MailboxDiscardedPayload {
            target_agent_id: handle.agent_id.clone(),
            delivery_ids: projection.pending_delivery_ids,
        };
        // 为什么使用默认上下文：discard payload 已经自带 target_agent_id
        let event_agent = AgentEventContext::default();
        let mut translator = astrcode_core::EventTranslator::new(session_state.current_phase()?);
        astrcode_session_runtime::append_mailbox_discarded(
            &session_state,
            ctx.turn_id().unwrap_or(&handle.parent_turn_id),
            event_agent,
            payload,
            &mut translator,
        )
        .await
        .map_err(|error| AstrError::Internal(error.to_string()))?;
        Ok(())
    }
}
