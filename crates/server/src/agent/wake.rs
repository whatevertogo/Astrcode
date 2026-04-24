//! 父级 delivery 唤醒调度。
//!
//! wake 是跨会话协作编排，不属于 session-runtime 的单会话真相面。
//! 这里负责把 child terminal delivery 追加到 durable input queue、排入 kernel queue，
//! 再通过“不分叉”的父级 wake turn 继续驱动父 agent。

use astrcode_core::{
    AgentCollaborationActionKind, AgentCollaborationOutcomeKind, AgentEventContext,
    InputBatchAckedPayload, InputBatchStartedPayload, InputQueuedPayload, StorageEventPayload,
    TurnId,
};

use super::{
    AgentOrchestrationError, AgentOrchestrationService, CollaborationFactRecord,
    child_delivery_input_queue_envelope, root_execution_event_context, subrun_event_context,
    terminal_notification_message,
};
use crate::{
    AppAgentPromptSubmission, RecoverableParentDelivery,
    session_identity::normalize_external_session_id,
};

const MAX_AUTOMATIC_INPUT_FOLLOW_UPS: u8 = 8;

impl AgentOrchestrationService {
    /// 父级 delivery 唤醒调度入口。
    ///
    /// 当子 agent turn 终态或显式上行 send 产生 delivery 时调用，
    /// 执行三步：持久化 durable input queue → 排入 kernel delivery buffer → 尝试启动父级 wake
    /// turn。 任何一步失败都会记录指标但不会传播错误（best-effort）。
    pub async fn reactivate_parent_agent_if_idle(
        &self,
        parent_session_id: &str,
        parent_turn_id: &str,
        notification: &astrcode_core::ChildSessionNotification,
    ) {
        self.metrics.record_parent_reactivation_requested();
        let parent_session_id = normalize_external_session_id(parent_session_id);

        if let Err(error) = self
            .append_parent_delivery_input_queue(&parent_session_id, parent_turn_id, notification)
            .await
        {
            log::warn!(
                "failed to persist durable parent input queue before wake: parentSession='{}', \
                 childAgent='{}', deliveryId='{}', error='{}'",
                parent_session_id,
                notification.child_ref.agent_id(),
                notification.notification_id,
                error
            );
            self.metrics.record_parent_reactivation_failed();
            return;
        }

        let queued = self
            .kernel
            .enqueue_child_delivery(
                parent_session_id.clone(),
                parent_turn_id.to_string(),
                notification.clone(),
            )
            .await;
        if queued {
            self.metrics.record_delivery_buffer_queued();
        } else {
            log::warn!(
                "parent delivery was not enqueued immediately; falling back to queue reconcile: \
                 parentSession='{}', deliveryId='{}'",
                parent_session_id,
                notification.notification_id
            );
        }

        if let Err(error) = self
            .try_start_parent_delivery_turn(&parent_session_id, MAX_AUTOMATIC_INPUT_FOLLOW_UPS)
            .await
        {
            self.metrics.record_parent_reactivation_failed();
            log::warn!(
                "failed to schedule parent wake turn from child delivery: parentSession='{}', \
                 childAgent='{}', subRunId='{}', error='{}'",
                parent_session_id,
                notification.child_ref.agent_id(),
                notification.child_ref.sub_run_id(),
                error
            );
        }
    }

