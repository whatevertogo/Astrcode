//! # 四工具模型 — Observe 实现
//!
//! `observe` 现在只返回只读快照，不再派生下一步建议，也不再暴露 input queue
//! 的内部补洞语义。

use astrcode_core::{
    AgentCollaborationActionKind, AgentCollaborationOutcomeKind, AgentLifecycleStatus,
    CollaborationResult, ObserveParams, ObserveSnapshot,
};

use super::{AgentOrchestrationService, ObserveSnapshotSignature};

impl AgentOrchestrationService {
    /// 获取目标 child agent 的只读快照。
    pub async fn observe_child(
        &self,
        params: ObserveParams,
        ctx: &astrcode_core::ToolContext,
    ) -> Result<CollaborationResult, super::AgentOrchestrationError> {
        let collaboration = self.tool_collaboration_context(ctx)?;
        params
            .validate()
            .map_err(super::AgentOrchestrationError::from)?;

        let child = self
            .require_direct_child_handle(
                &params.agent_id,
                AgentCollaborationActionKind::Observe,
                ctx,
                &collaboration,
            )
            .await?;

        let lifecycle_status = self
            .kernel
            .get_lifecycle(&params.agent_id)
            .await
            .unwrap_or(AgentLifecycleStatus::Pending);
        let last_turn_outcome = self.kernel.get_turn_outcome(&params.agent_id).await;

        let open_session_id = child
            .child_session_id
            .clone()
            .unwrap_or_else(|| child.session_id.clone());

        let snapshot = self
            .session_runtime
            .observe_agent_session(&open_session_id, &params.agent_id, lifecycle_status)
            .await
            .map_err(|e| {
                super::AgentOrchestrationError::Internal(format!(
                    "failed to build observe snapshot: {e}"
                ))
            })?;

        let observe_result = ObserveSnapshot {
            agent_id: child.agent_id.to_string(),
            session_id: open_session_id.to_string(),
            lifecycle_status,
            last_turn_outcome,
            phase: format!("{:?}", snapshot.phase),
            turn_count: snapshot.turn_count,
            active_task: snapshot.active_task,
            last_output_tail: snapshot.last_output_tail,
            last_turn_tail: snapshot.last_turn_tail,
        };
        let signature = observe_signature(&observe_result);
        if self.observe_snapshot_is_unchanged(&child, &collaboration, &signature)? {
            let error = super::AgentOrchestrationError::InvalidInput(
                "child state is unchanged since the previous observe in this turn; wait with \
                 shell sleep and observe again only after a new delivery or state change"
                    .to_string(),
            );
            return self
                .reject_with_fact(
                    collaboration.runtime(),
                    collaboration
                        .fact(
                            AgentCollaborationActionKind::Observe,
                            AgentCollaborationOutcomeKind::Rejected,
                        )
                        .child(&child)
                        .reason_code("state_unchanged")
                        .summary(error.to_string()),
                    error,
                )
                .await;
        }
        self.remember_observe_snapshot(&child, &collaboration, signature)?;

        log::info!(
            "observe: snapshot for child agent '{}' (lifecycle={:?}, phase={})",
            params.agent_id,
            lifecycle_status,
            observe_result.phase
        );
        self.record_fact_best_effort(
            collaboration.runtime(),
            collaboration
                .fact(
                    AgentCollaborationActionKind::Observe,
                    AgentCollaborationOutcomeKind::Accepted,
                )
                .child(&child)
                .summary(format_observe_summary(
                    &observe_result,
                    child.delegation.as_ref(),
                )),
        )
        .await;

        Ok(CollaborationResult::Observed {
            agent_ref: self
                .project_child_ref_status(self.build_child_ref_from_handle(&child).await)
                .await,
            summary: format_observe_summary(&observe_result, child.delegation.as_ref()),
            observe_result: Box::new(observe_result),
            delegation: child.delegation.clone(),
        })
    }

    fn observe_snapshot_is_unchanged(
        &self,
        child: &astrcode_core::SubRunHandle,
        collaboration: &super::ToolCollaborationContext,
        signature: &ObserveSnapshotSignature,
    ) -> std::result::Result<bool, super::AgentOrchestrationError> {
        let guard_key = observe_guard_key(child, collaboration);
        let guard = self.observe_guard.lock().map_err(|_| {
            super::AgentOrchestrationError::Internal("observe guard lock poisoned".to_string())
        })?;
        Ok(guard.is_unchanged(&guard_key, signature))
    }

    fn remember_observe_snapshot(
        &self,
        child: &astrcode_core::SubRunHandle,
        collaboration: &super::ToolCollaborationContext,
        signature: ObserveSnapshotSignature,
    ) -> std::result::Result<(), super::AgentOrchestrationError> {
        let guard_key = observe_guard_key(child, collaboration);
        let mut guard = self.observe_guard.lock().map_err(|_| {
            super::AgentOrchestrationError::Internal("observe guard lock poisoned".to_string())
        })?;
        guard.remember(guard_key, signature);
        Ok(())
    }
}

fn observe_guard_key(
    child: &astrcode_core::SubRunHandle,
    collaboration: &super::ToolCollaborationContext,
) -> String {
    format!(
        "{}:{}:{}:{}",
        collaboration.session_id(),
        collaboration.turn_id(),
        collaboration.parent_agent_id().unwrap_or_default(),
        child.agent_id
    )
}

