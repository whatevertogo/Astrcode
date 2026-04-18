use std::time::Instant;

use astrcode_core::{
    AgentCollaborationActionKind, AgentCollaborationOutcomeKind, AgentLifecycleStatus,
    AgentTurnOutcome, ChildSessionNotification, ChildSessionNotificationKind,
    CloseRequestParentDeliveryPayload, CompletedParentDeliveryPayload, CompletedSubRunOutcome,
    FailedParentDeliveryPayload, FailedSubRunOutcome, ParentDelivery, ParentDeliveryOrigin,
    ParentDeliveryPayload, ParentDeliveryTerminalSemantics, ProgressParentDeliveryPayload,
    StorageEventPayload, SubRunFailure, SubRunFailureCode, SubRunHandoff, SubRunResult,
    SubRunStatus,
};

use super::{
    AgentOrchestrationError, AgentOrchestrationService, child_collaboration_artifacts,
    subrun_event_context_for_parent_turn, terminal_notification_message,
};

/// child turn 终态投递到父侧的内部投影层。
///
/// 从 `SubRunResult` 提取出父侧 `ChildSessionNotification` 所需的三个维度：
/// notification kind（Delivered/Failed/Closed）、lifecycle status、typed delivery payload。
struct ChildTerminalDeliveryProjection {
    kind: ChildSessionNotificationKind,
    status: AgentLifecycleStatus,
    delivery: ParentDelivery,
}

/// 聚合 child turn 终态收口所需上下文，避免不同入口重复传参与路由真相漂移。
///
/// 注意：这里显式携带 parent routing truth。
/// `ChildAgentRef` 只用于 stable child reference / projection，
/// 禁止再从 `child_ref.session_id` 反推父侧 notification 的落点。
pub(super) struct ChildTurnTerminalContext {
    child: astrcode_core::SubRunHandle,
    execution_session_id: String,
    execution_turn_id: String,
    parent_session_id: String,
    parent_turn_id: String,
    source_tool_call_id: Option<String>,
    started_at: Instant,
}

impl ChildTurnTerminalContext {
    pub(super) fn new(
        child: astrcode_core::SubRunHandle,
        execution_session_id: String,
        execution_turn_id: String,
        parent_session_id: String,
        parent_turn_id: String,
        source_tool_call_id: Option<String>,
    ) -> Self {
        Self {
            child,
            execution_session_id,
            execution_turn_id,
            parent_session_id,
            parent_turn_id,
            source_tool_call_id,
            started_at: Instant::now(),
        }
    }
}

impl AgentOrchestrationService {
    pub(super) fn spawn_child_turn_terminal_watcher(
        &self,
        child: astrcode_core::SubRunHandle,
        execution_session_id: String,
        execution_turn_id: String,
        parent_session_id: String,
        parent_turn_id: String,
        source_tool_call_id: Option<String>,
    ) {
        let service = self.clone();
        let handle = tokio::spawn(async move {
            let watch = ChildTurnTerminalContext::new(
                child,
                execution_session_id,
                execution_turn_id,
                parent_session_id,
                parent_turn_id,
                source_tool_call_id,
            );
            if let Err(error) = service.finalize_child_turn_when_done(watch).await {
                log::warn!("failed to finalize child turn terminal delivery: {error}");
            }
        });
        // 为什么登记：child turn terminal watcher 负责任意 child turn 的统一终态收口，
        // 必须跟随治理关闭与
        // 测试 teardown 一起回收，避免后台 watcher 残留。
        self.task_registry.register_subagent_task(handle);
    }

    pub(super) async fn finalize_child_turn_when_done(
        &self,
        watch: ChildTurnTerminalContext,
    ) -> Result<(), AgentOrchestrationError> {
        let outcome = self
            .session_runtime
            .project_turn_outcome(&watch.execution_session_id, &watch.execution_turn_id)
            .await?;
        self.finalize_child_turn_with_outcome(watch, outcome).await
    }

