//! agent 协作路由与权限校验。
//!
//! 从旧 runtime/service/agent/routing.rs 迁入，去掉对 RuntimeService 的依赖，
//! 改为通过 Kernel + SessionRuntime 完成所有操作。

use astrcode_core::{
    AgentLifecycleStatus, ChildAgentRef, ChildSessionLineageKind, CloseAgentParams,
    CollaborationResult, CollaborationResultKind, InboxEnvelopeKind, SendAgentParams, SubRunHandle,
};

use super::AgentOrchestrationService;

impl AgentOrchestrationService {
    /// 验证调用者是否为目标子 agent 的直接父级。
    pub(super) fn verify_caller_owns_child(
        &self,
        ctx: &astrcode_core::ToolContext,
        child_handle: &SubRunHandle,
    ) -> Result<(), super::AgentOrchestrationError> {
        let caller_agent_id = ctx.agent_context().agent_id.as_deref();
        let child_parent_id = child_handle.parent_agent_id.as_deref();

        match (caller_agent_id, child_parent_id) {
            (Some(caller), Some(parent)) if caller == parent => Ok(()),
            (None, None) => Ok(()),
            _ => Err(super::AgentOrchestrationError::InvalidInput(format!(
                "caller '{}' does not own agent '{}' (parent: {})",
                caller_agent_id.unwrap_or("<root>"),
                child_handle.agent_id,
                child_parent_id.unwrap_or("<none>")
            ))),
        }
    }

    /// 向子 Agent 追加消息（send 协作工具的业务逻辑）。
    pub async fn send_to_child(
        &self,
        params: SendAgentParams,
        ctx: &astrcode_core::ToolContext,
    ) -> Result<CollaborationResult, super::AgentOrchestrationError> {
        params
            .validate()
            .map_err(super::AgentOrchestrationError::from)?;

        let child = self
            .kernel
            .agent_control()
            .get(&params.agent_id)
            .await
            .ok_or_else(|| {
                super::AgentOrchestrationError::NotFound(format!(
                    "agent '{}' not found",
                    params.agent_id
                ))
            })?;

        self.verify_caller_owns_child(ctx, &child)?;

        let lifecycle = self
            .kernel
            .agent_control()
            .get_lifecycle(&params.agent_id)
            .await;
        if matches!(lifecycle, Some(AgentLifecycleStatus::Terminated)) {
            return Err(super::AgentOrchestrationError::InvalidInput(format!(
                "agent '{}' has been terminated and cannot receive new messages",
                params.agent_id
            )));
        }

        // idle child 需要重新启动以消费新消息
        if matches!(lifecycle, Some(AgentLifecycleStatus::Idle)) && !child.lifecycle.occupies_slot()
        {
            let pending = self
                .kernel
                .agent_control()
                .drain_inbox(&child.agent_id)
                .await
                .unwrap_or_default();
            let resume_message = super::mailbox::compose_reusable_child_message(&pending, &params);

            // 通过 kernel resume 机制重启子 agent
            if let Some(reused_handle) = self.kernel.agent_control().resume(&params.agent_id).await
            {
                log::info!(
                    "send: reusable child agent '{}' restarted with new turn (subRunId='{}')",
                    params.agent_id,
                    reused_handle.sub_run_id
                );

                // 向子 session 提交 resume prompt
                if let Some(child_session_id) = reused_handle
                    .child_session_id
                    .as_ref()
                    .or(child.child_session_id.as_ref())
                {
                    let _ = self
                        .session_runtime
                        .submit_prompt(
                            child_session_id,
                            resume_message,
                            self.default_runtime_config(),
                        )
                        .await;
                }

                let child_ref = self.build_child_ref_from_handle(&reused_handle).await;
                return Ok(CollaborationResult {
                    accepted: true,
                    kind: CollaborationResultKind::Sent,
                    agent_ref: Some(self.project_child_ref_status(child_ref).await),
                    delivery_id: None,
                    summary: Some(format!("消息已发送到子 Agent {}", params.agent_id)),
                    cascade: None,
                    closed_root_agent_id: None,
                    failure: None,
                });
            }
        }

        // 非 idle child：直接追加 inbox 信封
        let delivery_id = format!("delivery-{}", uuid::Uuid::new_v4());
        let envelope = astrcode_core::AgentInboxEnvelope {
            delivery_id: delivery_id.clone(),
            from_agent_id: ctx.agent_context().agent_id.clone().unwrap_or_default(),
            to_agent_id: params.agent_id.clone(),
            kind: InboxEnvelopeKind::ParentMessage,
            message: params.message.clone(),
            context: params.context.clone(),
            is_final: false,
            summary: None,
            findings: Vec::new(),
            artifacts: Vec::new(),
        };
        self.append_durable_mailbox_queue(&child, &envelope, ctx)
            .await?;

        self.kernel
            .agent_control()
            .push_inbox(&child.agent_id, envelope)
            .await
            .ok_or_else(|| {
                super::AgentOrchestrationError::NotFound(format!(
                    "agent '{}' inbox not available",
                    params.agent_id
                ))
            })?;

        let child_ref = self.build_child_ref_from_handle(&child).await;
        log::info!(
            "send: message sent to child agent '{}' (subRunId='{}')",
            params.agent_id,
            child.sub_run_id
        );

        Ok(CollaborationResult {
            accepted: true,
            kind: CollaborationResultKind::Sent,
            agent_ref: Some(self.project_child_ref_status(child_ref).await),
            delivery_id: Some(delivery_id),
            summary: Some(format!("消息已发送到子 Agent {}", params.agent_id)),
            cascade: None,
            closed_root_agent_id: None,
            failure: None,
        })
    }

