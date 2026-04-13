use std::time::Instant;

use astrcode_core::{
    AgentLifecycleStatus, AgentTurnOutcome, ChildAgentRef, ChildSessionLineageKind,
    ChildSessionNotification, ChildSessionNotificationKind, EventTranslator, Phase, StorageEvent,
    StorageEventPayload, SubRunFailure, SubRunFailureCode, SubRunHandoff, SubRunResult,
};

use super::{
    AgentOrchestrationError, AgentOrchestrationService, subrun_event_context_for_parent_turn,
};

struct ChildTerminalDeliveryProjection {
    kind: ChildSessionNotificationKind,
    status: AgentLifecycleStatus,
    summary: String,
    final_reply_excerpt: Option<String>,
}

/// 聚合 watcher 完成收敛所需上下文，避免终态收口函数继续堆参数。
struct ChildTerminalWatchContext {
    child: astrcode_core::SubRunHandle,
    child_session_id: String,
    child_turn_id: String,
    parent_session_id: String,
    parent_turn_id: String,
    source_tool_call_id: Option<String>,
    started_at: Instant,
}

impl AgentOrchestrationService {
    pub(super) fn spawn_child_terminal_watcher(
        &self,
        child: astrcode_core::SubRunHandle,
        child_session_id: String,
        child_turn_id: String,
        parent_session_id: String,
        parent_turn_id: String,
        source_tool_call_id: Option<String>,
    ) {
        let service = self.clone();
        let handle = tokio::spawn(async move {
            let watch = ChildTerminalWatchContext {
                child,
                child_session_id,
                child_turn_id,
                parent_session_id,
                parent_turn_id,
                source_tool_call_id,
                started_at: Instant::now(),
            };
            if let Err(error) = service.finalize_child_turn_when_done(watch).await {
                log::warn!("failed to finalize child terminal delivery: {error}");
            }
        });
        // 为什么登记：child terminal watcher 负责子任务终态收口，必须跟随治理关闭与
        // 测试 teardown 一起回收，避免后台 watcher 残留。
        self.task_registry.register_subagent_task(handle);
    }

    async fn finalize_child_turn_when_done(
        &self,
        watch: ChildTerminalWatchContext,
    ) -> Result<(), AgentOrchestrationError> {
        let terminal = self
            .wait_for_turn_terminal_snapshot(&watch.child_session_id, &watch.child_turn_id)
            .await?;
        let outcome = project_child_turn_outcome(terminal.phase, &terminal.events);
        let result = build_child_subrun_result(&watch.child, &watch.parent_session_id, &outcome);
        let _ = self
            .kernel
            .agent_control()
            .complete_turn(&watch.child.agent_id, outcome.outcome)
            .await;
        self.metrics.record_subrun_execution(
            watch.started_at.elapsed().as_millis() as u64,
            to_subrun_execution_outcome(outcome.outcome),
            None,
            None,
            Some(watch.child.storage_mode),
        );

        let delivery = project_child_terminal_delivery(&result);
        let notification = ChildSessionNotification {
            notification_id: format!(
                "child-terminal:{}:{}",
                watch.child.sub_run_id,
                status_label(result.lifecycle, result.last_turn_outcome)
            ),
            child_ref: ChildAgentRef {
                agent_id: watch.child.agent_id.clone(),
                session_id: watch.parent_session_id.clone(),
                sub_run_id: watch.child.sub_run_id.clone(),
                parent_agent_id: watch.child.parent_agent_id.clone(),
                parent_sub_run_id: watch.child.parent_sub_run_id.clone(),
                lineage_kind: ChildSessionLineageKind::Spawn,
                status: delivery.status,
                open_session_id: child_open_session_id(&watch.child),
            },
            kind: delivery.kind,
            summary: delivery.summary,
            status: delivery.status,
            source_tool_call_id: watch.source_tool_call_id,
            final_reply_excerpt: delivery.final_reply_excerpt,
        };

        self.append_child_terminal_notification(&watch.child, &watch.parent_turn_id, &notification)
            .await?;
        self.reactivate_parent_agent_if_idle(
            &watch.parent_session_id,
            &watch.parent_turn_id,
            &notification,
        )
        .await;
        Ok(())
    }