    pub(super) async fn finalize_child_turn_with_outcome(
        &self,
        watch: ChildTurnTerminalContext,
        outcome: astrcode_session_runtime::ProjectedTurnOutcome,
    ) -> Result<(), AgentOrchestrationError> {
        let result = build_child_subrun_result(
            &watch.child,
            &watch.parent_session_id,
            &watch.execution_turn_id,
            &outcome,
        );
        let _ = self
            .kernel
            // 为什么这里需要单独的完成态推进端口：
            // child turn terminal finalizer 必须原子更新 live tree、并发槽位和最后一轮 outcome，
            // 但 application 仍只依赖 trait 契约，不再直接持有 AgentControl concrete。
            .complete_turn(&watch.child.agent_id, outcome.outcome)
            .await;
        self.metrics.record_subrun_execution(
            watch.started_at.elapsed().as_millis() as u64,
            outcome.outcome,
            None,
            None,
            Some(watch.child.storage_mode),
        );
        if self.has_explicit_terminal_delivery_for_turn(&watch).await? {
            log::info!(
                "skip fallback terminal delivery because explicit terminal delivery already \
                 exists: childAgent='{}', parentSession='{}', parentTurn='{}', sourceTurnId='{}'",
                watch.child.agent_id,
                watch.parent_session_id,
                watch.parent_turn_id,
                watch.execution_turn_id
            );
            return Ok(());
        }

        let fallback_notification_id = child_terminal_notification_id(
            watch.child.sub_run_id.as_str(),
            &watch.execution_turn_id,
            result.status(),
        );
        let delivery = project_child_terminal_delivery(&result, &fallback_notification_id);
        let notification_id = delivery.delivery.idempotency_key.clone();
        let notification = ChildSessionNotification {
            notification_id: notification_id.clone().into(),
            child_ref: watch.child.child_ref_with_status(delivery.status),
            kind: delivery.kind,
            source_tool_call_id: watch.source_tool_call_id.map(Into::into),
            delivery: Some(delivery.delivery),
        };

        self.append_child_session_notification(
            &watch.child,
            &watch.parent_session_id,
            &watch.parent_turn_id,
            &notification,
        )
        .await?;
        let runtime = self
            .resolve_runtime_config_for_session(&watch.parent_session_id)
            .await
            .unwrap_or_default();
        self.record_fact_best_effort(
            &runtime,
            super::CollaborationFactRecord::new(
                AgentCollaborationActionKind::Delivery,
                AgentCollaborationOutcomeKind::Delivered,
                &watch.parent_session_id,
                &watch.parent_turn_id,
            )
            .parent_agent_id(watch.child.parent_agent_id.clone().map(|id| id.to_string()))
            .child(&watch.child)
            .delivery_id(notification.notification_id.clone())
            .summary(terminal_notification_message(&notification))
            .source_tool_call_id(
                notification
                    .source_tool_call_id
                    .clone()
                    .map(|id| id.to_string()),
            ),
        )
        .await;
        self.reactivate_parent_agent_if_idle(
            &watch.parent_session_id,
            &watch.parent_turn_id,
            &notification,
        )
        .await;
        Ok(())
    }

    pub(super) async fn append_child_session_notification(
        &self,
        child: &astrcode_core::SubRunHandle,
        parent_session_id: &str,
        parent_turn_id: &str,
        notification: &ChildSessionNotification,
    ) -> Result<(), AgentOrchestrationError> {
        self.session_runtime
            .append_child_session_notification(
                parent_session_id,
                parent_turn_id,
                subrun_event_context_for_parent_turn(child, parent_turn_id),
                notification.clone(),
            )
            .await
            .map_err(AgentOrchestrationError::from)?;
        Ok(())
    }

    async fn has_explicit_terminal_delivery_for_turn(
        &self,
        watch: &ChildTurnTerminalContext,
    ) -> Result<bool, AgentOrchestrationError> {
        let stored = self
            .session_runtime
            .session_stored_events(&astrcode_core::SessionId::from(
                watch.parent_session_id.clone(),
            ))
            .await
            .map_err(AgentOrchestrationError::from)?;

        Ok(stored.iter().any(|stored| match &stored.event.payload {
            StorageEventPayload::ChildSessionNotification { notification, .. } => {
                notification.delivery.as_ref().is_some_and(|delivery| {
                    notification.child_ref.agent_id() == &watch.child.agent_id
                        && delivery.origin == ParentDeliveryOrigin::Explicit
                        && delivery.terminal_semantics == ParentDeliveryTerminalSemantics::Terminal
                        && delivery.source_turn_id.as_deref()
                            == Some(watch.execution_turn_id.as_str())
                })
            },
            _ => false,
        }))
    }
}

