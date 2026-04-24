use super::*;

impl AgentOrchestrationService {
    pub(in crate::agent) async fn build_explicit_parent_delivery_notification(
        &self,
        child: &SubRunHandle,
        payload: &ParentDeliveryPayload,
        ctx: &astrcode_tool_contract::ToolContext,
        source_turn_id: &str,
    ) -> ChildSessionNotification {
        let status = self
            .kernel
            .get_lifecycle(&child.agent_id)
            .await
            .unwrap_or(child.lifecycle);
        let notification_id = explicit_parent_delivery_id(
            &child.sub_run_id,
            source_turn_id,
            ctx.tool_call_id().map(ToString::to_string).as_deref(),
            payload,
        );

        ChildSessionNotification {
            notification_id: notification_id.clone().into(),
            child_ref: child.child_ref_with_status(status),
            kind: payload.notification_kind(),
            source_tool_call_id: ctx.tool_call_id().map(ToString::to_string).map(Into::into),
            delivery: Some(ParentDelivery {
                idempotency_key: notification_id,
                origin: ParentDeliveryOrigin::Explicit,
                terminal_semantics: payload.terminal_semantics(),
                source_turn_id: Some(source_turn_id.to_string()),
                payload: payload.clone(),
            }),
        }
    }

    pub(in crate::agent) async fn append_durable_input_queue(
        &self,
        child: &SubRunHandle,
        envelope: &AgentInboxEnvelope,
        ctx: &astrcode_tool_contract::ToolContext,
    ) -> astrcode_core::Result<()> {
        let target_session_id = child
            .child_session_id
            .clone()
            .unwrap_or_else(|| child.session_id.clone())
            .to_string();

        let sender_agent_id = ctx.agent_context().agent_id.clone().unwrap_or_default();
        let sender_lifecycle_status = if sender_agent_id.is_empty() {
            AgentLifecycleStatus::Running
        } else {
            self.kernel
                .get_lifecycle(&sender_agent_id)
                .await
                .unwrap_or(AgentLifecycleStatus::Running)
        };
        let sender_last_turn_outcome = if sender_agent_id.is_empty() {
            None
        } else {
            self.kernel.get_turn_outcome(&sender_agent_id).await
        };
        let sender_open_session_id = ctx
            .agent_context()
            .child_session_id
            .clone()
            .unwrap_or_else(|| ctx.session_id().to_string().into());

        let payload = InputQueuedPayload {
            envelope: astrcode_core::QueuedInputEnvelope {
                delivery_id: envelope.delivery_id.clone().into(),
                from_agent_id: envelope.from_agent_id.clone(),
                to_agent_id: envelope.to_agent_id.clone(),
                message: render_parent_message_input(
                    &envelope.message,
                    envelope.context.as_deref(),
                ),
                queued_at: chrono::Utc::now(),
                sender_lifecycle_status,
                sender_last_turn_outcome,
                sender_open_session_id: sender_open_session_id.to_string(),
            },
        };

        self.session_runtime
            .append_agent_input_queued(
                &target_session_id,
                ctx.turn_id().unwrap_or(child.parent_turn_id.as_str()),
                subrun_event_context(child),
                payload,
            )
            .await?;
        Ok(())
    }

    pub(in crate::agent) async fn append_durable_input_queue_discard_batch(
        &self,
        handles: &[SubRunHandle],
        ctx: &astrcode_tool_contract::ToolContext,
    ) -> astrcode_core::Result<()> {
        for handle in handles {
            self.append_durable_input_queue_discard(handle, ctx).await?;
        }
        Ok(())
    }

    async fn append_durable_input_queue_discard(
        &self,
        handle: &SubRunHandle,
        ctx: &astrcode_tool_contract::ToolContext,
    ) -> astrcode_core::Result<()> {
        let target_session_id = handle
            .child_session_id
            .clone()
            .unwrap_or_else(|| handle.session_id.clone());
        let pending_delivery_ids = self
            .session_runtime
            .pending_delivery_ids_for_agent(&target_session_id, &handle.agent_id)
            .await?;
        if pending_delivery_ids.is_empty() {
            return Ok(());
        }

        self.session_runtime
            .append_agent_input_discarded(
                &target_session_id,
                ctx.turn_id().unwrap_or(&handle.parent_turn_id),
                astrcode_core::AgentEventContext::default(),
                InputDiscardedPayload {
                    target_agent_id: handle.agent_id.to_string(),
                    delivery_ids: pending_delivery_ids.into_iter().map(Into::into).collect(),
                },
            )
            .await?;
        Ok(())
    }
}

pub(super) fn parent_delivery_label(payload: &ParentDeliveryPayload) -> &'static str {
    match payload {
        ParentDeliveryPayload::Progress(_) => "progress",
        ParentDeliveryPayload::Completed(_) => "completed",
        ParentDeliveryPayload::Failed(_) => "failed",
        ParentDeliveryPayload::CloseRequest(_) => "close_request",
    }
}

fn explicit_parent_delivery_id(
    sub_run_id: &str,
    source_turn_id: &str,
    source_tool_call_id: Option<&str>,
    payload: &ParentDeliveryPayload,
) -> String {
    let tool_call_id = source_tool_call_id.unwrap_or("tool-call-missing");
    format!(
        "child-send:{sub_run_id}:{source_turn_id}:{tool_call_id}:{}",
        parent_delivery_label(payload)
    )
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