    async fn append_child_terminal_notification(
        &self,
        child: &astrcode_core::SubRunHandle,
        parent_turn_id: &str,
        notification: &ChildSessionNotification,
    ) -> Result<(), AgentOrchestrationError> {
        let parent_session_id = notification.child_ref.session_id.clone();
        let session_state = self
            .session_runtime
            .get_session_state(&astrcode_core::SessionId::from(
                astrcode_session_runtime::normalize_session_id(&parent_session_id),
            ))
            .await
            .map_err(AgentOrchestrationError::from)?;
        let mut translator = EventTranslator::new(
            session_state
                .current_phase()
                .map_err(AgentOrchestrationError::from)?,
        );
        let agent = subrun_event_context_for_parent_turn(child, parent_turn_id);
        astrcode_session_runtime::append_and_broadcast(
            &session_state,
            &StorageEvent {
                turn_id: Some(parent_turn_id.to_string()),
                agent,
                payload: StorageEventPayload::ChildSessionNotification {
                    notification: notification.clone(),
                    timestamp: Some(chrono::Utc::now()),
                },
            },
            &mut translator,
        )
        .await
        .map_err(|error| AgentOrchestrationError::Internal(error.to_string()))?;
        Ok(())
    }
}

fn build_child_subrun_result(
    child: &astrcode_core::SubRunHandle,
    parent_session_id: &str,
    outcome: &ChildTurnOutcome,
) -> SubRunResult {
    match outcome.outcome {
        AgentTurnOutcome::Completed | AgentTurnOutcome::TokenExceeded => SubRunResult {
            lifecycle: AgentLifecycleStatus::Idle,
            last_turn_outcome: Some(outcome.outcome),
            handoff: Some(SubRunHandoff {
                summary: outcome.summary.clone(),
                findings: Vec::new(),
                artifacts: child_handoff_artifacts(child, parent_session_id),
            }),
            failure: None,
        },
        AgentTurnOutcome::Failed | AgentTurnOutcome::Cancelled => SubRunResult {
            lifecycle: AgentLifecycleStatus::Idle,
            last_turn_outcome: Some(outcome.outcome),
            handoff: None,
            failure: Some(SubRunFailure {
                code: match outcome.outcome {
                    AgentTurnOutcome::Cancelled => SubRunFailureCode::Interrupted,
                    AgentTurnOutcome::Failed => SubRunFailureCode::Internal,
                    AgentTurnOutcome::Completed => SubRunFailureCode::Internal,
                    AgentTurnOutcome::TokenExceeded => SubRunFailureCode::Internal,
                },
                display_message: outcome.summary.clone(),
                technical_message: outcome.technical_message.clone(),
                retryable: !matches!(outcome.outcome, AgentTurnOutcome::Cancelled),
            }),
        },
    }
}

fn child_handoff_artifacts(
    child: &astrcode_core::SubRunHandle,
    parent_session_id: &str,
) -> Vec<astrcode_core::ArtifactRef> {
    let child_session_id = child_open_session_id(child);
    let mut artifacts = vec![
        astrcode_core::ArtifactRef {
            kind: "subRun".to_string(),
            id: child.sub_run_id.clone(),
            label: "Sub Run".to_string(),
            session_id: Some(parent_session_id.to_string()),
            storage_seq: None,
            uri: None,
        },
        astrcode_core::ArtifactRef {
            kind: "agent".to_string(),
            id: child.agent_id.clone(),
            label: "Agent".to_string(),
            session_id: Some(child_session_id.clone()),
            storage_seq: None,
            uri: None,
        },
        astrcode_core::ArtifactRef {
            kind: "parentSession".to_string(),
            id: parent_session_id.to_string(),
            label: "Parent Session".to_string(),
            session_id: Some(parent_session_id.to_string()),
            storage_seq: None,
            uri: None,
        },
        astrcode_core::ArtifactRef {
            kind: "session".to_string(),
            id: child_session_id.clone(),
            label: "Child Session".to_string(),
            session_id: Some(child_session_id),
            storage_seq: None,
            uri: None,
        },
    ];
    if let Some(parent_agent_id) = &child.parent_agent_id {
        artifacts.push(astrcode_core::ArtifactRef {
            kind: "parentAgent".to_string(),
            id: parent_agent_id.clone(),
            label: "Parent Agent".to_string(),
            session_id: Some(parent_session_id.to_string()),
            storage_seq: None,
            uri: None,
        });
    }
    if let Some(parent_sub_run_id) = &child.parent_sub_run_id {
        artifacts.push(astrcode_core::ArtifactRef {
            kind: "parentSubRun".to_string(),
            id: parent_sub_run_id.clone(),
            label: "Parent Sub Run".to_string(),
            session_id: Some(parent_session_id.to_string()),
            storage_seq: None,
            uri: None,
        });
    }
    artifacts
}