    /// 尝试为父级 session 启动一个 delivery 消费 turn。
    ///
    /// 流程：
    /// 1. reconcile：从 durable 存储恢复可能丢失的 delivery
    /// 2. checkout：从 kernel buffer 批量取出待消费 delivery
    /// 3. 提交 queued_inputs prompt（而非普通用户 prompt）
    /// 4. 如果提交失败或 session 忙，requeue 让后续重试
    /// 5. 成功后启动 wake completion watcher 等待终态
    ///
    /// `remaining_follow_ups` 控制自动 follow-up 深度，防止无限递归消费。
    pub async fn try_start_parent_delivery_turn(
        &self,
        parent_session_id: &str,
        remaining_follow_ups: u8,
    ) -> Result<bool, AgentOrchestrationError> {
        let parent_session_id = normalize_external_session_id(parent_session_id);
        self.reconcile_parent_delivery_queue(&parent_session_id)
            .await?;
        let Some(delivery_batch) = self
            .kernel
            .checkout_parent_delivery_batch(&parent_session_id)
            .await
        else {
            return Ok(false);
        };
        self.metrics.record_delivery_buffer_dequeued();
        self.metrics.record_delivery_buffer_wake_requested();

        let batch_delivery_ids = delivery_batch
            .iter()
            .map(|delivery| delivery.delivery_id.clone())
            .collect::<Vec<_>>();
        let target_agent_id = delivery_batch
            .first()
            .and_then(|delivery| delivery.notification.child_ref.parent_agent_id().cloned())
            .ok_or_else(|| {
                AgentOrchestrationError::InvalidInput(
                    "parent delivery batch missing target parent agent id".to_string(),
                )
            })?;
        let wake_agent = self.resolve_wake_agent_context(&delivery_batch).await;
        let wake_turn_id = TurnId::from(format!("turn-{}", chrono::Utc::now().timestamp_millis()));
        let queued_inputs = queued_inputs_from_deliveries(&delivery_batch);
        let accepted = match self
            .session_runtime
            .submit_queued_inputs_for_agent_with_turn_id(
                &parent_session_id,
                wake_turn_id.clone(),
                queued_inputs,
                self.resolve_runtime_config_for_session(&parent_session_id)
                    .await?,
                AppAgentPromptSubmission {
                    agent: wake_agent.clone(),
                    ..Default::default()
                },
            )
            .await
        {
            Ok(Some(accepted)) => accepted,
            Ok(None) => {
                self.kernel
                    .requeue_parent_delivery_batch(&parent_session_id, &batch_delivery_ids)
                    .await;
                return Ok(false);
            },
            Err(error) => {
                self.kernel
                    .requeue_parent_delivery_batch(&parent_session_id, &batch_delivery_ids)
                    .await;
                self.metrics.record_delivery_buffer_wake_failed();
                return Err(AgentOrchestrationError::Internal(format!(
                    "wake turn submit failed: {error}"
                )));
            },
        };
        if let Err(error) = self
            .append_parent_delivery_batch_started(
                &parent_session_id,
                wake_turn_id.as_str(),
                &target_agent_id,
                &batch_delivery_ids,
                &wake_agent,
            )
            .await
        {
            log::warn!(
                "failed to persist parent input queue batch start: parentSession='{}', \
                 turnId='{}', error='{}'",
                parent_session_id,
                wake_turn_id,
                error
            );
        }

        self.spawn_parent_wake_completion_watcher(
            parent_session_id,
            accepted.turn_id.to_string(),
            delivery_batch,
            target_agent_id.to_string(),
            remaining_follow_ups,
        );
        Ok(true)
    }

    fn spawn_parent_wake_completion_watcher(
        &self,
        parent_session_id: String,
        turn_id: String,
        batch_deliveries: Vec<RecoverableParentDelivery>,
        target_agent_id: String,
        remaining_follow_ups: u8,
    ) {
        let service = self.clone();
        let handle = tokio::spawn(async move {
            if let Err(error) = service
                .finalize_parent_wake_turn(
                    parent_session_id,
                    turn_id,
                    batch_deliveries,
                    target_agent_id,
                    remaining_follow_ups,
                )
                .await
            {
                log::warn!("failed to finalize parent wake turn: {error}");
            }
        });
        // 为什么登记：这个 watcher 只服务父级 wake turn 的 ack/requeue 收口，
        // 生命周期应与 turn 任务一起在 shutdown 时统一 abort。
        self.task_registry.register_turn_task(handle);
    }

