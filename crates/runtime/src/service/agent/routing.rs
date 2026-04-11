//! agent 协作路由与权限校验。

use astrcode_core::{
    AgentLifecycleStatus, ChildAgentRef, ChildSessionLineageKind, CloseAgentParams,
    CollaborationResult, CollaborationResultKind, InboxEnvelopeKind, SendAgentParams, SubRunHandle,
};

use super::{AgentServiceHandle, mailbox::compose_reusable_child_message};
use crate::service::{ServiceError, ServiceResult};

impl AgentServiceHandle {
    /// 验证调用者是否为目标子 agent 的直接父级。
    pub(super) fn verify_caller_owns_child(
        &self,
        ctx: &astrcode_core::ToolContext,
        child_handle: &SubRunHandle,
    ) -> ServiceResult<()> {
        let caller_agent_id = ctx.agent_context().agent_id.as_deref();
        let child_parent_id = child_handle.parent_agent_id.as_deref();

        match (caller_agent_id, child_parent_id) {
            (Some(caller), Some(parent)) if caller == parent => Ok(()),
            (None, None) => Ok(()),
            _ => Err(ServiceError::InvalidInput(format!(
                "caller '{}' does not own agent '{}' (parent: {})",
                caller_agent_id.unwrap_or("<root>"),
                child_handle.agent_id,
                child_parent_id.unwrap_or("<none>")
            ))),
        }
    }

    /// 向子 Agent 追加消息。
    pub async fn send_to_child(
        &self,
        params: SendAgentParams,
        ctx: &astrcode_core::ToolContext,
    ) -> ServiceResult<CollaborationResult> {
        params.validate().map_err(ServiceError::from)?;

        let child = self
            .runtime
            .agent_control
            .get(&params.agent_id)
            .await
            .ok_or_else(|| {
                ServiceError::NotFound(format!("agent '{}' not found", params.agent_id))
            })?;

        self.verify_caller_owns_child(ctx, &child)?;

        let lifecycle = self
            .runtime
            .agent_control
            .get_lifecycle(&params.agent_id)
            .await;
        if matches!(lifecycle, Some(AgentLifecycleStatus::Terminated)) {
            return Err(ServiceError::InvalidInput(format!(
                "agent '{}' has been terminated and cannot receive new messages",
                params.agent_id
            )));
        }

        if matches!(lifecycle, Some(AgentLifecycleStatus::Idle)) && !child.lifecycle.occupies_slot()
        {
            let pending = self
                .runtime
                .agent_control
                .drain_inbox(&child.agent_id)
                .await
                .unwrap_or_default();
            let resume_message = compose_reusable_child_message(&pending, &params);
            let (reused_handle, _) = self
                .runtime
                .execution()
                .resume_child_session(&params.agent_id, Some(resume_message), ctx)
                .await?;
            let child_ref = self.build_child_ref_from_handle(&reused_handle).await;

            log::info!(
                "send: reusable child agent '{}' restarted with a new turn (subRunId='{}')",
                params.agent_id,
                reused_handle.sub_run_id
            );

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

        self.runtime
            .agent_control
            .push_inbox(&child.agent_id, envelope)
            .await
            .ok_or_else(|| {
                ServiceError::NotFound(format!("agent '{}' inbox not available", params.agent_id))
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

    pub async fn close_child(
        &self,
        params: CloseAgentParams,
        ctx: &astrcode_core::ToolContext,
    ) -> ServiceResult<CollaborationResult> {
        params.validate().map_err(ServiceError::from)?;

        let target = self
            .runtime
            .agent_control
            .get(&params.agent_id)
            .await
            .ok_or_else(|| {
                ServiceError::NotFound(format!("agent '{}' not found", params.agent_id))
            })?;
        self.verify_caller_owns_child(ctx, &target)?;

        let subtree_handles = self
            .runtime
            .agent_control
            .collect_subtree_handles(&params.agent_id)
            .await;
        let mut discard_targets = Vec::with_capacity(subtree_handles.len() + 1);
        discard_targets.push(target.clone());
        discard_targets.extend(subtree_handles.iter().cloned());

        self.append_durable_mailbox_discard_batch(&discard_targets, ctx)
            .await?;

        self.runtime
            .agent_control
            .terminate_subtree(&params.agent_id)
            .await
            .ok_or_else(|| {
                ServiceError::NotFound(format!(
                    "agent '{}' terminate failed (not found or already finalized)",
                    params.agent_id
                ))
            })?;

        let cancelled = self
            .runtime
            .agent_control
            .get(&params.agent_id)
            .await
            .ok_or_else(|| {
                ServiceError::NotFound(format!("agent '{}' not found", params.agent_id))
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

    pub(super) async fn build_child_ref_from_handle(&self, handle: &SubRunHandle) -> ChildAgentRef {
        self.build_child_ref_with_lineage(handle, ChildSessionLineageKind::Spawn)
            .await
    }

    pub(super) async fn build_child_ref_with_lineage(
        &self,
        handle: &SubRunHandle,
        lineage_kind: ChildSessionLineageKind,
    ) -> ChildAgentRef {
        ChildAgentRef {
            agent_id: handle.agent_id.clone(),
            session_id: handle.session_id.clone(),
            sub_run_id: handle.sub_run_id.clone(),
            parent_agent_id: handle.parent_agent_id.clone(),
            lineage_kind,
            status: handle.lifecycle,
            open_session_id: handle
                .child_session_id
                .clone()
                .unwrap_or_else(|| handle.session_id.clone()),
        }
    }

    pub(super) async fn project_child_ref_status(
        &self,
        mut child_ref: ChildAgentRef,
    ) -> ChildAgentRef {
        let lifecycle = self
            .runtime
            .agent_control
            .get_lifecycle(&child_ref.agent_id)
            .await;
        let last_turn_outcome = self
            .runtime
            .agent_control
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
///
/// 如果 live 控制平面没有 outcome 信息（如刚 spawn 还没跑完），保留 fallback。
pub(super) fn project_collaboration_lifecycle(
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