fn child_open_session_id(child: &astrcode_core::SubRunHandle) -> String {
    child
        .child_session_id
        .clone()
        .unwrap_or_else(|| child.session_id.clone())
}

struct ChildTurnOutcome {
    outcome: AgentTurnOutcome,
    summary: String,
    technical_message: String,
}

fn project_child_turn_outcome(
    phase: Phase,
    events: &[astrcode_core::StoredEvent],
) -> ChildTurnOutcome {
    let last_assistant = events
        .iter()
        .rev()
        .find_map(|stored| match &stored.event.payload {
            StorageEventPayload::AssistantFinal { content, .. } if !content.trim().is_empty() => {
                Some(content.trim().to_string())
            },
            _ => None,
        });
    let last_error = events
        .iter()
        .rev()
        .find_map(|stored| match &stored.event.payload {
            StorageEventPayload::Error { message, .. } if !message.trim().is_empty() => {
                Some(message.trim().to_string())
            },
            _ => None,
        });
    let turn_done_reason = events
        .iter()
        .rev()
        .find_map(|stored| match &stored.event.payload {
            StorageEventPayload::TurnDone { reason, .. } => reason.clone(),
            _ => None,
        });

    let outcome = if matches!(phase, Phase::Interrupted) {
        match last_error.as_deref() {
            Some("interrupted") | None => AgentTurnOutcome::Cancelled,
            Some(_) => AgentTurnOutcome::Failed,
        }
    } else if last_error.is_some() {
        AgentTurnOutcome::Failed
    } else if matches!(
        turn_done_reason.as_deref(),
        Some("budget_exhausted" | "diminishing_returns")
    ) {
        AgentTurnOutcome::TokenExceeded
    } else {
        AgentTurnOutcome::Completed
    };

    let summary = match outcome {
        AgentTurnOutcome::Completed => last_assistant
            .clone()
            .unwrap_or_else(|| "子 Agent 已完成，但没有返回可读总结。".to_string()),
        AgentTurnOutcome::TokenExceeded => last_assistant
            .clone()
            .unwrap_or_else(|| "子 Agent 因 token 限额结束，但没有返回可读总结。".to_string()),
        AgentTurnOutcome::Failed => last_error
            .clone()
            .or(last_assistant.clone())
            .unwrap_or_else(|| "子 Agent 失败，且没有返回可读错误信息。".to_string()),
        AgentTurnOutcome::Cancelled => last_error
            .clone()
            .unwrap_or_else(|| "子 Agent 已关闭。".to_string()),
    };

    ChildTurnOutcome {
        outcome,
        summary: summary.clone(),
        technical_message: last_error.unwrap_or(summary),
    }
}