fn observe_signature(result: &ObserveSnapshot) -> ObserveSnapshotSignature {
    ObserveSnapshotSignature {
        lifecycle_status: result.lifecycle_status,
        last_turn_outcome: result.last_turn_outcome,
        phase: result.phase.clone(),
        turn_count: result.turn_count,
        active_task: result.active_task.clone(),
        last_output_tail: result.last_output_tail.clone(),
        last_turn_tail: result.last_turn_tail.clone(),
    }
}

fn format_observe_summary(
    result: &ObserveSnapshot,
    delegation: Option<&astrcode_core::DelegationMetadata>,
) -> String {
    let mut parts = Vec::new();
    parts.push(format!(
        "子 Agent {} 当前为 {:?}",
        result.agent_id, result.lifecycle_status
    ));
    if let Some(metadata) = delegation {
        parts.push(format!("责任分支：{}", metadata.responsibility_summary));
    }
    if let Some(task) = result.active_task.as_deref() {
        parts.push(format!("当前任务：{task}"));
    }
    if let Some(output) = result.last_output_tail.as_deref() {
        parts.push(format!("最近输出：{output}"));
    }
    if !result.last_turn_tail.is_empty() {
        parts.push(format!(
            "最后一轮尾部：{}",
            result.last_turn_tail.join(" | ")
        ));
    }
    parts.join("；")
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

    use super::format_observe_summary;
    use crate::agent::{
        ObserveGuardState, ObserveSnapshotSignature,
        test_support::{TestLlmBehavior, build_agent_test_harness},
    };

    #[test]
    fn observe_summary_is_snapshot_oriented() {
        let result = astrcode_core::ObserveSnapshot {
            agent_id: "agent-7".to_string(),
            session_id: "session-child".to_string(),
            lifecycle_status: astrcode_core::AgentLifecycleStatus::Idle,
            last_turn_outcome: Some(astrcode_core::AgentTurnOutcome::Completed),
            phase: "Idle".to_string(),
            turn_count: 1,
            active_task: Some("整理结论".to_string()),
            last_output_tail: Some("done".to_string()),
            last_turn_tail: vec!["最近一条消息".to_string()],
        };

        let summary = format_observe_summary(&result, None);
        assert!(summary.contains("agent-7"));
        assert!(summary.contains("当前任务：整理结论"));
        assert!(summary.contains("最后一轮尾部"));
    }

    #[test]
    fn observe_guard_state_is_bounded() {
        let mut state = ObserveGuardState::default();
        for index in 0..1100 {
            state.remember(
                format!("session:turn-{index}:parent:child"),
                ObserveSnapshotSignature {
                    lifecycle_status: astrcode_core::AgentLifecycleStatus::Idle,
                    last_turn_outcome: Some(astrcode_core::AgentTurnOutcome::Completed),
                    phase: "Idle".to_string(),
                    turn_count: index as u32,
                    active_task: None,
                    last_output_tail: None,
                    last_turn_tail: Vec::new(),
                },
            );
        }

        assert!(
            state.entries.len() <= 1024,
            "observe guard should evict old entries instead of unbounded growth"
        );
        assert!(
            state.entries.contains_key("session:turn-1099:parent:child"),
            "latest observe snapshot should be retained"
        );
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
                    capability_grant: None,
                },
                &parent_ctx,
            )
            .await
            .expect("spawn should succeed");
        let child_agent_id = launched
            .handoff()
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
                .agent()
                .get_lifecycle(&child_agent_id)
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

        let observe_result = result
            .observe_result()
            .expect("observe result should exist");
        assert_eq!(
            observe_result.lifecycle_status,
            astrcode_core::AgentLifecycleStatus::Idle
        );
        assert!(result.summary().unwrap_or_default().contains("子 Agent"));

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
                    && fact.child_agent_id().map(|id| id.as_str())
                        == Some(observe_result.agent_id.as_str())
        )));
    }

    #[tokio::test]
    async fn observe_child_rejects_unchanged_snapshot_in_same_turn() {
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
                    capability_grant: None,
                },
                &parent_ctx,
            )
            .await
            .expect("spawn should succeed");
        let child_agent_id = launched
            .handoff()
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
                .agent()
                .get_lifecycle(&child_agent_id)
                .await
                .is_some_and(|lifecycle| lifecycle == astrcode_core::AgentLifecycleStatus::Idle)
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }

        harness
            .service
            .observe(
                ObserveParams {
                    agent_id: child_agent_id.clone(),
                },
                &parent_ctx,
            )
            .await
            .expect("first observe should succeed");

        let error = harness
            .service
            .observe(
                ObserveParams {
                    agent_id: child_agent_id.clone(),
                },
                &parent_ctx,
            )
            .await
            .expect_err("second observe should reject unchanged snapshot");

        assert!(error.to_string().contains("child state is unchanged"));

        let parent_events = harness
            .session_runtime
            .replay_stored_events(&SessionId::from(parent.session_id.clone()))
            .await
            .expect("parent events should replay");
        assert!(parent_events.iter().any(|stored| matches!(
            &stored.event.payload,
            StorageEventPayload::AgentCollaborationFact { fact, .. }
                if fact.action == AgentCollaborationActionKind::Observe
                    && fact.outcome == AgentCollaborationOutcomeKind::Rejected
                    && fact.reason_code.as_deref() == Some("state_unchanged")
                    && fact.child_agent_id().map(|id| id.as_str()) == Some(child_agent_id.as_str())
        )));
    }
}