    /// 关闭子 agent 及其子树（close 协作工具的业务逻辑）。
    pub async fn close_child(
        &self,
        params: CloseAgentParams,
        ctx: &astrcode_core::ToolContext,
    ) -> Result<CollaborationResult, super::AgentOrchestrationError> {
        params
            .validate()
            .map_err(super::AgentOrchestrationError::from)?;

        let target = self
            .kernel
            .agent_control()
            .get(&params.agent_id)
            .await
            .ok_or_else(|| {
                super::AgentOrchestrationError::NotFound(format!(
                    "agent '{}' not found",
                    params.agent_id
                ))
            })?;
        self.verify_caller_owns_child(ctx, &target)?;

        // 收集子树用于 durable discard
        let subtree_handles = self
            .kernel
            .agent_control()
            .collect_subtree_handles(&params.agent_id)
            .await;
        let mut discard_targets = Vec::with_capacity(subtree_handles.len() + 1);
        discard_targets.push(target.clone());
        discard_targets.extend(subtree_handles.iter().cloned());

        self.append_durable_mailbox_discard_batch(&discard_targets, ctx)
            .await?;

        // 执行 terminate
        self.kernel
            .agent_control()
            .terminate_subtree(&params.agent_id)
            .await
            .ok_or_else(|| {
                super::AgentOrchestrationError::NotFound(format!(
                    "agent '{}' terminate failed (not found or already finalized)",
                    params.agent_id
                ))
            })?;

        let cancelled = self
            .kernel
            .agent_control()
            .get(&params.agent_id)
            .await
            .ok_or_else(|| {
                super::AgentOrchestrationError::NotFound(format!(
                    "agent '{}' not found",
                    params.agent_id
                ))
            })?;

        let subtree_count = subtree_handles.len();
        let summary = if subtree_count > 0 {
            format!(
                "子 Agent {} 已关闭（含 {} 个后代）",
                params.agent_id, subtree_count
            )
        } else {
            format!("子 Agent {} 已关闭", params.agent_id)
        };

        Ok(CollaborationResult {
            accepted: true,
            kind: CollaborationResultKind::Closed,
            agent_ref: None,
            delivery_id: None,
            summary: Some(summary),
            cascade: Some(true),
            closed_root_agent_id: Some(cancelled.agent_id.clone()),
            failure: None,
        })
    }

    /// 从 SubRunHandle 构造 ChildAgentRef。
    pub(super) async fn build_child_ref_from_handle(&self, handle: &SubRunHandle) -> ChildAgentRef {
        self.build_child_ref_with_lineage(handle, ChildSessionLineageKind::Spawn)
            .await
    }

    async fn build_child_ref_with_lineage(
        &self,
        handle: &SubRunHandle,
        lineage_kind: ChildSessionLineageKind,
    ) -> ChildAgentRef {
        ChildAgentRef {
            agent_id: handle.agent_id.clone(),
            session_id: handle.session_id.clone(),
            sub_run_id: handle.sub_run_id.clone(),
            parent_agent_id: handle.parent_agent_id.clone(),
            parent_sub_run_id: handle.parent_sub_run_id.clone(),
            lineage_kind,
            status: handle.lifecycle,
            open_session_id: handle
                .child_session_id
                .clone()
                .unwrap_or_else(|| handle.session_id.clone()),
        }
    }

    /// 用 live 控制面的最新 lifecycle 投影更新 ChildAgentRef。
    pub(super) async fn project_child_ref_status(
        &self,
        mut child_ref: ChildAgentRef,
    ) -> ChildAgentRef {
        let lifecycle = self
            .kernel
            .agent_control()
            .get_lifecycle(&child_ref.agent_id)
            .await;
        let last_turn_outcome = self
            .kernel
            .agent_control()
            .get_turn_outcome(&child_ref.agent_id)
            .await
            .flatten();
        if let Some(lifecycle) = lifecycle {
            child_ref.status =
                project_collaboration_lifecycle(lifecycle, last_turn_outcome, child_ref.status);
        }
        child_ref
    }
}

/// 将 live 控制平面的 lifecycle + outcome 投影回 ChildAgentRef 的 lifecycle。
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
