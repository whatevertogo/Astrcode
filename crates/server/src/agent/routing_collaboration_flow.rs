use super::*;

#[path = "routing/child_send.rs"]
mod child_send;
#[path = "routing/parent_delivery.rs"]
mod parent_delivery;

impl AgentOrchestrationService {
    /// 关闭子 agent 及其整个子树（close 协作工具的业务逻辑）。
    ///
    /// 流程：
    /// 1. 验证调用者是目标子 agent 的直接父级
    /// 2. 收集子树所有 handle 用于 durable discard
    /// 3. 持久化 InputDiscarded 事件，标记待处理消息为已丢弃
    /// 4. 执行 kernel.terminate_subtree() 级联终止
    /// 5. 记录 Close collaboration fact
    pub(in crate::agent) async fn close_child(
        &self,
        params: CloseAgentParams,
        ctx: &astrcode_core::ToolContext,
    ) -> Result<CollaborationResult, super::AgentOrchestrationError> {
        let collaboration = self.tool_collaboration_context(ctx).await?;
        params
            .validate()
            .map_err(super::AgentOrchestrationError::from)?;

        let target = self
            .require_direct_child_handle(
                &params.agent_id,
                AgentCollaborationActionKind::Close,
                ctx,
                &collaboration,
            )
            .await?;

        let subtree_handles = self.kernel.collect_subtree_handles(&params.agent_id).await;
        let mut discard_targets = Vec::with_capacity(subtree_handles.len() + 1);
        discard_targets.push(target.clone());
        discard_targets.extend(subtree_handles.iter().cloned());

        self.append_durable_input_queue_discard_batch(&discard_targets, ctx)
            .await?;

        let cancelled = self
            .kernel
            .terminate_subtree(&params.agent_id)
            .await
            .ok_or_else(|| {
                super::AgentOrchestrationError::NotFound(format!(
                    "agent '{}' terminate failed (not found or already finalized)",
                    params.agent_id
                ))
            })?;

        let subtree_count = subtree_handles.len();
        let summary = if subtree_count > 0 {
            format!(
                "已级联关闭子 Agent {} 及 {} 个后代。",
                params.agent_id, subtree_count
            )
        } else {
            format!("已关闭子 Agent {}。", params.agent_id)
        };
        self.record_fact_best_effort(
            collaboration.runtime(),
            collaboration
                .fact(
                    AgentCollaborationActionKind::Close,
                    AgentCollaborationOutcomeKind::Closed,
                )
                .child(&target)
                .summary(summary.clone()),
        )
        .await;

        Ok(CollaborationResult::Closed {
            continuation: Some(astrcode_core::ExecutionContinuation::child_agent(
                self.build_child_ref_from_handle(&target).await,
            )),
            summary: Some(summary),
            cascade: true,
            closed_root_agent_id: cancelled.agent_id.clone(),
        })
    }

    /// 从 SubRunHandle 构造 ChildAgentRef。
    pub(in crate::agent) async fn build_child_ref_from_handle(
        &self,
        handle: &SubRunHandle,
    ) -> ChildAgentRef {
        handle.child_ref()
    }

    /// 用 live 控制面的最新 lifecycle 投影更新 ChildAgentRef。
    pub(in crate::agent) async fn project_child_ref_status(
        &self,
        mut child_ref: ChildAgentRef,
    ) -> ChildAgentRef {
        let lifecycle = self.kernel.get_lifecycle(child_ref.agent_id()).await;
        let last_turn_outcome = self.kernel.get_turn_outcome(child_ref.agent_id()).await;
        if let Some(lifecycle) = lifecycle {
            child_ref.status =
                project_collaboration_lifecycle(lifecycle, last_turn_outcome, child_ref.status);
        }
        child_ref
    }
}

pub(super) fn parent_delivery_label(payload: &ParentDeliveryPayload) -> &'static str {
    parent_delivery::parent_delivery_label(payload)
}

/// 将 live 控制面的 lifecycle + outcome 投影回 `ChildAgentRef` 的 lifecycle。
///
/// `Idle` + `None` outcome 的含义是：agent 已空闲但还没有完成过一轮 turn，
/// 此时保留调用方传入的 fallback 状态（通常是 handle 上当前记录的 lifecycle）。
/// 这避免了把刚 spawn 还没执行过 turn 的 agent 误标为 Idle。
fn project_collaboration_lifecycle(
    lifecycle: AgentLifecycleStatus,
    last_turn_outcome: Option<astrcode_core::AgentTurnOutcome>,
    fallback: AgentLifecycleStatus,
) -> AgentLifecycleStatus {
    match lifecycle {
        AgentLifecycleStatus::Pending => AgentLifecycleStatus::Pending,
        AgentLifecycleStatus::Running => AgentLifecycleStatus::Running,
        AgentLifecycleStatus::Idle => match last_turn_outcome {
            Some(_) => AgentLifecycleStatus::Idle,
            None => fallback,
        },
        AgentLifecycleStatus::Terminated => AgentLifecycleStatus::Terminated,
    }
}
