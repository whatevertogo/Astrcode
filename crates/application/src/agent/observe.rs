//! # 四工具模型 — Observe 实现
//!
//! `observe` 是四工具模型（send / observe / close / interrupt）中的只读观察操作。
//! 从旧 runtime/service/agent/observe.rs 迁入，去掉对 RuntimeService 的依赖。
//!
//! 快照聚合两层：
//! 1. 从 kernel AgentControl 获取 lifecycle / turn_outcome
//! 2. 从 session-runtime 获取稳定 observe 视图

use astrcode_core::{
    AgentCollaborationActionKind, AgentCollaborationOutcomeKind, AgentLifecycleStatus,
    CollaborationResult, CollaborationResultKind, ObserveAgentResult, ObserveParams,
};

use super::{AgentOrchestrationService, CollaborationFactRecord};

impl AgentOrchestrationService {
    /// 获取目标 child agent 的增强快照（四工具模型 observe）。
    pub async fn observe_child(
        &self,
        params: ObserveParams,
        ctx: &astrcode_core::ToolContext,
    ) -> Result<CollaborationResult, super::AgentOrchestrationError> {
        let runtime = self.resolve_runtime_config_for_working_dir(ctx.working_dir())?;
        let turn_id = ctx.turn_id().unwrap_or("unknown-turn").to_string();
        let parent_session_id = ctx.session_id().to_string();
        let parent_agent_id = ctx.agent_context().agent_id.clone();
        let source_tool_call_id = ctx.tool_call_id().map(ToString::to_string);
        params
            .validate()
            .map_err(super::AgentOrchestrationError::from)?;

        let child = match self.kernel.get_agent_handle(&params.agent_id).await {
            Some(child) => child,
            None => {
                let error = super::AgentOrchestrationError::NotFound(format!(
                    "agent '{}' not found",
                    params.agent_id
                ));
                let _ = self
                    .record_collaboration_fact(
                        &runtime,
                        CollaborationFactRecord {
                            action: AgentCollaborationActionKind::Observe,
                            outcome: AgentCollaborationOutcomeKind::Rejected,
                            session_id: &parent_session_id,
                            turn_id: &turn_id,
                            parent_agent_id: parent_agent_id.clone(),
                            child: None,
                            delivery_id: None,
                            reason_code: Some("child_not_found".to_string()),
                            summary: Some(error.to_string()),
                            latency_ms: None,
                            source_tool_call_id: source_tool_call_id.clone(),
                        },
                    )
                    .await;
                return Err(error);
            },
        };

        if let Err(error) = self.verify_caller_owns_child(ctx, &child) {
            let _ = self
                .record_collaboration_fact(
                    &runtime,
                    CollaborationFactRecord {
                        action: AgentCollaborationActionKind::Observe,
                        outcome: AgentCollaborationOutcomeKind::Rejected,
                        session_id: &parent_session_id,
                        turn_id: &turn_id,
                        parent_agent_id: parent_agent_id.clone(),
                        child: Some(&child),
                        delivery_id: None,
                        reason_code: Some("ownership_mismatch".to_string()),
                        summary: Some(error.to_string()),
                        latency_ms: None,
                        source_tool_call_id: source_tool_call_id.clone(),
                    },
                )
                .await;
            return Err(error);
        }

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

        let observe_snapshot = self
            .session_runtime
            .observe_agent_session(&open_session_id, &params.agent_id, lifecycle_status)
            .await
            .map_err(|e| {
                super::AgentOrchestrationError::Internal(format!(
                    "failed to build observe snapshot: {e}"
                ))
            })?;
        let recommended_next_action = recommended_next_action(
            lifecycle_status,
            observe_snapshot.pending_message_count,
            observe_snapshot.active_task.as_deref(),
            observe_snapshot.pending_task.as_deref(),
        )
        .to_string();
        let recommended_reason = recommended_reason(
            lifecycle_status,
            last_turn_outcome,
            observe_snapshot.pending_message_count,
            observe_snapshot.active_task.as_deref(),
            observe_snapshot.pending_task.as_deref(),
        );
        let delivery_freshness = delivery_freshness(
            lifecycle_status,
            observe_snapshot.pending_message_count,
            observe_snapshot.active_task.as_deref(),
            observe_snapshot.pending_task.as_deref(),
        )
        .to_string();

        let observe_result = ObserveAgentResult {
            agent_id: child.agent_id.clone(),
            sub_run_id: child.sub_run_id.clone(),
            session_id: child.session_id.clone(),
            open_session_id,
            parent_agent_id: child.parent_agent_id.clone().unwrap_or_default(),
            lifecycle_status,
            last_turn_outcome,
            phase: format!("{:?}", observe_snapshot.phase),
            turn_count: observe_snapshot.turn_count,
            pending_message_count: observe_snapshot.pending_message_count,
            active_task: observe_snapshot.active_task,
            pending_task: observe_snapshot.pending_task,
            recent_mailbox_messages: observe_snapshot.recent_mailbox_messages,
            last_output: observe_snapshot.last_output,
            recommended_next_action,
            recommended_reason,
            delivery_freshness,
        };

        log::info!(
            "observe: snapshot for child agent '{}' (lifecycle={:?}, pending={})",
            params.agent_id,
            lifecycle_status,
            observe_result.pending_message_count
        );
        let _ = self
            .record_collaboration_fact(
                &runtime,
                CollaborationFactRecord {
                    action: AgentCollaborationActionKind::Observe,
                    outcome: AgentCollaborationOutcomeKind::Accepted,
                    session_id: &parent_session_id,
                    turn_id: &turn_id,
                    parent_agent_id,
                    child: Some(&child),
                    delivery_id: None,
                    reason_code: None,
                    summary: Some(format_observe_summary(&observe_result)),
                    latency_ms: None,
                    source_tool_call_id,
                },
            )
            .await;

        Ok(CollaborationResult {
            accepted: true,
            kind: CollaborationResultKind::Observed,
            agent_ref: Some(
                self.project_child_ref_status(self.build_child_ref_from_handle(&child).await)
                    .await,
            ),
            delivery_id: None,
            summary: Some(format_observe_summary(&observe_result)),
            observe_result: Some(observe_result),
            cascade: None,
            closed_root_agent_id: None,
            failure: None,
        })
    }
}