fn project_child_terminal_delivery(result: &SubRunResult) -> ChildTerminalDeliveryProjection {
    let (kind, status) = match result.last_turn_outcome {
        Some(AgentTurnOutcome::Completed | AgentTurnOutcome::TokenExceeded) => (
            ChildSessionNotificationKind::Delivered,
            AgentLifecycleStatus::Idle,
        ),
        Some(AgentTurnOutcome::Failed) => (
            ChildSessionNotificationKind::Failed,
            AgentLifecycleStatus::Idle,
        ),
        Some(AgentTurnOutcome::Cancelled) => (
            ChildSessionNotificationKind::Closed,
            AgentLifecycleStatus::Idle,
        ),
        None => (
            ChildSessionNotificationKind::ProgressSummary,
            result.lifecycle,
        ),
    };

    let summary = result
        .handoff
        .as_ref()
        .map(|handoff| handoff.summary.trim())
        .filter(|summary| !summary.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            result
                .failure
                .as_ref()
                .map(|failure| failure.display_message.trim())
                .filter(|message| !message.is_empty())
                .map(ToString::to_string)
        })
        .unwrap_or_else(|| match result.last_turn_outcome {
            Some(AgentTurnOutcome::Completed) => {
                "子 Agent 已完成，但没有返回可读总结。".to_string()
            },
            Some(AgentTurnOutcome::TokenExceeded) => {
                "子 Agent 因 token 限额结束，但没有返回可读总结。".to_string()
            },
            Some(AgentTurnOutcome::Failed) => "子 Agent 失败，且没有返回可读错误信息。".to_string(),
            Some(AgentTurnOutcome::Cancelled) => "子 Agent 已关闭。".to_string(),
            None => "子 Agent 状态未知。".to_string(),
        });
    let final_reply_excerpt = result
        .handoff
        .as_ref()
        .map(|handoff| handoff.summary.trim().to_string())
        .filter(|summary| !summary.is_empty())
        .or_else(|| {
            matches!(
                result.last_turn_outcome,
                Some(AgentTurnOutcome::Completed | AgentTurnOutcome::TokenExceeded)
            )
            .then_some(summary.clone())
        });

    ChildTerminalDeliveryProjection {
        kind,
        status,
        summary,
        final_reply_excerpt,
    }
}

fn to_subrun_execution_outcome(outcome: AgentTurnOutcome) -> astrcode_core::SubRunExecutionOutcome {
    match outcome {
        AgentTurnOutcome::Completed => astrcode_core::SubRunExecutionOutcome::Completed,
        AgentTurnOutcome::Failed => astrcode_core::SubRunExecutionOutcome::Failed,
        AgentTurnOutcome::Cancelled => astrcode_core::SubRunExecutionOutcome::Aborted,
        AgentTurnOutcome::TokenExceeded => astrcode_core::SubRunExecutionOutcome::TokenExceeded,
    }
}

fn status_label(
    lifecycle: AgentLifecycleStatus,
    outcome: Option<AgentTurnOutcome>,
) -> &'static str {
    match outcome {
        Some(AgentTurnOutcome::Completed) => "completed",
        Some(AgentTurnOutcome::Cancelled) => "cancelled",
        Some(AgentTurnOutcome::Failed) => "failed",
        Some(AgentTurnOutcome::TokenExceeded) => "token_exceeded",
        None => match lifecycle {
            AgentLifecycleStatus::Pending => "pending",
            AgentLifecycleStatus::Running => "running",
            AgentLifecycleStatus::Idle => "idle",
            AgentLifecycleStatus::Terminated => "terminated",
        },
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use astrcode_core::{
        AgentEventContext, AgentLifecycleStatus, ChildSessionNotificationKind, Phase, SessionId,
        StorageEvent, StorageEventPayload, SubRunStorageMode,
    };
    use astrcode_session_runtime::{append_and_broadcast, complete_session_execution};

    use super::*;
    use crate::{
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
                Some(root.agent_id.clone()),
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
            .finalize_child_turn_when_done(ChildTerminalWatchContext {
                child: child_handle.clone(),
                child_session_id: child.session_id.clone(),
                child_turn_id: "turn-child".to_string(),
                parent_session_id: parent.session_id.clone(),
                parent_turn_id: "turn-parent".to_string(),
                source_tool_call_id: Some("tool-call-1".to_string()),
                started_at: Instant::now(),
            })
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
                        && notification.final_reply_excerpt.as_deref() == Some("子 Agent 总结")
            )),
            "child finalize should append terminal notification to parent session"
        );
        assert!(
            parent_events.iter().any(|stored| matches!(
                &stored.event.payload,
                StorageEventPayload::AgentMailboxQueued { payload }
                    if payload.envelope.message == "子 Agent 总结"
            )),
            "durable mailbox message should reuse child final excerpt"
        );
        assert!(
            parent_events.iter().any(|stored| matches!(
                &stored.event.payload,
                StorageEventPayload::UserMessage { content, .. }
                    if content.contains("delivery_id: child-terminal:")
                        && content.contains("message: 子 Agent 总结")
            )),
            "wake prompt should consume the same delivery summary"
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
}
