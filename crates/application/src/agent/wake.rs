//! 父级 delivery 唤醒调度。
//!
//! 从旧 runtime/service/agent/wake.rs 迁入，去掉对 RuntimeService / lifecycle 的依赖，
//! 改为通过 Kernel + SessionRuntime 完成所有操作。
//!
//! wake 是四工具模型的关键补充：当子 agent 完成后，系统需要自动
//! 将子 agent 的执行结果投递到父 agent 的 inbox 并触发父级继续执行。

use astrcode_core::{
    AgentEventContext, AgentLifecycleStatus, AgentMailboxEnvelope, AgentTurnOutcome,
    MailboxQueuedPayload, SessionId,
};

use super::AgentOrchestrationService;

impl AgentOrchestrationService {
    /// 子 agent 完成时触发的父级唤醒入口。
    ///
    /// 流程：
    /// 1. 向父 session 的 durable event log 追加 MailboxQueued 事件
    /// 2. 向 kernel AgentControl 的 delivery queue 排入通知
    /// 3. 尝试 checkout delivery batch 并启动父级 wake turn
    pub async fn reactivate_parent_agent_if_idle(
        &self,
        parent_session_id: &str,
        parent_turn_id: &str,
        notification: &astrcode_core::ChildSessionNotification,
    ) {
        self.metrics.record_parent_reactivation_requested();
        let parent_session_id = astrcode_session_runtime::normalize_session_id(parent_session_id);

        // 1. 加载父 session state 并追加 durable mailbox queue
        let parent_session_id_for_state = parent_session_id.clone();
        let session_state = match self
            .session_runtime
            .get_session_state(&SessionId::from(parent_session_id_for_state.clone()))
            .await
        {
            Ok(state) => state,
            Err(error) => {
                log::warn!(
                    "failed to load parent session for mailbox queue append: parentSession='{}', \
                     childAgent='{}', error='{}'",
                    parent_session_id,
                    notification.child_ref.agent_id,
                    error
                );
                return;
            },
        };

        if let Err(error) = self
            .append_parent_delivery_mailbox_queue(&session_state, parent_turn_id, notification)
            .await
        {
            log::warn!(
                "failed to persist durable parent mailbox queue before wake: parentSession='{}', \
                 childAgent='{}', deliveryId='{}', error='{}'",
                parent_session_id,
                notification.child_ref.agent_id,
                notification.notification_id,
                error
            );
            return;
        }

        // 2. 向 kernel delivery queue 排入通知
        let queued = self
            .kernel
            .enqueue_child_delivery(
                parent_session_id.clone(),
                parent_turn_id.to_string(),
                notification.clone(),
            )
            .await;
        self.metrics.record_delivery_buffer_queued();
        if !queued {
            log::warn!(
                "failed to enqueue parent delivery: parentSession='{}', deliveryId='{}'",
                parent_session_id,
                notification.notification_id
            );
        }

        // 3. 尝试启动父级 wake turn
        if let Err(error) = self
            .try_start_parent_delivery_turn(&parent_session_id)
            .await
        {
            self.metrics.record_parent_reactivation_failed();
            log::warn!(
                "failed to schedule parent wake turn from child delivery: parentSession='{}', \
                 childAgent='{}', subRunId='{}', error='{}'",
                parent_session_id,
                notification.child_ref.agent_id,
                notification.child_ref.sub_run_id,
                error
            );
        }
        self.metrics.record_parent_reactivation_succeeded();
    }