/// 将 Anthropic turn 终态映射为 `SubRunResult`。
///
/// 关键设计决策：`TokenExceeded` 被视为"完成"（带 handoff），而非"失败"。
/// 原因是 token 超限时 LLM 通常已输出了有价值的部分结果，
/// 父级应该能通过 typed handoff delivery 获取这些内容。
fn build_child_subrun_result(
    child: &astrcode_core::SubRunHandle,
    parent_session_id: &str,
    source_turn_id: &str,
    outcome: &astrcode_session_runtime::ProjectedTurnOutcome,
) -> SubRunResult {
    match outcome.outcome {
        AgentTurnOutcome::Completed | AgentTurnOutcome::TokenExceeded => SubRunResult::Completed {
            outcome: match outcome.outcome {
                AgentTurnOutcome::Completed => CompletedSubRunOutcome::Completed,
                AgentTurnOutcome::TokenExceeded => CompletedSubRunOutcome::TokenExceeded,
                AgentTurnOutcome::Failed | AgentTurnOutcome::Cancelled => unreachable!(),
            },
            handoff: SubRunHandoff {
                findings: Vec::new(),
                artifacts: child_handoff_artifacts(child, parent_session_id),
                delivery: Some(ParentDelivery {
                    idempotency_key: child_terminal_notification_id(
                        child.sub_run_id.as_str(),
                        source_turn_id,
                        match outcome.outcome {
                            AgentTurnOutcome::Completed => SubRunStatus::Completed,
                            AgentTurnOutcome::TokenExceeded => SubRunStatus::TokenExceeded,
                            AgentTurnOutcome::Failed => SubRunStatus::Failed,
                            AgentTurnOutcome::Cancelled => SubRunStatus::Cancelled,
                        },
                    ),
                    origin: ParentDeliveryOrigin::Fallback,
                    terminal_semantics: ParentDeliveryTerminalSemantics::Terminal,
                    source_turn_id: Some(source_turn_id.to_string()),
                    payload: ParentDeliveryPayload::Completed(CompletedParentDeliveryPayload {
                        message: outcome.summary.clone(),
                        findings: Vec::new(),
                        artifacts: child_handoff_artifacts(child, parent_session_id),
                    }),
                }),
            },
        },
        AgentTurnOutcome::Failed | AgentTurnOutcome::Cancelled => SubRunResult::Failed {
            outcome: match outcome.outcome {
                AgentTurnOutcome::Failed => FailedSubRunOutcome::Failed,
                AgentTurnOutcome::Cancelled => FailedSubRunOutcome::Cancelled,
                AgentTurnOutcome::Completed | AgentTurnOutcome::TokenExceeded => unreachable!(),
            },
            failure: SubRunFailure {
                code: match outcome.outcome {
                    AgentTurnOutcome::Cancelled => SubRunFailureCode::Interrupted,
                    AgentTurnOutcome::Failed => SubRunFailureCode::Internal,
                    AgentTurnOutcome::Completed => SubRunFailureCode::Internal,
                    AgentTurnOutcome::TokenExceeded => SubRunFailureCode::Internal,
                },
                display_message: outcome.summary.clone(),
                technical_message: outcome.technical_message.clone(),
                retryable: !matches!(outcome.outcome, AgentTurnOutcome::Cancelled),
            },
        },
    }
}

fn child_handoff_artifacts(
    child: &astrcode_core::SubRunHandle,
    parent_session_id: &str,
) -> Vec<astrcode_core::ArtifactRef> {
    child_collaboration_artifacts(child, parent_session_id, true)
}

fn child_terminal_notification_id(sub_run_id: &str, turn_id: &str, status: SubRunStatus) -> String {
    format!("child-terminal:{sub_run_id}:{turn_id}:{}", status.label())
}

