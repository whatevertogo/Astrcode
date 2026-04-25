//! agent 协作路由与权限校验。
//!
//! 改为通过 Kernel + SessionRuntime 完成所有操作。

#[path = "routing_collaboration_flow.rs"]
mod collaboration_flow;
use astrcode_core::{
    AgentCollaborationActionKind, AgentCollaborationOutcomeKind, AgentInboxEnvelope,
    AgentLifecycleStatus, ChildAgentRef, ChildSessionNotification, CloseAgentParams,
    CollaborationResult, InboxEnvelopeKind, InputDiscardedPayload, InputQueuedPayload,
    ParentDelivery, ParentDeliveryOrigin, ParentDeliveryPayload, SendAgentParams,
    SendToChildParams, SendToParentParams,
};
use astrcode_host_session::SubRunHandle;
use astrcode_tool_contract::ToolContext;
use collaboration_flow::parent_delivery_label;

use super::{
    AgentOrchestrationError, AgentOrchestrationService, ToolCollaborationContext,
    build_delegation_metadata, subrun_event_context,
};
use crate::governance_surface::{ResumedChildGovernanceInput, collaboration_policy_context};

impl AgentOrchestrationService {
    /// 验证调用者是否为目标子 agent 的直接父级。
    pub(super) fn verify_caller_owns_child(
        &self,
        ctx: &ToolContext,
        child_handle: &SubRunHandle,
    ) -> Result<(), super::AgentOrchestrationError> {
        let caller_agent_id = ctx.agent_context().agent_id.as_deref();
        let child_parent_id = child_handle.parent_agent_id.as_deref();

        match (caller_agent_id, child_parent_id) {
            (Some(caller), Some(parent)) if caller == parent => Ok(()),
            (None, None) => Ok(()),
            _ => Err(super::AgentOrchestrationError::InvalidInput(format!(
                "agent '{}' is not a direct child of caller '{}' (actual parent: {}); \
                 send/observe/close only support direct children",
                child_handle.agent_id,
                caller_agent_id.unwrap_or("<root>"),
                child_parent_id.unwrap_or("<none>")
            ))),
        }
    }

    pub(super) async fn require_direct_child_handle(
        &self,
        agent_id: &str,
        action: AgentCollaborationActionKind,
        ctx: &ToolContext,
        collaboration: &super::ToolCollaborationContext,
    ) -> Result<SubRunHandle, super::AgentOrchestrationError> {
        let child = match self.kernel.get_handle(agent_id).await {
            Some(child) => child,
            None => {
                let error = super::AgentOrchestrationError::NotFound(format!(
                    "agent '{}' not found",
                    agent_id
                ));
                return self
                    .reject_with_fact(
                        collaboration.runtime(),
                        collaboration
                            .fact(action, AgentCollaborationOutcomeKind::Rejected)
                            .reason_code("child_not_found")
                            .summary(error.to_string()),
                        error,
                    )
                    .await;
            },
        };

        if let Err(error) = self.verify_caller_owns_child(ctx, &child) {
            return self
                .reject_with_fact(
                    collaboration.runtime(),
                    collaboration
                        .fact(action, AgentCollaborationOutcomeKind::Rejected)
                        .child(&child)
                        .reason_code("ownership_mismatch")
                        .summary(error.to_string()),
                    error,
                )
                .await;
        }

        Ok(child)
    }

    async fn reject_child_action<T>(
        &self,
        collaboration: &super::ToolCollaborationContext,
        action: AgentCollaborationActionKind,
        child: &SubRunHandle,
        reason_code: &str,
        error: super::AgentOrchestrationError,
    ) -> Result<T, super::AgentOrchestrationError> {
        self.reject_with_fact(
            collaboration.runtime(),
            collaboration
                .fact(action, AgentCollaborationOutcomeKind::Rejected)
                .child(child)
                .reason_code(reason_code)
                .summary(error.to_string()),
            error,
        )
        .await
    }

    /// 统一 send 入口：按参数形状决定上行或下行。
    pub async fn route_send(
        &self,
        params: SendAgentParams,
        ctx: &ToolContext,
    ) -> Result<CollaborationResult, super::AgentOrchestrationError> {
        params
            .validate()
            .map_err(super::AgentOrchestrationError::from)?;

        match params {
            SendAgentParams::ToChild(params) => self.send_to_child(params, ctx).await,
            SendAgentParams::ToParent(params) => self.send_to_parent(params, ctx).await,
        }
    }