    /// 父级 wake turn 完成后的收口处理。
    ///
    /// 判断 wake turn 是否成功（Idle + TurnDone + 无 Error）：
    /// - 成功：消费 delivery batch、记录 Consumed fact、 如果还有剩余 delivery 则自动触发下一轮
    ///   follow-up
    /// - 失败：requeue delivery batch 让后续重试
    ///
    /// 关键设计：wake turn 不会向更上一级制造新的 terminal delivery，
    /// 避免"协作协调 turn"被误当成新的 child work turn 而形成自激膨胀。
    async fn finalize_parent_wake_turn(
        &self,
        parent_session_id: String,
        turn_id: String,
        batch_deliveries: Vec<RecoverableParentDelivery>,
        target_agent_id: String,
        remaining_follow_ups: u8,
    ) -> Result<(), AgentOrchestrationError> {
        let batch_delivery_ids = batch_deliveries
            .iter()
            .map(|delivery| delivery.delivery_id.clone())
            .collect::<Vec<_>>();
        let terminal = self
            .session_runtime
            .wait_for_turn_terminal_snapshot(&parent_session_id, &turn_id)
            .await?;
        let wake_succeeded =
            matches!(terminal.phase, astrcode_core::Phase::Idle)
                && terminal.events.iter().any(|stored| {
                    matches!(stored.event.payload, StorageEventPayload::TurnDone { .. })
                })
                && !terminal.events.iter().any(|stored| {
                    matches!(stored.event.payload, StorageEventPayload::Error { .. })
                });
        // 为什么 wake turn 不再自动向更上一级制造 terminal delivery：
        // Claude Code 的稳定点是“worker 每轮进入 idle，但 idle 通知只是状态转换，不代表
        // 又生成了一项新的上游任务”。这里保持同样边界：wake 只负责消费当前 input queue batch，
        // 避免把协作协调 turn 误当成新的 child work turn，从而形成自激膨胀。

        if wake_succeeded {
            self.append_parent_delivery_batch_acked(
                &parent_session_id,
                &turn_id,
                &target_agent_id,
                &batch_delivery_ids,
            )
            .await?;
            let consumed = self
                .kernel
                .consume_parent_delivery_batch(&parent_session_id, &batch_delivery_ids)
                .await;
            if consumed {
                let runtime = self
                    .resolve_runtime_config_for_session(&parent_session_id)
                    .await
                    .unwrap_or_default();
                for delivery in &batch_deliveries {
                    if let Some(child_handle) = self
                        .kernel
                        .get_handle(delivery.notification.child_ref.agent_id())
                        .await
                    {
                        self.record_fact_best_effort(
                            &runtime,
                            CollaborationFactRecord::new(
                                AgentCollaborationActionKind::Delivery,
                                AgentCollaborationOutcomeKind::Consumed,
                                &parent_session_id,
                                &turn_id,
                            )
                            .parent_agent_id(
                                delivery
                                    .notification
                                    .child_ref
                                    .parent_agent_id()
                                    .cloned()
                                    .map(|id| id.to_string()),
                            )
                            .child(&child_handle)
                            .delivery_id(delivery.delivery_id.clone())
                            .summary(terminal_notification_message(&delivery.notification))
                            .latency_ms(
                                (chrono::Utc::now().timestamp_millis() - delivery.queued_at_ms)
                                    .max(0) as u64,
                            )
                            .source_tool_call_id(
                                delivery
                                    .notification
                                    .source_tool_call_id
                                    .clone()
                                    .map(|id| id.to_string()),
                            ),
                        )
                        .await;
                    }
                }
                self.metrics.record_parent_reactivation_succeeded();
                self.metrics.record_delivery_buffer_wake_succeeded();
                if remaining_follow_ups > 1 {
                    let _ = self
                        .try_start_parent_delivery_turn(
                            &parent_session_id,
                            remaining_follow_ups.saturating_sub(1),
                        )
                        .await?;
                } else {
                    let remaining_queued_inputs = self
                        .kernel
                        .pending_parent_delivery_count(&parent_session_id)
                        .await;
                    if remaining_queued_inputs == 0 {
                        return Ok(());
                    }
                    log::warn!(
                        "automatic parent input follow-up limit reached: parentSession='{}', \
                         remainingQueuedInputs='{}', maxAutomaticFollowUps='{}'",
                        parent_session_id,
                        remaining_queued_inputs,
                        MAX_AUTOMATIC_INPUT_FOLLOW_UPS
                    );
                }
                return Ok(());
            }

            log::warn!(
                "parent wake turn succeeded but delivery batch consume failed: \
                 parentSession='{}', turnId='{}'",
                parent_session_id,
                turn_id
            );
        }

        self.kernel
            .requeue_parent_delivery_batch(&parent_session_id, &batch_delivery_ids)
            .await;
        self.metrics.record_parent_reactivation_failed();
        self.metrics.record_delivery_buffer_wake_failed();
        Ok(())
    }

    async fn append_parent_delivery_input_queue(
        &self,
        parent_session_id: &str,
        parent_turn_id: &str,
        notification: &astrcode_core::ChildSessionNotification,
    ) -> Result<(), AgentOrchestrationError> {
        let target_agent_id = notification
            .child_ref
            .parent_agent_id()
            .cloned()
            .ok_or_else(|| {
                AgentOrchestrationError::InvalidInput(
                    "child terminal delivery missing direct parent agent id".to_string(),
                )
            })?;

        self.session_runtime
            .append_agent_input_queued(
                parent_session_id,
                parent_turn_id,
                AgentEventContext::default(),
                InputQueuedPayload {
                    envelope: child_delivery_input_queue_envelope(
                        notification,
                        target_agent_id.to_string(),
                    ),
                },
            )
            .await
            .map_err(AgentOrchestrationError::from)?;
        Ok(())
    }

    async fn append_parent_delivery_batch_started(
        &self,
        parent_session_id: &str,
        turn_id: &str,
        target_agent_id: &str,
        batch_delivery_ids: &[String],
        event_agent: &AgentEventContext,
    ) -> Result<(), AgentOrchestrationError> {
        self.session_runtime
            .append_agent_input_batch_started(
                parent_session_id,
                turn_id,
                event_agent.clone(),
                InputBatchStartedPayload {
                    target_agent_id: target_agent_id.to_string(),
                    turn_id: turn_id.to_string(),
                    batch_id: parent_wake_batch_id(turn_id),
                    delivery_ids: batch_delivery_ids.iter().cloned().map(Into::into).collect(),
                },
            )
            .await
            .map_err(AgentOrchestrationError::from)?;
        Ok(())
    }