    /// 尝试从 delivery queue 中 checkout 一批交付并启动父级 wake turn。
    pub async fn try_start_parent_delivery_turn(
        &self,
        parent_session_id: &str,
    ) -> Result<bool, super::AgentOrchestrationError> {
        let parent_session_id = astrcode_session_runtime::normalize_session_id(parent_session_id);

        let delivery_batch = self
            .kernel
            .checkout_parent_delivery_batch(&parent_session_id)
            .await
            .ok_or_else(|| {
                super::AgentOrchestrationError::Internal("no delivery batch available".to_string())
            })?;
        self.metrics.record_delivery_buffer_dequeued();
        self.metrics.record_delivery_buffer_wake_requested();

        let batch_delivery_ids: Vec<String> = delivery_batch
            .iter()
            .map(|d| d.delivery_id.clone())
            .collect();

        // 向父 session 提交 wake prompt
        let wake_prompt = build_wake_prompt_from_deliveries(&delivery_batch);
        let result = self
            .session_runtime
            .submit_prompt(
                &parent_session_id,
                wake_prompt,
                self.default_runtime_config(),
            )
            .await;

        match result {
            Ok(_) => {
                // consume delivery batch
                let consumed = self
                    .kernel
                    .consume_parent_delivery_batch(&parent_session_id, &batch_delivery_ids)
                    .await;
                if !consumed {
                    log::warn!(
                        "parent wake turn succeeded but delivery batch consume failed: \
                         parentSession='{}'",
                        parent_session_id
                    );
                }
                self.metrics.record_delivery_buffer_wake_succeeded();
                Ok(true)
            },
            Err(error) => {
                // requeue delivery batch 以便重试
                self.kernel
                    .requeue_parent_delivery_batch(&parent_session_id, &batch_delivery_ids)
                    .await;
                self.metrics.record_delivery_buffer_wake_failed();
                log::warn!(
                    "parent wake turn failed, requeued deliveries: parentSession='{}', error='{}'",
                    parent_session_id,
                    error
                );
                Err(super::AgentOrchestrationError::Internal(format!(
                    "wake turn submit failed: {error}"
                )))
            },
        }
    }

    /// 向父 session 追加 durable MailboxQueued 事件。
    async fn append_parent_delivery_mailbox_queue(
        &self,
        parent_session_state: &astrcode_session_runtime::SessionState,
        parent_turn_id: &str,
        notification: &astrcode_core::ChildSessionNotification,
    ) -> Result<(), super::AgentOrchestrationError> {
        let target_agent_id = notification
            .child_ref
            .parent_agent_id
            .clone()
            .ok_or_else(|| {
                super::AgentOrchestrationError::InvalidInput(
                    "child terminal delivery missing direct parent agent id".to_string(),
                )
            })?;

        let message = notification
            .final_reply_excerpt
            .as_deref()
            .filter(|excerpt| !excerpt.trim().is_empty())
            .unwrap_or(notification.summary.as_str())
            .to_string();

        let sender_last_turn_outcome = match notification.status {
            AgentLifecycleStatus::Idle => match notification.kind {
                astrcode_core::ChildSessionNotificationKind::Delivered => {
                    Some(AgentTurnOutcome::Completed)
                },
                astrcode_core::ChildSessionNotificationKind::Failed => {
                    Some(AgentTurnOutcome::Failed)
                },
                _ => None,
            },
            _ => None,
        };

        let payload = MailboxQueuedPayload {
            envelope: AgentMailboxEnvelope {
                delivery_id: notification.notification_id.clone(),
                from_agent_id: notification.child_ref.agent_id.clone(),
                to_agent_id: target_agent_id,
                message,
                queued_at: chrono::Utc::now(),
                sender_lifecycle_status: AgentLifecycleStatus::Idle,
                sender_last_turn_outcome,
                sender_open_session_id: notification.child_ref.open_session_id.clone(),
            },
        };

        let mut translator =
            astrcode_core::EventTranslator::new(parent_session_state.current_phase()?);
        astrcode_session_runtime::append_mailbox_queued(
            parent_session_state,
            parent_turn_id,
            AgentEventContext::default(),
            payload,
            &mut translator,
        )
        .await
        .map_err(|error| super::AgentOrchestrationError::Internal(error.to_string()))?;

        Ok(())
    }
}

/// 从 delivery 批次构造 wake prompt。
fn build_wake_prompt_from_deliveries(
    deliveries: &[astrcode_kernel::PendingParentDelivery],
) -> String {
    let parts: Vec<String> = deliveries
        .iter()
        .map(|delivery| {
            format!(
                "[Agent Mailbox Message]\ndelivery_id: {}\nfrom_agent_id: \
                 {}\nsender_lifecycle_status: Idle\nmessage: {}\n\n注意：如果你看到相同 \
                 delivery_id 再次出现，不要把它当作新任务重复处理。",
                delivery.delivery_id,
                delivery.notification.child_ref.agent_id,
                delivery.notification.summary,
            )
        })
        .collect();

    if parts.len() == 1 {
        parts.into_iter().next().unwrap_or_default()
    } else {
        format!(
            "请按顺序处理以下子 Agent 交付结果：\n\n{}",
            parts
                .into_iter()
                .enumerate()
                .map(|(i, p)| format!("{}. {}", i + 1, p))
                .collect::<Vec<_>>()
                .join("\n\n")
        )
    }
}