    /// 向子 Agent 追加消息（send 协作工具的下行业务逻辑）。
    async fn send_to_child(
        &self,
        params: SendToChildParams,
        ctx: &ToolContext,
    ) -> Result<CollaborationResult, super::AgentOrchestrationError> {
        let collaboration = self.tool_collaboration_context(ctx).await?;

        let child = self
            .require_direct_child_handle(
                &params.agent_id,
                AgentCollaborationActionKind::Send,
                ctx,
                &collaboration,
            )
            .await?;

        let lifecycle = self.kernel.get_lifecycle(&params.agent_id).await;
        if matches!(lifecycle, Some(AgentLifecycleStatus::Terminated)) {
            let error = super::AgentOrchestrationError::InvalidInput(format!(
                "agent '{}' has been terminated and cannot receive new messages",
                params.agent_id
            ));
            return self
                .reject_child_action(
                    &collaboration,
                    AgentCollaborationActionKind::Send,
                    &child,
                    "child_terminated",
                    error,
                )
                .await;
        }

        if let Some(result) = self
            .resume_idle_child_if_needed(&child, &params, ctx, &collaboration, lifecycle)
            .await?
        {
            return Ok(result);
        }

        self.queue_message_for_active_child(&child, &params, ctx, &collaboration)
            .await
    }

    /// 子代理上行消息投递（send to parent）。
    ///
    /// 完整校验链：
    /// 1. 验证调用者有 child agent context
    /// 2. 验证调用者的 handle 存在
    /// 3. 验证存在 direct parent agent（root agent 不能上行）
    /// 4. 验证 parent handle 存在且未终止
    /// 5. 验证有 source turn id
    ///
    /// 投递后触发父级 reactivation，让父级消费这条 delivery。
    async fn send_to_parent(
        &self,
        params: SendToParentParams,
        ctx: &ToolContext,
    ) -> Result<CollaborationResult, super::AgentOrchestrationError> {
        let fallback_collaboration = self.tool_collaboration_context(ctx).await?;
        let Some(child_agent_id) = ctx.agent_context().agent_id.as_deref() else {
            let error = super::AgentOrchestrationError::InvalidInput(
                "upstream send requires a child agent execution context".to_string(),
            );
            log::warn!(
                "upstream send rejected before routing: reason='missing_child_context', \
                 session='{}', turn='{}'",
                ctx.session_id(),
                ctx.turn_id().unwrap_or("unknown-turn")
            );
            return self
                .reject_with_fact(
                    fallback_collaboration.runtime(),
                    fallback_collaboration
                        .fact(
                            AgentCollaborationActionKind::Delivery,
                            AgentCollaborationOutcomeKind::Rejected,
                        )
                        .reason_code("missing_child_context")
                        .summary(error.to_string()),
                    error,
                )
                .await;
        };

        let child = match self.kernel.get_handle(child_agent_id).await {
            Some(child) => child,
            None => {
                let error = super::AgentOrchestrationError::NotFound(format!(
                    "agent '{}' not found",
                    child_agent_id
                ));
                log::warn!(
                    "upstream send rejected before routing: reason='sender_handle_missing', \
                     childAgent='{}', session='{}', turn='{}'",
                    child_agent_id,
                    ctx.session_id(),
                    ctx.turn_id().unwrap_or("unknown-turn")
                );
                return self
                    .reject_with_fact(
                        fallback_collaboration.runtime(),
                        fallback_collaboration
                            .fact(
                                AgentCollaborationActionKind::Delivery,
                                AgentCollaborationOutcomeKind::Rejected,
                            )
                            .reason_code("sender_handle_missing")
                            .summary(error.to_string()),
                        error,
                    )
                    .await;
            },
        };

        let collaboration = self.upstream_collaboration_context(&child, ctx).await?;
        let Some(parent_agent_id) = child.parent_agent_id.as_ref() else {
            let error = super::AgentOrchestrationError::InvalidInput(
                "root agent cannot use upstream send because it has no direct parent".to_string(),
            );
            log::warn!(
                "upstream send rejected before routing: reason='missing_direct_parent', \
                 childAgent='{}', parentSession='{}', parentTurn='{}'",
                child.agent_id,
                collaboration.session_id(),
                collaboration.turn_id()
            );
            return self
                .reject_with_fact(
                    collaboration.runtime(),
                    collaboration
                        .fact(
                            AgentCollaborationActionKind::Delivery,
                            AgentCollaborationOutcomeKind::Rejected,
                        )
                        .child(&child)
                        .reason_code("missing_direct_parent")
                        .summary(error.to_string()),
                    error,
                )
                .await;
        };

        let Some(parent_handle) = self.kernel.get_handle(parent_agent_id).await else {
            let error = super::AgentOrchestrationError::NotFound(format!(
                "direct parent agent '{}' not found",
                parent_agent_id
            ));
            log::warn!(
                "upstream send rejected before routing: reason='parent_not_found', \
                 childAgent='{}', parentAgent='{}', parentSession='{}', parentTurn='{}'",
                child.agent_id,
                parent_agent_id,
                collaboration.session_id(),
                collaboration.turn_id()
            );
            return self
                .reject_with_fact(
                    collaboration.runtime(),
                    collaboration
                        .fact(
                            AgentCollaborationActionKind::Delivery,
                            AgentCollaborationOutcomeKind::Rejected,
                        )
                        .child(&child)
                        .reason_code("parent_not_found")
                        .summary(error.to_string()),
                    error,
                )
                .await;
        };

        let parent_lifecycle = self.kernel.get_lifecycle(parent_agent_id).await;
        if matches!(parent_lifecycle, Some(AgentLifecycleStatus::Terminated))
            || matches!(parent_handle.lifecycle, AgentLifecycleStatus::Terminated)
        {
            let error = super::AgentOrchestrationError::InvalidInput(format!(
                "direct parent agent '{}' has been terminated and cannot receive upstream send",
                parent_agent_id
            ));
            log::warn!(
                "upstream send rejected before routing: reason='parent_terminated', \
                 childAgent='{}', parentAgent='{}', parentSession='{}', parentTurn='{}'",
                child.agent_id,
                parent_agent_id,
                collaboration.session_id(),
                collaboration.turn_id()
            );
            return self
                .reject_with_fact(
                    collaboration.runtime(),
                    collaboration
                        .fact(
                            AgentCollaborationActionKind::Delivery,
                            AgentCollaborationOutcomeKind::Rejected,
                        )
                        .child(&child)
                        .reason_code("parent_terminated")
                        .summary(error.to_string()),
                    error,
                )
                .await;
        }

        let Some(source_turn_id) = ctx.turn_id().map(ToString::to_string) else {
            let error = super::AgentOrchestrationError::InvalidInput(
                "upstream send requires the current child work turn id".to_string(),
            );
            log::warn!(
                "upstream send rejected before routing: reason='missing_source_turn', \
                 childAgent='{}', parentSession='{}', parentTurn='{}'",
                child.agent_id,
                collaboration.session_id(),
                collaboration.turn_id()
            );
            return self
                .reject_with_fact(
                    collaboration.runtime(),
                    collaboration
                        .fact(
                            AgentCollaborationActionKind::Delivery,
                            AgentCollaborationOutcomeKind::Rejected,
                        )
                        .child(&child)
                        .reason_code("missing_source_turn")
                        .summary(error.to_string()),
                    error,
                )
                .await;
        };

        let payload = params.payload;
        let notification = self
            .build_explicit_parent_delivery_notification(&child, &payload, ctx, &source_turn_id)
            .await;
        self.append_child_session_notification(
            &child,
            collaboration.session_id(),
            collaboration.turn_id(),
            &notification,
        )
        .await?;
        self.record_fact_best_effort(
            collaboration.runtime(),
            collaboration
                .fact(
                    AgentCollaborationActionKind::Delivery,
                    AgentCollaborationOutcomeKind::Delivered,
                )
                .child(&child)
                .delivery_id(notification.notification_id.clone())
                .summary(payload.message().trim().to_string()),
        )
        .await;
        log::info!(
            "explicit upstream send delivered: childAgent='{}', parentAgent='{}', \
             parentSession='{}', parentTurn='{}', deliveryId='{}', sourceTurnId='{}'",
            child.agent_id,
            parent_agent_id,
            collaboration.session_id(),
            collaboration.turn_id(),
            notification.notification_id,
            source_turn_id
        );
        self.reactivate_parent_agent_if_idle(
            collaboration.session_id(),
            collaboration.turn_id(),
            &notification,
        )
        .await;

        let child_ref = self.build_child_ref_from_handle(&child).await;
        Ok(CollaborationResult::Sent {
            continuation: Some(astrcode_core::ExecutionContinuation::child_agent(
                self.project_child_ref_status(child_ref).await,
            )),
            delivery_id: Some(notification.notification_id.clone()),
            summary: Some(format!(
                "已向 direct parent 发送 {} 消息。",
                parent_delivery_label(&payload)
            )),
            delegation: child.delegation.clone(),
        })
    }