    async fn append_parent_delivery_batch_acked(
        &self,
        parent_session_id: &str,
        turn_id: &str,
        target_agent_id: &str,
        batch_delivery_ids: &[String],
    ) -> Result<(), AgentOrchestrationError> {
        self.session_runtime
            .append_agent_input_batch_acked(
                parent_session_id,
                turn_id,
                AgentEventContext::default(),
                InputBatchAckedPayload {
                    target_agent_id: target_agent_id.to_string(),
                    turn_id: turn_id.to_string(),
                    batch_id: parent_wake_batch_id(turn_id),
                    delivery_ids: batch_delivery_ids.iter().cloned().map(Into::into).collect(),
                },
            )
            .await
            .map_err(AgentOrchestrationError::from)?;
        Ok(())
    }

    /// 从 durable 存储恢复可能丢失的 parent delivery。
    ///
    /// 场景：进程崩溃后重启，kernel buffer 中的内存状态已丢失，
    /// 但 durable input queue 中仍有未消费的 queued 事件。
    /// 此函数通过 `recoverable_parent_deliveries` 从存储事件中
    /// 重建 delivery 并重新排入 kernel buffer。
    async fn reconcile_parent_delivery_queue(
        &self,
        parent_session_id: &str,
    ) -> Result<(), AgentOrchestrationError> {
        let recoverable = self
            .session_runtime
            .recoverable_parent_deliveries(parent_session_id)
            .await
            .map_err(AgentOrchestrationError::from)?;
        if recoverable.is_empty() {
            return Ok(());
        }

        for pending in recoverable {
            let runtime = self
                .resolve_runtime_config_for_session(parent_session_id)
                .await
                .unwrap_or_default();
            if let Some(child_handle) = self
                .kernel
                .get_handle(pending.notification.child_ref.agent_id())
                .await
            {
                self.record_fact_best_effort(
                    &runtime,
                    CollaborationFactRecord::new(
                        AgentCollaborationActionKind::Delivery,
                        AgentCollaborationOutcomeKind::Replayed,
                        parent_session_id,
                        &pending.parent_turn_id,
                    )
                    .parent_agent_id(
                        pending
                            .notification
                            .child_ref
                            .parent_agent_id()
                            .cloned()
                            .map(|id| id.to_string()),
                    )
                    .child(&child_handle)
                    .delivery_id(pending.delivery_id.clone())
                    .reason_code("durable_recovery")
                    .summary(terminal_notification_message(&pending.notification))
                    .source_tool_call_id(
                        pending
                            .notification
                            .source_tool_call_id
                            .clone()
                            .map(|id| id.to_string()),
                    ),
                )
                .await;
            }
            let _ = self
                .kernel
                .enqueue_child_delivery(
                    pending.parent_session_id.clone(),
                    pending.parent_turn_id.clone(),
                    pending.notification,
                )
                .await;
        }
        Ok(())
    }

    async fn resolve_wake_agent_context(
        &self,
        deliveries: &[RecoverableParentDelivery],
    ) -> AgentEventContext {
        let Some(target_agent_id) = deliveries
            .first()
            .and_then(|delivery| delivery.notification.child_ref.parent_agent_id().cloned())
        else {
            return AgentEventContext::default();
        };
        let Some(parent_handle) = self.kernel.get_handle(&target_agent_id).await else {
            return AgentEventContext::default();
        };
        if parent_handle.depth == 0 {
            root_execution_event_context(
                parent_handle.agent_id.clone(),
                parent_handle.agent_profile.clone(),
            )
        } else {
            subrun_event_context(&parent_handle)
        }
    }
}

fn parent_wake_batch_id(turn_id: &str) -> String {
    format!("parent-wake-batch:{turn_id}")
}