/// 从 `SubRunResult` 投影出 `ChildTerminalDeliveryProjection`。
fn project_child_terminal_delivery(
    result: &SubRunResult,
    fallback_notification_id: &str,
) -> ChildTerminalDeliveryProjection {
    let status_projection = result.status();
    let last_turn_outcome = status_projection.last_turn_outcome();
    let (kind, status) = match status_projection {
        SubRunStatus::Completed | SubRunStatus::TokenExceeded => (
            ChildSessionNotificationKind::Delivered,
            AgentLifecycleStatus::Idle,
        ),
        SubRunStatus::Failed => (
            ChildSessionNotificationKind::Failed,
            AgentLifecycleStatus::Idle,
        ),
        SubRunStatus::Cancelled => (
            ChildSessionNotificationKind::Closed,
            AgentLifecycleStatus::Idle,
        ),
        SubRunStatus::Running => (
            ChildSessionNotificationKind::ProgressSummary,
            status_projection.lifecycle(),
        ),
    };

    let delivery = result
        .handoff()
        .and_then(|handoff| handoff.delivery.as_ref())
        .cloned()
        .unwrap_or_else(|| ParentDelivery {
            idempotency_key: fallback_notification_id.to_string(),
            origin: ParentDeliveryOrigin::Fallback,
            terminal_semantics: match last_turn_outcome {
                Some(AgentTurnOutcome::Completed)
                | Some(AgentTurnOutcome::TokenExceeded)
                | Some(AgentTurnOutcome::Failed)
                | Some(AgentTurnOutcome::Cancelled) => ParentDeliveryTerminalSemantics::Terminal,
                None => ParentDeliveryTerminalSemantics::NonTerminal,
            },
            source_turn_id: None,
            payload: match last_turn_outcome {
                Some(AgentTurnOutcome::Completed | AgentTurnOutcome::TokenExceeded) => {
                    let message = result
                        .handoff()
                        .and_then(|handoff| handoff.delivery.as_ref())
                        .map(|delivery| delivery.payload.message().trim())
                        .filter(|message| !message.is_empty())
                        .map(ToString::to_string)
                        .unwrap_or_else(|| match last_turn_outcome {
                            Some(AgentTurnOutcome::Completed) => {
                                "子 Agent 已完成，但没有返回可读总结。".to_string()
                            },
                            Some(AgentTurnOutcome::TokenExceeded) => {
                                "子 Agent 因 token 限额结束，但没有返回可读总结。".to_string()
                            },
                            _ => {
                                unreachable!("completed branch should only serve terminal handoff")
                            },
                        });
                    ParentDeliveryPayload::Completed(CompletedParentDeliveryPayload {
                        message,
                        findings: result
                            .handoff()
                            .map(|handoff| handoff.findings.clone())
                            .unwrap_or_default(),
                        artifacts: result
                            .handoff()
                            .map(|handoff| handoff.artifacts.clone())
                            .unwrap_or_default(),
                    })
                },
                Some(AgentTurnOutcome::Failed) => {
                    let failure = result.failure();
                    let message = failure
                        .map(|failure| failure.display_message.trim())
                        .filter(|message| !message.is_empty())
                        .map(ToString::to_string)
                        .unwrap_or_else(|| "子 Agent 失败，且没有返回可读错误信息。".to_string());
                    ParentDeliveryPayload::Failed(FailedParentDeliveryPayload {
                        message,
                        code: failure
                            .map(|failure| failure.code)
                            .unwrap_or(SubRunFailureCode::Internal),
                        technical_message: failure.map(|failure| failure.technical_message.clone()),
                        retryable: failure.is_some_and(|failure| failure.retryable),
                    })
                },
                Some(AgentTurnOutcome::Cancelled) => {
                    ParentDeliveryPayload::CloseRequest(CloseRequestParentDeliveryPayload {
                        message: "子 Agent 已关闭。".to_string(),
                        reason: Some("child_turn_cancelled".to_string()),
                    })
                },
                None => ParentDeliveryPayload::Progress(ProgressParentDeliveryPayload {
                    message: "子 Agent 状态未知。".to_string(),
                }),
            },
        });

    ChildTerminalDeliveryProjection {
        kind,
        status,
        delivery,
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use astrcode_core::{
        AgentEventContext, AgentLifecycleStatus, ChildAgentRef, ChildExecutionIdentity,
        ChildSessionNotificationKind, ParentExecutionRef, Phase, SessionId, StorageEvent,
        StorageEventPayload, SubRunStorageMode,
    };
    use astrcode_session_runtime::{append_and_broadcast, complete_session_execution};

    use super::*;
    use crate::{
        ChildSessionLineageKind,
        agent::test_support::{TestLlmBehavior, build_agent_test_harness, sample_profile},
        lifecycle::governance::ObservabilitySnapshotProvider,
    };

    fn child_completion_events(agent: AgentEventContext, turn_id: &str) -> Vec<StorageEvent> {
        vec![
            StorageEvent {
                turn_id: Some(turn_id.to_string()),
                agent: agent.clone(),
                payload: StorageEventPayload::UserMessage {
                    content: "子任务".to_string(),
                    origin: astrcode_core::UserMessageOrigin::User,
                    timestamp: chrono::Utc::now(),
                },
            },
            StorageEvent {
                turn_id: Some(turn_id.to_string()),
                agent: agent.clone(),
                payload: StorageEventPayload::AssistantFinal {
                    content: "子 Agent 总结".to_string(),
                    reasoning_content: None,
                    reasoning_signature: None,
                    timestamp: Some(chrono::Utc::now()),
                },
            },
            StorageEvent {
                turn_id: Some(turn_id.to_string()),
                agent,
                payload: StorageEventPayload::TurnDone {
                    timestamp: chrono::Utc::now(),
                    reason: Some("completed".to_string()),
                },
            },
        ]
    }

    #[tokio::test]
    async fn finalize_child_turn_appends_notification_and_triggers_parent_wake() {
        let harness = build_agent_test_harness(TestLlmBehavior::Succeed {
            content: "父级已收到交付。".to_string(),
        })
        .expect("test harness should build");
        let project = tempfile::tempdir().expect("tempdir should be created");
        let parent = harness
            .session_runtime
            .create_session(project.path().display().to_string())
            .await
            .expect("parent session should be created");
        let child = harness
            .session_runtime
            .create_session(project.path().display().to_string())
            .await
            .expect("child session should be created");
        let root = harness
            .kernel
            .agent_control()
            .register_root_agent(
                "root-agent".to_string(),
                parent.session_id.clone(),
                "root-profile".to_string(),
            )
            .await
            .expect("root agent should register");
        let child_handle = harness
            .kernel
            .agent_control()
            .spawn_with_storage(
                &sample_profile("reviewer"),
                parent.session_id.clone(),
                Some(child.session_id.clone()),
                "turn-parent".to_string(),
                Some(root.agent_id.to_string()),
                SubRunStorageMode::IndependentSession,
            )
            .await
            .expect("child handle should spawn");
        harness
            .kernel
            .agent_control()
            .set_lifecycle(&child_handle.agent_id, AgentLifecycleStatus::Running)
            .await
            .expect("child lifecycle should update");

        let child_state = harness
            .session_runtime
            .get_session_state(&SessionId::from(child.session_id.clone()))
            .await
            .expect("child state should load");
        let mut translator = astrcode_core::EventTranslator::new(Phase::Idle);
        let child_agent = AgentEventContext::from(&child_handle);
        for event in child_completion_events(child_agent, "turn-child") {
            append_and_broadcast(child_state.as_ref(), &event, &mut translator)
                .await
                .expect("child completion event should persist");
        }
        complete_session_execution(child_state.as_ref(), Phase::Idle);

        harness
            .service
            .finalize_child_turn_when_done(ChildTurnTerminalContext::new(
                child_handle.clone(),
                child.session_id.clone(),
                "turn-child".to_string(),
                parent.session_id.clone(),
                "turn-parent".to_string(),
                Some("tool-call-1".to_string()),
            ))
            .await
            .expect("child finalize should succeed");

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if harness
                .kernel
                .agent_control()
                .pending_parent_delivery_count(&parent.session_id)
                .await
                == 0
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "parent wake should drain delivery queue"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        let parent_events = harness
            .session_runtime
            .replay_stored_events(&SessionId::from(parent.session_id.clone()))
            .await
            .expect("parent events should replay");
        assert!(
            parent_events.iter().any(|stored| matches!(
                &stored.event.payload,
                StorageEventPayload::ChildSessionNotification { notification, .. }
                    if notification.kind == ChildSessionNotificationKind::Delivered
                        && notification.delivery.as_ref().is_some_and(|delivery| {
                            delivery.payload.message() == "子 Agent 总结"
                        })
            )),
            "child finalize should append terminal notification to parent session"
        );
        assert!(
            parent_events.iter().any(|stored| matches!(
                &stored.event.payload,
                StorageEventPayload::AgentInputQueued { payload }
                    if payload.envelope.message == "子 Agent 总结"
            )),
            "durable input queue message should reuse child final excerpt"
        );
        assert!(
            parent_events.iter().any(|stored| matches!(
                &stored.event.payload,
                StorageEventPayload::UserMessage { content, origin, .. }
                    if *origin == astrcode_core::UserMessageOrigin::QueuedInput
                        && content.contains("子 Agent 总结")
            )),
            "wake turn should consume the same delivery summary as queued input"
        );
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
    async fn finalize_child_turn_preserves_resume_lineage_in_notification() {
        let harness = build_agent_test_harness(TestLlmBehavior::Succeed {
            content: "父级已收到交付。".to_string(),
        })
        .expect("test harness should build");
        let project = tempfile::tempdir().expect("tempdir should be created");
        let parent = harness
            .session_runtime
            .create_session(project.path().display().to_string())
            .await
            .expect("parent session should be created");
        let child = harness
            .session_runtime
            .create_session(project.path().display().to_string())
            .await
            .expect("child session should be created");
        let root = harness
            .kernel
            .agent_control()
            .register_root_agent(
                "root-agent".to_string(),
                parent.session_id.clone(),
                "root-profile".to_string(),
            )
            .await
            .expect("root agent should register");
        let child_handle = harness
            .kernel
            .agent_control()
            .spawn_with_storage(
                &sample_profile("reviewer"),
                parent.session_id.clone(),
                Some(child.session_id.clone()),
                "turn-parent".to_string(),
                Some(root.agent_id.to_string()),
                SubRunStorageMode::IndependentSession,
            )
            .await
            .expect("child handle should spawn");
        harness
            .kernel
            .agent_control()
            .set_lifecycle(&child_handle.agent_id, AgentLifecycleStatus::Running)
            .await
            .expect("child lifecycle should update");

        let mut resumed_child_handle = child_handle.clone();
        resumed_child_handle.lineage_kind = ChildSessionLineageKind::Resume;

        let child_state = harness
            .session_runtime
            .get_session_state(&SessionId::from(child.session_id.clone()))
            .await
            .expect("child state should load");
        let mut translator = astrcode_core::EventTranslator::new(Phase::Idle);
        let child_agent = AgentEventContext::from(&resumed_child_handle);
        for event in child_completion_events(child_agent, "turn-child-resume") {
            append_and_broadcast(child_state.as_ref(), &event, &mut translator)
                .await
                .expect("child completion event should persist");
        }
        complete_session_execution(child_state.as_ref(), Phase::Idle);

        harness
            .service
            .finalize_child_turn_when_done(ChildTurnTerminalContext::new(
                resumed_child_handle,
                child.session_id.clone(),
                "turn-child-resume".to_string(),
                parent.session_id.clone(),
                "turn-parent".to_string(),
                Some("tool-call-2".to_string()),
            ))
            .await
            .expect("child finalize should succeed");

        let parent_events = harness
            .session_runtime
            .replay_stored_events(&SessionId::from(parent.session_id.clone()))
            .await
            .expect("parent events should replay");
        assert!(parent_events.iter().any(|stored| matches!(
            &stored.event.payload,
            StorageEventPayload::ChildSessionNotification { notification, .. }
                if notification.child_ref.lineage_kind == ChildSessionLineageKind::Resume
                    && notification.delivery.as_ref().is_some_and(|delivery| {
                        delivery.source_turn_id.as_deref() == Some("turn-child-resume")
                    })
        )));
    }

    #[test]
    fn project_child_terminal_delivery_preserves_explicit_envelope() {
        let explicit_delivery = ParentDelivery {
            idempotency_key: "delivery-explicit".to_string(),
            origin: ParentDeliveryOrigin::Explicit,
            terminal_semantics: ParentDeliveryTerminalSemantics::Terminal,
            source_turn_id: Some("turn-child".to_string()),
            payload: ParentDeliveryPayload::Completed(CompletedParentDeliveryPayload {
                message: "显式最终回复".to_string(),
                findings: vec!["finding-1".to_string()],
                artifacts: Vec::new(),
            }),
        };
        let result = SubRunResult::Completed {
            outcome: CompletedSubRunOutcome::Completed,
            handoff: SubRunHandoff {
                findings: vec!["finding-1".to_string()],
                artifacts: Vec::new(),
                delivery: Some(explicit_delivery.clone()),
            },
        };

        let projection = project_child_terminal_delivery(
            &result,
            "child-terminal:subrun-1:turn-child:completed",
        );

        assert_eq!(projection.kind, ChildSessionNotificationKind::Delivered);
        assert_eq!(projection.status, AgentLifecycleStatus::Idle);
        assert_eq!(projection.delivery, explicit_delivery);
    }

    #[tokio::test]
    async fn append_child_session_notification_uses_explicit_parent_session_route() {
        let harness = build_agent_test_harness(TestLlmBehavior::Succeed {
            content: "父级已收到交付。".to_string(),
        })
        .expect("test harness should build");
        let project = tempfile::tempdir().expect("tempdir should be created");
        let parent = harness
            .session_runtime
            .create_session(project.path().display().to_string())
            .await
            .expect("parent session should be created");
        let wrong_parent = harness
            .session_runtime
            .create_session(project.path().display().to_string())
            .await
            .expect("wrong parent session should be created");
        let child = harness
            .session_runtime
            .create_session(project.path().display().to_string())
            .await
            .expect("child session should be created");
        let root = harness
            .kernel
            .agent_control()
            .register_root_agent(
                "root-agent".to_string(),
                parent.session_id.clone(),
                "root-profile".to_string(),
            )
            .await
            .expect("root agent should register");
        let child_handle = harness
            .kernel
            .agent_control()
            .spawn_with_storage(
                &sample_profile("reviewer"),
                parent.session_id.clone(),
                Some(child.session_id.clone()),
                "turn-parent".to_string(),
                Some(root.agent_id.to_string()),
                SubRunStorageMode::IndependentSession,
            )
            .await
            .expect("child handle should spawn");

        harness
            .service
            .append_child_session_notification(
                &child_handle,
                &parent.session_id,
                "turn-parent",
                &ChildSessionNotification {
                    notification_id: "child-terminal:subrun-test:turn-child:completed"
                        .to_string()
                        .into(),
                    child_ref: ChildAgentRef {
                        identity: ChildExecutionIdentity {
                            agent_id: child_handle.agent_id.clone(),
                            // 故意写错：验证内部不再从 child_ref.session_id 反推父侧路由。
                            session_id: wrong_parent.session_id.clone().into(),
                            sub_run_id: child_handle.sub_run_id.clone(),
                        },
                        parent: ParentExecutionRef {
                            parent_agent_id: child_handle.parent_agent_id.clone(),
                            parent_sub_run_id: child_handle.parent_sub_run_id.clone(),
                        },
                        lineage_kind: ChildSessionLineageKind::Spawn,
                        status: AgentLifecycleStatus::Idle,
                        open_session_id: child.session_id.clone().into(),
                    },
                    kind: ChildSessionNotificationKind::Delivered,
                    source_tool_call_id: None,
                    delivery: Some(ParentDelivery {
                        idempotency_key: "child-terminal:subrun-test:turn-child:completed"
                            .to_string(),
                        origin: ParentDeliveryOrigin::Explicit,
                        terminal_semantics: ParentDeliveryTerminalSemantics::Terminal,
                        source_turn_id: Some("turn-child".to_string()),
                        payload: ParentDeliveryPayload::Completed(CompletedParentDeliveryPayload {
                            message: "最终回复".to_string(),
                            findings: Vec::new(),
                            artifacts: Vec::new(),
                        }),
                    }),
                },
            )
            .await
            .expect("explicit parent session route should succeed");

        let parent_events = harness
            .session_runtime
            .replay_stored_events(&SessionId::from(parent.session_id.clone()))
            .await
            .expect("parent events should replay");
        let wrong_parent_events = harness
            .session_runtime
            .replay_stored_events(&SessionId::from(wrong_parent.session_id.clone()))
            .await
            .expect("wrong parent events should replay");
        assert!(
            parent_events.iter().any(|stored| matches!(
                &stored.event.payload,
                StorageEventPayload::ChildSessionNotification { notification, .. }
                    if notification.delivery.as_ref().is_some_and(|delivery| {
                        delivery.payload.message() == "最终回复"
                    })
            )),
            "notification should be written to the explicit parent session"
        );
        assert!(
            !wrong_parent_events.iter().any(|stored| matches!(
                &stored.event.payload,
                StorageEventPayload::ChildSessionNotification { .. }
            )),
            "wrong child_ref.session_id must not hijack the durable notification route"
        );
    }

    #[tokio::test]
    async fn finalize_child_turn_does_not_wait_for_descendant_settlement() {
        let harness = build_agent_test_harness(TestLlmBehavior::Succeed {
            content: "父级已收到交付。".to_string(),
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
        let leaf_session = harness
            .session_runtime
            .create_session(project.path().display().to_string())
            .await
            .expect("leaf session should be created");
        let root = harness
            .kernel
            .agent_control()
            .register_root_agent(
                "root-agent".to_string(),
                root_session.session_id.clone(),
                "root-profile".to_string(),
            )
            .await
            .expect("root agent should register");
        let middle = harness
            .kernel
            .agent_control()
            .spawn_with_storage(
                &sample_profile("reviewer"),
                root_session.session_id.clone(),
                Some(middle_session.session_id.clone()),
                "turn-root".to_string(),
                Some(root.agent_id.to_string()),
                SubRunStorageMode::IndependentSession,
            )
            .await
            .expect("middle handle should spawn");
        let leaf = harness
            .kernel
            .agent_control()
            .spawn_with_storage(
                &sample_profile("explore"),
                middle_session.session_id.clone(),
                Some(leaf_session.session_id.clone()),
                "turn-middle".to_string(),
                Some(middle.agent_id.to_string()),
                SubRunStorageMode::IndependentSession,
            )
            .await
            .expect("leaf handle should spawn");
        harness
            .kernel
            .agent_control()
            .set_lifecycle(&middle.agent_id, AgentLifecycleStatus::Running)
            .await
            .expect("middle lifecycle should update");
        harness
            .kernel
            .agent_control()
            .set_lifecycle(&leaf.agent_id, AgentLifecycleStatus::Running)
            .await
            .expect("leaf lifecycle should update");

        let middle_state = harness
            .session_runtime
            .get_session_state(&SessionId::from(middle_session.session_id.clone()))
            .await
            .expect("middle state should load");
        let mut translator = astrcode_core::EventTranslator::new(Phase::Idle);
        let middle_agent = AgentEventContext::from(&middle);
        for event in child_completion_events(middle_agent, "turn-middle-wake") {
            append_and_broadcast(middle_state.as_ref(), &event, &mut translator)
                .await
                .expect("middle completion event should persist");
        }
        complete_session_execution(middle_state.as_ref(), Phase::Idle);

        harness
            .service
            .finalize_child_turn_when_done(ChildTurnTerminalContext::new(
                middle.clone(),
                middle_session.session_id.clone(),
                "turn-middle-wake".to_string(),
                root_session.session_id.clone(),
                "turn-root".to_string(),
                None,
            ))
            .await
            .expect("middle finalize should not block on running descendants");

        let root_events = harness
            .session_runtime
            .replay_stored_events(&SessionId::from(root_session.session_id.clone()))
            .await
            .expect("root events should replay");
        assert!(
            root_events.iter().any(|stored| matches!(
                &stored.event.payload,
                StorageEventPayload::ChildSessionNotification { notification, .. }
                    if notification.child_ref.agent_id() == &middle.agent_id
                        && notification.kind == ChildSessionNotificationKind::Delivered
            )),
            "middle should publish its own terminal delivery even when a descendant is still \
             running"
        );
        let leaf_status = harness
            .kernel
            .agent()
            .get_lifecycle(&leaf.agent_id)
            .await
            .expect("leaf should still exist");
        assert_eq!(
            leaf_status,
            AgentLifecycleStatus::Running,
            "running descendants should not block or be mutated by middle turn finalization"
        );
    }
}