fn recommended_next_action(
    lifecycle_status: AgentLifecycleStatus,
    pending_message_count: usize,
    active_task: Option<&str>,
    pending_task: Option<&str>,
) -> &'static str {
    match lifecycle_status {
        AgentLifecycleStatus::Pending | AgentLifecycleStatus::Running => "wait",
        AgentLifecycleStatus::Terminated => "none",
        AgentLifecycleStatus::Idle if active_task.is_some() || pending_task.is_some() => "wait",
        AgentLifecycleStatus::Idle if pending_message_count > 0 => "wait",
        AgentLifecycleStatus::Idle => "send_or_close",
    }
}

fn recommended_reason(
    lifecycle_status: AgentLifecycleStatus,
    last_turn_outcome: Option<astrcode_core::AgentTurnOutcome>,
    pending_message_count: usize,
    active_task: Option<&str>,
    pending_task: Option<&str>,
) -> String {
    match lifecycle_status {
        AgentLifecycleStatus::Pending | AgentLifecycleStatus::Running => {
            if let Some(task) = active_task {
                format!("子 Agent 仍在处理当前任务：{task}")
            } else if let Some(task) = pending_task {
                format!("子 Agent 还有待消费的 mailbox 任务：{task}")
            } else {
                "子 Agent 仍在执行中，当前更适合继续等待。".to_string()
            }
        },
        AgentLifecycleStatus::Terminated => "子 Agent 已终止，不能再接收 \
                                             send；如需继续工作应改由当前 Agent 或新的分支处理。"
            .to_string(),
        AgentLifecycleStatus::Idle if active_task.is_some() || pending_task.is_some() => {
            "子 Agent 当前空闲，但还有待处理任务痕迹，先等待当前 mailbox 周期稳定。".to_string()
        },
        AgentLifecycleStatus::Idle if pending_message_count > 0 => {
            "子 Agent 已空闲，但 mailbox 里仍有待处理消息；先等待这些消息被消费。".to_string()
        },
        AgentLifecycleStatus::Idle => match last_turn_outcome {
            Some(astrcode_core::AgentTurnOutcome::Completed) => "子 Agent 已完成上一轮工作；\
                                                                 如果责任边界不变可直接 send \
                                                                 复用，否则 close 结束该分支。"
                .to_string(),
            Some(astrcode_core::AgentTurnOutcome::Failed) => {
                "子 Agent 上一轮失败；若要继续同一责任可 send 明确返工要求，否则 close 止损。"
                    .to_string()
            },
            Some(astrcode_core::AgentTurnOutcome::Cancelled) => {
                "子 Agent 上一轮已取消；通常更适合 close，只有在确实要复用同一分支时才 send。"
                    .to_string()
            },
            Some(astrcode_core::AgentTurnOutcome::TokenExceeded) => {
                "子 Agent 上一轮受 token 限制中断；若继续复用，请先收窄任务范围后再 send。"
                    .to_string()
            },
            None => "子 Agent 当前空闲，可根据责任是否继续存在来选择 send 或 close。".to_string(),
        },
    }
}