fn queued_inputs_from_deliveries(deliveries: &[RecoverableParentDelivery]) -> Vec<String> {
    deliveries
        .iter()
        .map(|delivery| {
            format!(
                "子 Agent {} 刚交付了一条结果：\n{}",
                delivery.notification.child_ref.agent_id(),
                terminal_notification_message(&delivery.notification),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use astrcode_core::{
        AgentEventContext, AgentLifecycleStatus, ChildAgentRef, ChildExecutionIdentity,
        ChildSessionLineageKind, ChildSessionNotification, ChildSessionNotificationKind,
        ParentExecutionRef, Phase, QueuedInputEnvelope, SessionId, StorageEvent, StoredEvent,
    };

    use super::*;
    use crate::{
        agent::{
            terminal_notification_turn_outcome,
            test_support::{TestLlmBehavior, build_agent_test_harness, sample_profile},
        },
        lifecycle::governance::ObservabilitySnapshotProvider,
    };

    fn sample_notification(
        parent_session_id: &str,
        parent_agent_id: &str,
        kind: ChildSessionNotificationKind,
    ) -> ChildSessionNotification {
        ChildSessionNotification {
            notification_id: format!("delivery-{kind:?}").to_lowercase().into(),
            child_ref: ChildAgentRef {
                identity: ChildExecutionIdentity {
                    agent_id: "agent-child".to_string().into(),
                    session_id: parent_session_id.to_string().into(),
                    sub_run_id: "subrun-child".to_string().into(),
                },
                parent: ParentExecutionRef {
                    parent_agent_id: Some(parent_agent_id.to_string().into()),
                    parent_sub_run_id: Some("subrun-parent".to_string().into()),
                },
                lineage_kind: ChildSessionLineageKind::Spawn,
                status: AgentLifecycleStatus::Idle,
                open_session_id: "session-child".to_string().into(),
            },
            kind,
            source_tool_call_id: Some("tool-call-1".to_string().into()),
            delivery: Some(astrcode_core::ParentDelivery {
                idempotency_key: format!("delivery-{kind:?}").to_lowercase(),
                origin: astrcode_core::ParentDeliveryOrigin::Explicit,
                terminal_semantics: match kind {
                    ChildSessionNotificationKind::Started
                    | ChildSessionNotificationKind::ProgressSummary
                    | ChildSessionNotificationKind::Waiting
                    | ChildSessionNotificationKind::Resumed => {
                        astrcode_core::ParentDeliveryTerminalSemantics::NonTerminal
                    },
                    ChildSessionNotificationKind::Delivered
                    | ChildSessionNotificationKind::Closed
                    | ChildSessionNotificationKind::Failed => {
                        astrcode_core::ParentDeliveryTerminalSemantics::Terminal
                    },
                },
                source_turn_id: Some("turn-child".to_string()),
                payload: match kind {
                    ChildSessionNotificationKind::Delivered => {
                        astrcode_core::ParentDeliveryPayload::Completed(
                            astrcode_core::CompletedParentDeliveryPayload {
                                message: "最终回复摘录".to_string(),
                                findings: Vec::new(),
                                artifacts: Vec::new(),
                            },
                        )
                    },
                    ChildSessionNotificationKind::Failed => {
                        astrcode_core::ParentDeliveryPayload::Failed(
                            astrcode_core::FailedParentDeliveryPayload {
                                message: "子 Agent 已完成".to_string(),
                                code: astrcode_core::SubRunFailureCode::Internal,
                                technical_message: None,
                                retryable: false,
                            },
                        )
                    },
                    ChildSessionNotificationKind::Closed => {
                        astrcode_core::ParentDeliveryPayload::CloseRequest(
                            astrcode_core::CloseRequestParentDeliveryPayload {
                                message: "子 Agent 已完成".to_string(),
                                reason: Some("child_closed".to_string()),
                            },
                        )
                    },
                    ChildSessionNotificationKind::Started
                    | ChildSessionNotificationKind::ProgressSummary
                    | ChildSessionNotificationKind::Waiting
                    | ChildSessionNotificationKind::Resumed => {
                        astrcode_core::ParentDeliveryPayload::Progress(
                            astrcode_core::ProgressParentDeliveryPayload {
                                message: "子 Agent 已完成".to_string(),
                            },
                        )
                    },
                },
            }),
        }
    }

    fn child_notification_stored_event(
        storage_seq: u64,
        parent_turn_id: &str,
        notification: ChildSessionNotification,
    ) -> StoredEvent {
        StoredEvent {
            storage_seq,
            event: StorageEvent {
                turn_id: Some(parent_turn_id.to_string()),
                agent: AgentEventContext::default(),
                payload: StorageEventPayload::ChildSessionNotification {
                    notification,
                    timestamp: Some(chrono::Utc::now()),
                },
            },
        }
    }

    #[tokio::test]
    async fn busy_parent_requeues_delivery_until_explicit_retry() {
        let harness = build_agent_test_harness(TestLlmBehavior::Succeed {
            content: "父级稍后继续处理。".to_string(),
        })
        .expect("test harness should build");
        let project = tempfile::tempdir().expect("tempdir should be created");
        let parent = harness
            .session_runtime
            .create_session(project.path().display().to_string())
            .await
            .expect("parent session should be created");
        let root = harness
            .session_runtime
            .agent_control()
            .register_root_agent(
                "root-agent".to_string(),
                parent.session_id.clone(),
                "root-profile".to_string(),
            )
            .await
            .expect("root agent should register");
        let generation = harness
            .prepare_busy_turn(&parent.session_id, "turn-busy")
            .await
            .expect("busy state should prepare");

        let notification = sample_notification(
            &parent.session_id,
            &root.agent_id,
            ChildSessionNotificationKind::Delivered,
        );
        harness
            .service
            .reactivate_parent_agent_if_idle(&parent.session_id, "turn-parent", &notification)
            .await;

        assert_eq!(
            harness
                .session_runtime
                .agent_control()
                .pending_parent_delivery_count(&parent.session_id)
                .await,
            1,
            "busy parent should keep delivery queued for retry"
        );
        assert_eq!(
            harness
                .session_runtime
                .list_session_ids()
                .await
                .expect("session ids should list")
                .len(),
            1,
            "busy wake should not branch a new session"
        );

        harness
            .complete_turn_state(&parent.session_id, generation, Phase::Idle)
            .await
            .expect("completion should succeed");
        let started = harness
            .service
            .try_start_parent_delivery_turn(&parent.session_id, MAX_AUTOMATIC_INPUT_FOLLOW_UPS)
            .await
            .expect("retry should succeed");
        assert!(started, "idle parent should start wake turn on retry");

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if harness
                .session_runtime
                .agent_control()
                .pending_parent_delivery_count(&parent.session_id)
                .await
                == 0
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "retried wake turn should consume queued delivery"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        let metrics = harness.metrics.snapshot();
        assert_eq!(
            metrics.execution_diagnostics.parent_reactivation_requested,
            1
        );
        assert_eq!(
            metrics.execution_diagnostics.parent_reactivation_succeeded,
            1
        );
        assert_eq!(
            metrics.execution_diagnostics.delivery_buffer_wake_succeeded,
            1
        );
    }

    #[tokio::test]
    async fn wake_turn_drains_delivery_without_bubbling_terminal_notification_upward() {
        let harness = build_agent_test_harness(TestLlmBehavior::Succeed {
            content: "middle 已处理 leaf 交付。".to_string(),
        })
        .expect("test harness should build");
        let project = tempfile::tempdir().expect("tempdir should be created");
        let root_session = harness
            .session_runtime
            .create_session(project.path().display().to_string())
            .await
            .expect("root session should be created");
        let middle_session = harness
            .session_runtime
            .create_session(project.path().display().to_string())
            .await
            .expect("middle session should be created");
        let root = harness
            .session_runtime
            .agent_control()
            .register_root_agent(
                "root-agent".to_string(),
                root_session.session_id.clone(),
                "root-profile".to_string(),
            )
            .await
            .expect("root agent should register");
        let middle = harness
            .session_runtime
            .agent_control()
            .spawn_with_storage(
                &sample_profile("reviewer"),
                root_session.session_id.clone(),
                Some(middle_session.session_id.clone()),
                "turn-root".to_string(),
                Some(root.agent_id.to_string()),
                astrcode_core::SubRunStorageMode::IndependentSession,
            )
            .await
            .expect("middle handle should spawn");
        harness
            .session_runtime
            .agent_control()
            .set_lifecycle(&middle.agent_id, AgentLifecycleStatus::Running)
            .await
            .expect("middle lifecycle should update");

        let leaf_delivery = ChildSessionNotification {
            notification_id: "leaf-terminal:turn-leaf:completed".to_string().into(),
            child_ref: ChildAgentRef {
                identity: ChildExecutionIdentity {
                    agent_id: "agent-leaf".to_string().into(),
                    session_id: middle_session.session_id.clone().into(),
                    sub_run_id: "subrun-leaf".to_string().into(),
                },
                parent: ParentExecutionRef {
                    parent_agent_id: Some(middle.agent_id.clone()),
                    parent_sub_run_id: Some(middle.sub_run_id.clone()),
                },
                lineage_kind: ChildSessionLineageKind::Spawn,
                status: AgentLifecycleStatus::Idle,
                open_session_id: "session-leaf".to_string().into(),
            },
            kind: ChildSessionNotificationKind::Delivered,
            source_tool_call_id: None,
            delivery: Some(astrcode_core::ParentDelivery {
                idempotency_key: "leaf-terminal:turn-leaf:completed".to_string(),
                origin: astrcode_core::ParentDeliveryOrigin::Explicit,
                terminal_semantics: astrcode_core::ParentDeliveryTerminalSemantics::Terminal,
                source_turn_id: Some("turn-leaf".to_string()),
                payload: astrcode_core::ParentDeliveryPayload::Completed(
                    astrcode_core::CompletedParentDeliveryPayload {
                        message: "leaf 最终回复".to_string(),
                        findings: Vec::new(),
                        artifacts: Vec::new(),
                    },
                ),
            }),
        };

        harness
            .service
            .reactivate_parent_agent_if_idle(
                &middle_session.session_id,
                "turn-middle-parent",
                &leaf_delivery,
            )
            .await;

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let middle_pending = harness
                .session_runtime
                .agent_control()
                .pending_parent_delivery_count(&middle_session.session_id)
                .await;
            let root_pending = harness
                .session_runtime
                .agent_control()
                .pending_parent_delivery_count(&root_session.session_id)
                .await;
            if middle_pending == 0 && root_pending == 0 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "wake turn should drain the middle delivery queue without inflating root queue"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        let root_events = harness
            .session_runtime
            .replay_stored_events(&SessionId::from(root_session.session_id.clone()))
            .await
            .expect("root events should replay");
        assert!(
            !root_events.iter().any(|stored| matches!(
                &stored.event.payload,
                StorageEventPayload::ChildSessionNotification { notification, .. }
                    if notification.child_ref.agent_id() == &middle.agent_id
            )),
            "wake turn is a coordination turn and must not auto-manufacture a new upward delivery"
        );
    }

    #[tokio::test]
    async fn wake_failure_requeues_delivery_batch() {
        let harness = build_agent_test_harness(TestLlmBehavior::Fail {
            message: "wake llm offline".to_string(),
        })
        .expect("test harness should build");
        let project = tempfile::tempdir().expect("tempdir should be created");
        let parent = harness
            .session_runtime
            .create_session(project.path().display().to_string())
            .await
            .expect("parent session should be created");
        let root = harness
            .session_runtime
            .agent_control()
            .register_root_agent(
                "root-agent".to_string(),
                parent.session_id.clone(),
                "root-profile".to_string(),
            )
            .await
            .expect("root agent should register");
        let notification = sample_notification(
            &parent.session_id,
            &root.agent_id,
            ChildSessionNotificationKind::Failed,
        );

        harness
            .service
            .reactivate_parent_agent_if_idle(&parent.session_id, "turn-parent", &notification)
            .await;

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let metrics = harness.metrics.snapshot();
            if harness
                .session_runtime
                .agent_control()
                .pending_parent_delivery_count(&parent.session_id)
                .await
                == 1
                && metrics.execution_diagnostics.parent_reactivation_failed == 1
                && metrics.execution_diagnostics.delivery_buffer_wake_failed == 1
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "failed wake should requeue delivery"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        let metrics = harness.metrics.snapshot();
        assert_eq!(
            metrics.execution_diagnostics.parent_reactivation_requested,
            1
        );
        assert_eq!(metrics.execution_diagnostics.parent_reactivation_failed, 1);
        assert_eq!(metrics.execution_diagnostics.delivery_buffer_wake_failed, 1);
    }

    #[tokio::test]
    async fn try_start_parent_delivery_turn_recovers_durable_pending_delivery() {
        let harness = build_agent_test_harness(TestLlmBehavior::Succeed {
            content: "父级已恢复 durable 交付。".to_string(),
        })
        .expect("test harness should build");
        let project = tempfile::tempdir().expect("tempdir should be created");
        let parent = harness
            .session_runtime
            .create_session(project.path().display().to_string())
            .await
            .expect("parent session should be created");
        let root = harness
            .session_runtime
            .agent_control()
            .register_root_agent(
                "root-agent".to_string(),
                parent.session_id.clone(),
                "root-profile".to_string(),
            )
            .await
            .expect("root agent should register");
        let notification = sample_notification(
            &parent.session_id,
            &root.agent_id,
            ChildSessionNotificationKind::Delivered,
        );
        harness
            .service
            .append_parent_delivery_input_queue(&parent.session_id, "turn-parent", &notification)
            .await
            .expect("durable input queue should append");
        harness
            .append_events_to_session(
                &parent.session_id,
                Phase::Idle,
                &[StorageEvent {
                    turn_id: Some("turn-parent".to_string()),
                    agent: AgentEventContext::default(),
                    payload: StorageEventPayload::ChildSessionNotification {
                        notification: notification.clone(),
                        timestamp: Some(chrono::Utc::now()),
                    },
                }],
            )
            .await
            .expect("child notification should persist");

        let started = harness
            .service
            .try_start_parent_delivery_turn(&parent.session_id, MAX_AUTOMATIC_INPUT_FOLLOW_UPS)
            .await
            .expect("wake should recover pending durable delivery");
        assert!(started, "recovered durable delivery should start wake turn");

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if harness
                .session_runtime
                .agent_control()
                .pending_parent_delivery_count(&parent.session_id)
                .await
                == 0
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "recovered wake turn should eventually drain delivery queue"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        let parent_events = harness
            .session_runtime
            .replay_stored_events(&SessionId::from(parent.session_id.clone()))
            .await
            .expect("parent events should replay");
        assert!(parent_events.iter().any(|stored| matches!(
            &stored.event.payload,
            StorageEventPayload::AgentInputBatchStarted { payload }
                if payload.target_agent_id == root.agent_id.to_string()
                    && payload.delivery_ids == vec![notification.notification_id.clone()]
        )));
        assert!(parent_events.iter().any(|stored| matches!(
            &stored.event.payload,
            StorageEventPayload::AgentInputBatchAcked { payload }
                if payload.target_agent_id == root.agent_id.to_string()
                    && payload.delivery_ids == vec![notification.notification_id.clone()]
        )));
    }

    #[test]
    fn queued_inputs_use_delivery_message_without_removed_fields() {
        let delivered = sample_notification(
            "session-parent",
            "agent-parent",
            ChildSessionNotificationKind::Delivered,
        );
        let failed = sample_notification(
            "session-parent",
            "agent-parent",
            ChildSessionNotificationKind::Failed,
        );

        assert_eq!(terminal_notification_message(&delivered), "最终回复摘录");
        assert_eq!(terminal_notification_message(&failed), "子 Agent 已完成");

        let queued_inputs = queued_inputs_from_deliveries(&[
            RecoverableParentDelivery {
                delivery_id: "delivery-1".to_string(),
                parent_session_id: "session-parent".to_string(),
                parent_turn_id: "turn-parent".to_string(),
                queued_at_ms: chrono::Utc::now().timestamp_millis(),
                notification: delivered,
            },
            RecoverableParentDelivery {
                delivery_id: "delivery-2".to_string(),
                parent_session_id: "session-parent".to_string(),
                parent_turn_id: "turn-parent".to_string(),
                queued_at_ms: chrono::Utc::now().timestamp_millis(),
                notification: failed,
            },
        ]);
        assert_eq!(queued_inputs.len(), 2);
        assert!(queued_inputs[0].contains("子 Agent"));
        assert!(queued_inputs[0].contains("最终回复摘录"));
        assert!(queued_inputs[1].contains("子 Agent 已完成"));
    }

    #[test]
    fn recoverable_parent_deliveries_skips_active_batch_entries() {
        let delivered = sample_notification(
            "session-parent",
            "agent-parent",
            ChildSessionNotificationKind::Delivered,
        );
        let failed = ChildSessionNotification {
            notification_id: "delivery-failed".to_string().into(),
            ..sample_notification(
                "session-parent",
                "agent-parent",
                ChildSessionNotificationKind::Failed,
            )
        };
        let events = vec![
            child_notification_stored_event(1, "turn-parent", delivered.clone()),
            StoredEvent {
                storage_seq: 2,
                event: StorageEvent {
                    turn_id: Some("turn-wake-1".to_string()),
                    agent: AgentEventContext::default(),
                    payload: StorageEventPayload::AgentInputQueued {
                        payload: InputQueuedPayload {
                            envelope: QueuedInputEnvelope {
                                delivery_id: delivered.notification_id.clone(),
                                from_agent_id: delivered.child_ref.agent_id().to_string(),
                                to_agent_id: "agent-parent".to_string(),
                                message: terminal_notification_message(&delivered),
                                queued_at: chrono::Utc::now(),
                                sender_lifecycle_status: AgentLifecycleStatus::Idle,
                                sender_last_turn_outcome: terminal_notification_turn_outcome(
                                    &delivered,
                                ),
                                sender_open_session_id: delivered
                                    .child_ref
                                    .open_session_id
                                    .to_string(),
                            },
                        },
                    },
                },
            },
            StoredEvent {
                storage_seq: 3,
                event: StorageEvent {
                    turn_id: Some("turn-wake-1".to_string()),
                    agent: AgentEventContext::default(),
                    payload: StorageEventPayload::AgentInputBatchStarted {
                        payload: InputBatchStartedPayload {
                            target_agent_id: "agent-parent".to_string(),
                            turn_id: "turn-wake-1".to_string(),
                            batch_id: parent_wake_batch_id("turn-wake-1"),
                            delivery_ids: vec![delivered.notification_id.clone()],
                        },
                    },
                },
            },
            child_notification_stored_event(4, "turn-parent", failed.clone()),
            StoredEvent {
                storage_seq: 5,
                event: StorageEvent {
                    turn_id: Some("turn-parent".to_string()),
                    agent: AgentEventContext::default(),
                    payload: StorageEventPayload::AgentInputQueued {
                        payload: InputQueuedPayload {
                            envelope: QueuedInputEnvelope {
                                delivery_id: failed.notification_id.clone(),
                                from_agent_id: failed.child_ref.agent_id().to_string(),
                                to_agent_id: "agent-parent".to_string(),
                                message: terminal_notification_message(&failed),
                                queued_at: chrono::Utc::now(),
                                sender_lifecycle_status: AgentLifecycleStatus::Idle,
                                sender_last_turn_outcome: terminal_notification_turn_outcome(
                                    &failed,
                                ),
                                sender_open_session_id: failed
                                    .child_ref
                                    .open_session_id
                                    .to_string(),
                            },
                        },
                    },
                },
            },
        ];

        let recovered = crate::ports::recoverable_parent_deliveries(&events);

        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].delivery_id, failed.notification_id.to_string());
    }
}