    async fn upstream_collaboration_context(
        &self,
        child: &SubRunHandle,
        ctx: &ToolContext,
    ) -> Result<super::ToolCollaborationContext, super::AgentOrchestrationError> {
        let parent_turn_id = match ctx.agent_context().parent_turn_id.clone() {
            Some(id) => id,
            None => {
                log::warn!(
                    "agent_tool_routing: child {} missing parent_turn_id in tool context, falling \
                     back to handle value (may be stale)",
                    child.agent_id
                );
                child.parent_turn_id.clone()
            },
        };

        let runtime = self
            .resolve_runtime_config_for_session(child.session_id.as_str())
            .await?;
        let mode_id = self
            .session_runtime
            .session_mode_state(child.session_id.as_str())
            .await
            .map_err(super::AgentOrchestrationError::from)?
            .current_mode_id;

        Ok(super::ToolCollaborationContext::new(
            super::ToolCollaborationContextInput {
                runtime: runtime.clone(),
                session_id: child.session_id.to_string(),
                turn_id: parent_turn_id.to_string(),
                parent_agent_id: child.parent_agent_id.clone().map(Into::into),
                source_tool_call_id: ctx.tool_call_id().map(ToString::to_string),
                policy: collaboration_policy_context(&runtime),
                governance_revision: super::AGENT_COLLABORATION_POLICY_REVISION.to_string(),
                mode_id,
            },
        ))
    }
}

#[cfg(test)]
mod tests;