fn delivery_freshness(
    lifecycle_status: AgentLifecycleStatus,
    pending_message_count: usize,
    active_task: Option<&str>,
    pending_task: Option<&str>,
) -> &'static str {
    match lifecycle_status {
        AgentLifecycleStatus::Pending | AgentLifecycleStatus::Running => "pending_child_work",
        AgentLifecycleStatus::Terminated => "terminated",
        AgentLifecycleStatus::Idle if active_task.is_some() || pending_task.is_some() => {
            "pending_child_work"
        },
        AgentLifecycleStatus::Idle if pending_message_count > 0 => "pending_child_work",
        AgentLifecycleStatus::Idle => "ready_for_follow_up",
    }
}

fn format_observe_summary(result: &ObserveAgentResult) -> String {
    let base = format!(
        "子 Agent {} 当前为 {:?}；建议 {}：{}",
        result.agent_id,
        result.lifecycle_status,
        result.recommended_next_action,
        result.recommended_reason
    );
    if result.recent_mailbox_messages.is_empty() {
        return base;
    }

    format!(
        "{base}；最近 mailbox 摘要：{}",
        result.recent_mailbox_messages.join(" | ")
    )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use astrcode_core::{
        AgentCollaborationActionKind, AgentCollaborationOutcomeKind, CancelToken, ObserveParams,
        SessionId, StorageEventPayload, ToolContext,
        agent::executor::{CollaborationExecutor, SubAgentExecutor},
    };
    use tokio::time::sleep;

    use super::{
        delivery_freshness, format_observe_summary, recommended_next_action, recommended_reason,
    };
    use crate::agent::test_support::{TestLlmBehavior, build_agent_test_harness};

    #[test]
    fn recommendation_helpers_prefer_wait_for_running_child() {
        assert_eq!(
            recommended_next_action(
                astrcode_core::AgentLifecycleStatus::Running,
                0,
                Some("scan repo"),
                None,
            ),
            "wait"
        );
        assert_eq!(
            delivery_freshness(
                astrcode_core::AgentLifecycleStatus::Running,
                0,
                Some("scan repo"),
                None,
            ),
            "pending_child_work"
        );
        assert!(
            recommended_reason(
                astrcode_core::AgentLifecycleStatus::Running,
                None,
                0,
                Some("scan repo"),
                None,
            )
            .contains("scan repo")
        );
    }

    #[test]
    fn recommendation_helpers_prefer_send_or_close_for_idle_child() {
        assert_eq!(
            recommended_next_action(astrcode_core::AgentLifecycleStatus::Idle, 0, None, None),
            "send_or_close"
        );
        assert_eq!(
            delivery_freshness(astrcode_core::AgentLifecycleStatus::Idle, 0, None, None),
            "ready_for_follow_up"
        );
    }

    #[test]
    fn observe_summary_is_decision_oriented() {
        let result = astrcode_core::ObserveAgentResult {
            agent_id: "agent-7".to_string(),
            sub_run_id: "subrun-7".to_string(),
            session_id: "session-parent".to_string(),
            open_session_id: "session-child".to_string(),
            parent_agent_id: "agent-root".to_string(),
            lifecycle_status: astrcode_core::AgentLifecycleStatus::Idle,
            last_turn_outcome: Some(astrcode_core::AgentTurnOutcome::Completed),
            phase: "Idle".to_string(),
            turn_count: 1,
            pending_message_count: 0,
            active_task: None,
            pending_task: None,
            recent_mailbox_messages: vec!["最近一条消息".to_string()],
            last_output: Some("done".to_string()),
            recommended_next_action: "send_or_close".to_string(),
            recommended_reason: "上一轮已完成".to_string(),
            delivery_freshness: "ready_for_follow_up".to_string(),
        };

        let summary = format_observe_summary(&result);
        assert!(summary.contains("建议 send_or_close"));
        assert!(summary.contains("agent-7"));
        assert!(summary.contains("最近 mailbox 摘要"));
    }

    #[tokio::test]
    async fn observe_child_returns_projection_for_direct_child() {
        let harness = build_agent_test_harness(TestLlmBehavior::Succeed {
            content: "初始工作完成。".to_string(),
        })
        .expect("test harness should build");
        let project = tempfile::tempdir().expect("tempdir should be created");
        let parent = harness
            .session_runtime
            .create_session(project.path().display().to_string())
            .await
            .expect("parent session should be created");
        harness
            .kernel
            .agent_control()
            .register_root_agent(
                "root-agent".to_string(),
                parent.session_id.clone(),
                "root-profile".to_string(),
            )
            .await
            .expect("root agent should be registered");
        let parent_ctx = ToolContext::new(
            parent.session_id.clone().into(),
            project.path().to_path_buf(),
            CancelToken::new(),
        )
        .with_turn_id("turn-parent")
        .with_agent_context(super::super::root_execution_event_context(
            "root-agent",
            "root-profile",
        ));

        let launched = harness
            .service
            .launch(
                astrcode_core::SpawnAgentParams {
                    r#type: Some("reviewer".to_string()),
                    description: "检查 crates".to_string(),
                    prompt: "请检查 crates 目录".to_string(),
                    context: None,
                },
                &parent_ctx,
            )
            .await
            .expect("spawn should succeed");
        let child_agent_id = launched
            .handoff
            .as_ref()
            .and_then(|handoff| {
                handoff
                    .artifacts
                    .iter()
                    .find(|artifact| artifact.kind == "agent")
                    .map(|artifact| artifact.id.clone())
            })
            .expect("child agent artifact should exist");
        for _ in 0..20 {
            if harness
                .kernel
                .get_agent_lifecycle(&child_agent_id)
                .await
                .is_some_and(|lifecycle| lifecycle == astrcode_core::AgentLifecycleStatus::Idle)
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }

        let result = harness
            .service
            .observe(
                ObserveParams {
                    agent_id: child_agent_id,
                },
                &parent_ctx,
            )
            .await
            .expect("observe should succeed");

        let observe_result = result.observe_result.expect("observe result should exist");
        assert_eq!(observe_result.recommended_next_action, "send_or_close");
        assert_eq!(observe_result.delivery_freshness, "ready_for_follow_up");
        assert!(
            result
                .summary
                .unwrap_or_default()
                .contains("建议 send_or_close")
        );

        let parent_events = harness
            .session_runtime
            .replay_stored_events(&SessionId::from(parent.session_id.clone()))
            .await
            .expect("parent events should replay");
        assert!(parent_events.iter().any(|stored| matches!(
            &stored.event.payload,
            StorageEventPayload::AgentCollaborationFact { fact, .. }
                if fact.action == AgentCollaborationActionKind::Observe
                    && fact.outcome == AgentCollaborationOutcomeKind::Accepted
                    && fact.child_agent_id.as_deref() == Some(observe_result.agent_id.as_str())
        )));
    }
}
