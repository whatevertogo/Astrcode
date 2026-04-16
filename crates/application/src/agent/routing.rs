//! agent 协作路由与权限校验。
//!
//! 从旧 runtime/service/agent/routing.rs 迁入，去掉对 RuntimeService 的依赖，
//! 改为通过 Kernel + SessionRuntime 完成所有操作。

use astrcode_core::{
    AgentCollaborationActionKind, AgentCollaborationOutcomeKind, AgentInboxEnvelope,
    AgentLifecycleStatus, ChildAgentRef, ChildSessionNotification, ChildSessionNotificationKind,
    CloseAgentParams, CollaborationResult, InboxEnvelopeKind, MailboxDiscardedPayload,
    MailboxQueuedPayload, ParentDelivery, ParentDeliveryOrigin, ParentDeliveryPayload,
    ParentDeliveryTerminalSemantics, SendAgentParams, SendToChildParams, SendToParentParams,
    SubRunHandle,
};

use super::{
    AgentOrchestrationService, build_delegation_metadata, build_resumed_child_contract,
    subrun_event_context,
};

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
        ctx: &astrcode_core::ToolContext,
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
        ctx: &astrcode_core::ToolContext,
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
        ctx: &astrcode_core::ToolContext,
    ) -> Result<CollaborationResult, super::AgentOrchestrationError> {
        let collaboration = self.tool_collaboration_context(ctx)?;

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

    async fn send_to_parent(
        &self,
        params: SendToParentParams,
        ctx: &astrcode_core::ToolContext,
    ) -> Result<CollaborationResult, super::AgentOrchestrationError> {
        let fallback_collaboration = self.tool_collaboration_context(ctx)?;
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
            agent_ref: Some(self.project_child_ref_status(child_ref).await),
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
        ctx: &astrcode_core::ToolContext,
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

        Ok(super::ToolCollaborationContext::new(
            self.resolve_runtime_config_for_session(child.session_id.as_str())
                .await?,
            child.session_id.to_string(),
            parent_turn_id.to_string(),
            child.parent_agent_id.clone().map(Into::into),
            ctx.tool_call_id().map(ToString::to_string),
        ))
    }

    async fn build_explicit_parent_delivery_notification(
        &self,
        child: &SubRunHandle,
        payload: &ParentDeliveryPayload,
        ctx: &astrcode_core::ToolContext,
        source_turn_id: &str,
    ) -> ChildSessionNotification {
        let status = self
            .kernel
            .get_lifecycle(&child.agent_id)
            .await
            .unwrap_or(child.lifecycle);
        let notification_id = explicit_parent_delivery_id(
            &child.sub_run_id,
            source_turn_id,
            ctx.tool_call_id().map(ToString::to_string).as_deref(),
            payload,
        );

        ChildSessionNotification {
            notification_id: notification_id.clone().into(),
            child_ref: child.child_ref_with_status(status),
            kind: parent_delivery_notification_kind(payload),
            source_tool_call_id: ctx.tool_call_id().map(ToString::to_string).map(Into::into),
            delivery: Some(ParentDelivery {
                idempotency_key: notification_id,
                origin: ParentDeliveryOrigin::Explicit,
                terminal_semantics: parent_delivery_terminal_semantics(payload),
                source_turn_id: Some(source_turn_id.to_string()),
                payload: payload.clone(),
            }),
        }
    }

    /// 关闭子 agent 及其子树（close 协作工具的业务逻辑）。
    pub async fn close_child(
        &self,
        params: CloseAgentParams,
        ctx: &astrcode_core::ToolContext,
    ) -> Result<CollaborationResult, super::AgentOrchestrationError> {
        let collaboration = self.tool_collaboration_context(ctx)?;
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

        // 收集子树用于 durable discard
        let subtree_handles = self.kernel.collect_subtree_handles(&params.agent_id).await;
        let mut discard_targets = Vec::with_capacity(subtree_handles.len() + 1);
        discard_targets.push(target.clone());
        discard_targets.extend(subtree_handles.iter().cloned());

        self.append_durable_mailbox_discard_batch(&discard_targets, ctx)
            .await?;

        // 执行 terminate
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
            summary: Some(summary),
            cascade: true,
            closed_root_agent_id: cancelled.agent_id.clone(),
        })
    }

    /// 从 SubRunHandle 构造 ChildAgentRef。
    pub(super) async fn build_child_ref_from_handle(&self, handle: &SubRunHandle) -> ChildAgentRef {
        handle.child_ref()
    }

    /// 用 live 控制面的最新 lifecycle 投影更新 ChildAgentRef。
    pub(super) async fn project_child_ref_status(
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

    /// resume 失败时恢复之前 drain 出的 inbox 信封。
    /// 必须在 resume 前先 drain（否则无法取到 pending 消息来组合 resume prompt），
    /// 但如果 resume 本身失败，必须把信封放回去，避免消息丢失。
    async fn restore_pending_inbox(&self, agent_id: &str, pending: Vec<AgentInboxEnvelope>) {
        for envelope in pending {
            if self.kernel.deliver(agent_id, envelope).await.is_none() {
                log::warn!(
                    "failed to restore drained inbox after resume error: agent='{}'",
                    agent_id
                );
                break;
            }
        }
    }

    async fn restore_pending_inbox_and_fail<T>(
        &self,
        agent_id: &str,
        pending: Vec<AgentInboxEnvelope>,
        message: String,
    ) -> Result<T, super::AgentOrchestrationError> {
        self.restore_pending_inbox(agent_id, pending).await;
        Err(super::AgentOrchestrationError::Internal(message))
    }

    async fn resume_idle_child_if_needed(
        &self,
        child: &SubRunHandle,
        params: &SendToChildParams,
        ctx: &astrcode_core::ToolContext,
        collaboration: &super::ToolCollaborationContext,
        lifecycle: Option<AgentLifecycleStatus>,
    ) -> Result<Option<CollaborationResult>, super::AgentOrchestrationError> {
        if !matches!(lifecycle, Some(AgentLifecycleStatus::Idle)) || child.lifecycle.occupies_slot()
        {
            return Ok(None);
        }

        let pending = self
            .kernel
            .drain_inbox(&child.agent_id)
            .await
            .unwrap_or_default();
        let resume_message = compose_reusable_child_message(&pending, params);
        let current_parent_turn_id = ctx.turn_id().unwrap_or(&child.parent_turn_id).to_string();

        let Some(reused_handle) = self
            .kernel
            .resume(&params.agent_id, &current_parent_turn_id)
            .await
        else {
            self.restore_pending_inbox(&child.agent_id, pending).await;
            return Ok(None);
        };

        log::info!(
            "send: reusable child agent '{}' restarted with new turn (subRunId='{}')",
            params.agent_id,
            reused_handle.sub_run_id
        );

        let Some(child_session_id) = reused_handle
            .child_session_id
            .as_ref()
            .or(child.child_session_id.as_ref())
        else {
            return self
                .restore_pending_inbox_and_fail(
                    &child.agent_id,
                    pending,
                    format!(
                        "agent '{}' resume failed: missing child session id",
                        params.agent_id
                    ),
                )
                .await;
        };

        let allowed_tools =
            super::effective_tool_names_for_handle(&reused_handle, &self.kernel.gateway());
        let scoped_router = match self
            .kernel
            .gateway()
            .capabilities()
            .subset_for_tools_checked(&allowed_tools)
        {
            Ok(router) => router,
            Err(error) => {
                return self
                    .restore_pending_inbox_and_fail(
                        &child.agent_id,
                        pending,
                        format!(
                            "agent '{}' resume capability resolution failed: {error}",
                            params.agent_id
                        ),
                    )
                    .await;
            },
        };

        let fallback_delegation = build_delegation_metadata(
            "",
            params.message.as_str(),
            &reused_handle.resolved_limits,
            false,
        );
        let resume_delegation = reused_handle
            .delegation
            .clone()
            .unwrap_or(fallback_delegation);

        let accepted = match self
            .session_runtime
            .submit_prompt_for_agent_with_submission(
                child_session_id,
                resume_message,
                self.resolve_runtime_config_for_session(child_session_id)
                    .await?,
                astrcode_session_runtime::AgentPromptSubmission {
                    agent: astrcode_core::AgentEventContext::from(&reused_handle),
                    capability_router: Some(scoped_router),
                    prompt_declarations: vec![build_resumed_child_contract(
                        &resume_delegation,
                        params.message.as_str(),
                        params.context.as_deref(),
                    )],
                    resolved_limits: Some(reused_handle.resolved_limits.clone()),
                    resolved_overrides: None,
                    injected_messages: Vec::new(),
                    source_tool_call_id: ctx.tool_call_id().map(ToString::to_string),
                },
            )
            .await
        {
            Ok(accepted) => accepted,
            Err(error) => {
                return self
                    .restore_pending_inbox_and_fail(
                        &child.agent_id,
                        pending,
                        format!("agent '{}' resume submit failed: {error}", params.agent_id),
                    )
                    .await;
            },
        };
        self.spawn_child_turn_terminal_watcher(
            reused_handle.clone(),
            accepted.session_id.to_string(),
            accepted.turn_id.to_string(),
            ctx.session_id().to_string(),
            current_parent_turn_id,
            ctx.tool_call_id().map(ToString::to_string),
        );

        let child_ref = self.build_child_ref_from_handle(&reused_handle).await;
        self.record_fact_best_effort(
            collaboration.runtime(),
            collaboration
                .fact(
                    AgentCollaborationActionKind::Send,
                    AgentCollaborationOutcomeKind::Reused,
                )
                .child(&reused_handle)
                .summary("idle child resumed"),
        )
        .await;
        Ok(Some(CollaborationResult::Sent {
            agent_ref: Some(self.project_child_ref_status(child_ref).await),
            delivery_id: None,
            summary: Some(format!(
                "子 Agent {} 已恢复，并开始处理新的具体指令。",
                params.agent_id
            )),
            delegation: reused_handle.delegation.clone(),
        }))
    }

    async fn queue_message_for_active_child(
        &self,
        child: &SubRunHandle,
        params: &SendToChildParams,
        ctx: &astrcode_core::ToolContext,
        collaboration: &super::ToolCollaborationContext,
    ) -> Result<CollaborationResult, super::AgentOrchestrationError> {
        let delivery_id = format!("delivery-{}", uuid::Uuid::new_v4());
        let envelope = astrcode_core::AgentInboxEnvelope {
            delivery_id: delivery_id.clone(),
            from_agent_id: ctx
                .agent_context()
                .agent_id
                .clone()
                .unwrap_or_default()
                .to_string(),
            to_agent_id: params.agent_id.to_string(),
            kind: InboxEnvelopeKind::ParentMessage,
            message: params.message.clone(),
            context: params.context.clone(),
            is_final: false,
            summary: None,
            findings: Vec::new(),
            artifacts: Vec::new(),
        };
        self.append_durable_mailbox_queue(child, &envelope, ctx)
            .await?;

        self.kernel
            .deliver(&child.agent_id, envelope)
            .await
            .ok_or_else(|| {
                super::AgentOrchestrationError::NotFound(format!(
                    "agent '{}' inbox not available",
                    params.agent_id
                ))
            })?;

        let child_ref = self.build_child_ref_from_handle(child).await;
        log::info!(
            "send: message sent to child agent '{}' (subRunId='{}')",
            params.agent_id,
            child.sub_run_id
        );
        self.record_fact_best_effort(
            collaboration.runtime(),
            collaboration
                .fact(
                    AgentCollaborationActionKind::Send,
                    AgentCollaborationOutcomeKind::Queued,
                )
                .child(child)
                .delivery_id(delivery_id.clone())
                .summary("message queued for running child"),
        )
        .await;

        Ok(CollaborationResult::Sent {
            agent_ref: Some(self.project_child_ref_status(child_ref).await),
            delivery_id: Some(delivery_id.into()),
            summary: Some(format!(
                "子 Agent {} 正在运行；消息已进入 mailbox 排队，待当前工作完成后处理。",
                params.agent_id
            )),
            delegation: child.delegation.clone(),
        })
    }
}

/// 将待处理的 inbox 信封与新的 send 输入拼接为 resume 消息。
fn compose_reusable_child_message(
    pending: &[astrcode_core::AgentInboxEnvelope],
    params: &astrcode_core::SendToChildParams,
) -> String {
    let mut parts = pending
        .iter()
        .filter(|envelope| {
            matches!(
                envelope.kind,
                astrcode_core::InboxEnvelopeKind::ParentMessage
            )
        })
        .map(render_parent_message_envelope)
        .collect::<Vec<_>>();
    parts.push(render_parent_message_input(
        params.message.as_str(),
        params.context.as_deref(),
    ));

    if parts.len() == 1 {
        return parts.pop().unwrap_or_default();
    }

    let enumerated = parts
        .into_iter()
        .enumerate()
        .map(|(index, part)| format!("{}. {}", index + 1, part))
        .collect::<Vec<_>>()
        .join("\n\n");
    format!("请按顺序处理以下追加要求：\n\n{enumerated}")
}

fn parent_delivery_terminal_semantics(
    payload: &ParentDeliveryPayload,
) -> ParentDeliveryTerminalSemantics {
    match payload {
        ParentDeliveryPayload::Progress(_) => ParentDeliveryTerminalSemantics::NonTerminal,
        ParentDeliveryPayload::Completed(_)
        | ParentDeliveryPayload::Failed(_)
        | ParentDeliveryPayload::CloseRequest(_) => ParentDeliveryTerminalSemantics::Terminal,
    }
}

fn parent_delivery_notification_kind(
    payload: &ParentDeliveryPayload,
) -> ChildSessionNotificationKind {
    match payload {
        ParentDeliveryPayload::Progress(_) => ChildSessionNotificationKind::ProgressSummary,
        ParentDeliveryPayload::Completed(_) => ChildSessionNotificationKind::Delivered,
        ParentDeliveryPayload::Failed(_) => ChildSessionNotificationKind::Failed,
        ParentDeliveryPayload::CloseRequest(_) => ChildSessionNotificationKind::Closed,
    }
}

fn parent_delivery_label(payload: &ParentDeliveryPayload) -> &'static str {
    match payload {
        ParentDeliveryPayload::Progress(_) => "progress",
        ParentDeliveryPayload::Completed(_) => "completed",
        ParentDeliveryPayload::Failed(_) => "failed",
        ParentDeliveryPayload::CloseRequest(_) => "close_request",
    }
}

fn explicit_parent_delivery_id(
    sub_run_id: &str,
    source_turn_id: &str,
    source_tool_call_id: Option<&str>,
    payload: &ParentDeliveryPayload,
) -> String {
    let tool_call_id = source_tool_call_id.unwrap_or("tool-call-missing");
    format!(
        "child-send:{sub_run_id}:{source_turn_id}:{tool_call_id}:{}",
        parent_delivery_label(payload)
    )
}

fn render_parent_message_envelope(envelope: &astrcode_core::AgentInboxEnvelope) -> String {
    render_parent_message_input(envelope.message.as_str(), envelope.context.as_deref())
}

fn render_parent_message_input(message: &str, context: Option<&str>) -> String {
    match context {
        Some(context) if !context.trim().is_empty() => {
            format!("{message}\n\n补充上下文：{context}")
        },
        _ => message.to_string(),
    }
}

impl AgentOrchestrationService {
    pub(super) async fn append_durable_mailbox_queue(
        &self,
        child: &SubRunHandle,
        envelope: &AgentInboxEnvelope,
        ctx: &astrcode_core::ToolContext,
    ) -> astrcode_core::Result<()> {
        let target_session_id = child
            .child_session_id
            .clone()
            .unwrap_or_else(|| child.session_id.clone())
            .to_string();

        let sender_agent_id = ctx.agent_context().agent_id.clone().unwrap_or_default();
        let sender_lifecycle_status = if sender_agent_id.is_empty() {
            AgentLifecycleStatus::Running
        } else {
            self.kernel
                .get_lifecycle(&sender_agent_id)
                .await
                .unwrap_or(AgentLifecycleStatus::Running)
        };
        let sender_last_turn_outcome = if sender_agent_id.is_empty() {
            None
        } else {
            self.kernel.get_turn_outcome(&sender_agent_id).await
        };
        let sender_open_session_id = ctx
            .agent_context()
            .child_session_id
            .clone()
            .unwrap_or_else(|| ctx.session_id().to_string().into());

        let payload = MailboxQueuedPayload {
            envelope: astrcode_core::AgentMailboxEnvelope {
                delivery_id: envelope.delivery_id.clone().into(),
                from_agent_id: envelope.from_agent_id.clone(),
                to_agent_id: envelope.to_agent_id.clone(),
                message: render_parent_message_input(
                    &envelope.message,
                    envelope.context.as_deref(),
                ),
                queued_at: chrono::Utc::now(),
                sender_lifecycle_status,
                sender_last_turn_outcome,
                sender_open_session_id: sender_open_session_id.to_string(),
            },
        };

        self.session_runtime
            .append_agent_mailbox_queued(
                &target_session_id,
                ctx.turn_id().unwrap_or(child.parent_turn_id.as_str()),
                subrun_event_context(child),
                payload,
            )
            .await?;
        Ok(())
    }

    pub(super) async fn append_durable_mailbox_discard_batch(
        &self,
        handles: &[SubRunHandle],
        ctx: &astrcode_core::ToolContext,
    ) -> astrcode_core::Result<()> {
        for handle in handles {
            self.append_durable_mailbox_discard(handle, ctx).await?;
        }
        Ok(())
    }

    async fn append_durable_mailbox_discard(
        &self,
        handle: &SubRunHandle,
        ctx: &astrcode_core::ToolContext,
    ) -> astrcode_core::Result<()> {
        let target_session_id = handle
            .child_session_id
            .clone()
            .unwrap_or_else(|| handle.session_id.clone());
        let pending_delivery_ids = self
            .session_runtime
            .pending_delivery_ids_for_agent(&target_session_id, &handle.agent_id)
            .await?;
        if pending_delivery_ids.is_empty() {
            return Ok(());
        }

        self.session_runtime
            .append_agent_mailbox_discarded(
                &target_session_id,
                ctx.turn_id().unwrap_or(&handle.parent_turn_id),
                astrcode_core::AgentEventContext::default(),
                MailboxDiscardedPayload {
                    target_agent_id: handle.agent_id.to_string(),
                    delivery_ids: pending_delivery_ids.into_iter().map(Into::into).collect(),
                },
            )
            .await?;
        Ok(())
    }
}

/// 将 live 控制面的 lifecycle + outcome 投影回 `ChildAgentRef` 的 lifecycle。
///
/// `Idle` + `None` outcome 的含义是：agent 已空闲但还没有完成过一轮 turn，
/// 此时保留调用方传入的 fallback 状态（通常是 handle 上的旧 lifecycle）。
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

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use astrcode_core::{
        AgentCollaborationActionKind, AgentCollaborationOutcomeKind, CancelToken, CloseAgentParams,
        CompletedParentDeliveryPayload, ObserveParams, ParentDeliveryPayload, SendAgentParams,
        SendToChildParams, SendToParentParams, SessionId, SpawnAgentParams, StorageEventPayload,
        ToolContext,
        agent::executor::{CollaborationExecutor, SubAgentExecutor},
    };
    use tokio::time::sleep;

    use super::super::{root_execution_event_context, subrun_event_context};
    use crate::{
        AgentKernelPort, AppKernelPort,
        agent::test_support::{TestLlmBehavior, build_agent_test_harness},
        lifecycle::governance::ObservabilitySnapshotProvider,
    };

    async fn spawn_direct_child(
        harness: &crate::agent::test_support::AgentTestHarness,
        parent_session_id: &str,
        working_dir: &std::path::Path,
    ) -> (String, String) {
        harness
            .kernel
            .agent_control()
            .register_root_agent(
                "root-agent".to_string(),
                parent_session_id.to_string(),
                "root-profile".to_string(),
            )
            .await
            .expect("root agent should be registered");
        let parent_ctx = ToolContext::new(
            parent_session_id.to_string().into(),
            working_dir.to_path_buf(),
            CancelToken::new(),
        )
        .with_turn_id("turn-parent")
        .with_agent_context(root_execution_event_context("root-agent", "root-profile"));

        let launched = harness
            .service
            .launch(
                SpawnAgentParams {
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
                .get_lifecycle(&child_agent_id)
                .await
                .is_some_and(|lifecycle| lifecycle == astrcode_core::AgentLifecycleStatus::Idle)
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
        (child_agent_id, parent_ctx.session_id().to_string())
    }

    #[tokio::test]
    async fn collaboration_calls_reject_non_direct_child() {
        let harness = build_agent_test_harness(TestLlmBehavior::Succeed {
            content: "完成。".to_string(),
        })
        .expect("test harness should build");
        let project = tempfile::tempdir().expect("tempdir should be created");

        let parent_a = harness
            .session_runtime
            .create_session(project.path().display().to_string())
            .await
            .expect("parent session A should be created");
        let (child_agent_id, _) =
            spawn_direct_child(&harness, &parent_a.session_id, project.path()).await;

        let parent_b = harness
            .session_runtime
            .create_session(project.path().display().to_string())
            .await
            .expect("parent session B should be created");
        harness
            .kernel
            .agent_control()
            .register_root_agent(
                "other-root".to_string(),
                parent_b.session_id.clone(),
                "root-profile".to_string(),
            )
            .await
            .expect("other root agent should be registered");
        let other_ctx = ToolContext::new(
            parent_b.session_id.clone().into(),
            project.path().to_path_buf(),
            CancelToken::new(),
        )
        .with_turn_id("turn-other")
        .with_agent_context(root_execution_event_context("other-root", "root-profile"));

        let send_error = harness
            .service
            .send(
                SendAgentParams::ToChild(SendToChildParams {
                    agent_id: child_agent_id.clone().into(),
                    message: "继续".to_string(),
                    context: None,
                }),
                &other_ctx,
            )
            .await
            .expect_err("send should reject non-direct child");
        assert!(send_error.to_string().contains("direct child"));

        let observe_error = harness
            .service
            .observe(
                ObserveParams {
                    agent_id: child_agent_id.clone(),
                },
                &other_ctx,
            )
            .await
            .expect_err("observe should reject non-direct child");
        assert!(observe_error.to_string().contains("direct child"));

        let close_error = harness
            .service
            .close(
                CloseAgentParams {
                    agent_id: child_agent_id.into(),
                },
                &other_ctx,
            )
            .await
            .expect_err("close should reject non-direct child");
        assert!(close_error.to_string().contains("direct child"));

        let parent_b_events = harness
            .session_runtime
            .replay_stored_events(&SessionId::from(parent_b.session_id.clone()))
            .await
            .expect("other parent events should replay");
        assert!(parent_b_events.iter().any(|stored| matches!(
            &stored.event.payload,
            StorageEventPayload::AgentCollaborationFact { fact, .. }
                if fact.action == AgentCollaborationActionKind::Send
                    && fact.outcome == AgentCollaborationOutcomeKind::Rejected
                    && fact.reason_code.as_deref() == Some("ownership_mismatch")
        )));
    }

    #[tokio::test]
    async fn send_to_idle_child_reports_resume_semantics() {
        let harness = build_agent_test_harness(TestLlmBehavior::Succeed {
            content: "完成。".to_string(),
        })
        .expect("test harness should build");
        let project = tempfile::tempdir().expect("tempdir should be created");
        let parent = harness
            .session_runtime
            .create_session(project.path().display().to_string())
            .await
            .expect("parent session should be created");
        let (child_agent_id, parent_session_id) =
            spawn_direct_child(&harness, &parent.session_id, project.path()).await;
        let parent_ctx = ToolContext::new(
            parent_session_id.into(),
            project.path().to_path_buf(),
            CancelToken::new(),
        )
        .with_turn_id("turn-parent-2")
        .with_agent_context(root_execution_event_context("root-agent", "root-profile"));

        let result = harness
            .service
            .send(
                SendAgentParams::ToChild(SendToChildParams {
                    agent_id: child_agent_id.into(),
                    message: "请继续整理结论".to_string(),
                    context: None,
                }),
                &parent_ctx,
            )
            .await
            .expect("send should succeed");

        assert_eq!(result.delivery_id(), None);
        assert!(
            result
                .summary()
                .is_some_and(|summary| summary.contains("已恢复"))
        );
        assert_eq!(
            result
                .delegation()
                .map(|metadata| metadata.responsibility_summary.as_str()),
            Some("检查 crates"),
            "resumed child should keep the original responsibility branch metadata"
        );
        assert_eq!(
            result.agent_ref().map(|child_ref| child_ref.lineage_kind),
            Some(astrcode_core::ChildSessionLineageKind::Resume),
            "resumed child projection should expose resume lineage instead of masquerading as \
             spawn"
        );
        let resumed_child = harness
            .kernel
            .get_handle(
                result
                    .agent_ref()
                    .map(|child_ref| child_ref.agent_id().as_str())
                    .expect("child ref should exist"),
            )
            .await
            .expect("resumed child handle should exist");
        assert_eq!(resumed_child.parent_turn_id, "turn-parent-2".into());
        assert_eq!(
            resumed_child.lineage_kind,
            astrcode_core::ChildSessionLineageKind::Resume
        );
    }

    #[tokio::test]
    async fn send_to_running_child_reports_mailbox_queue_semantics() {
        let harness = build_agent_test_harness(TestLlmBehavior::Succeed {
            content: "完成。".to_string(),
        })
        .expect("test harness should build");
        let project = tempfile::tempdir().expect("tempdir should be created");
        let parent = harness
            .session_runtime
            .create_session(project.path().display().to_string())
            .await
            .expect("parent session should be created");
        let (child_agent_id, parent_session_id) =
            spawn_direct_child(&harness, &parent.session_id, project.path()).await;
        let parent_ctx = ToolContext::new(
            parent_session_id.into(),
            project.path().to_path_buf(),
            CancelToken::new(),
        )
        .with_turn_id("turn-parent-3")
        .with_agent_context(root_execution_event_context("root-agent", "root-profile"));

        let _ = harness
            .kernel
            .agent_control()
            .set_lifecycle(
                &child_agent_id,
                astrcode_core::AgentLifecycleStatus::Running,
            )
            .await;

        let result = harness
            .service
            .send(
                SendAgentParams::ToChild(SendToChildParams {
                    agent_id: child_agent_id.into(),
                    message: "继续第二轮".to_string(),
                    context: Some("只看 CI".to_string()),
                }),
                &parent_ctx,
            )
            .await
            .expect("send should succeed");

        assert!(result.delivery_id().is_some());
        assert!(
            result
                .summary()
                .is_some_and(|summary| summary.contains("mailbox 排队"))
        );
    }

    #[tokio::test]
    async fn send_to_parent_rejects_root_execution_without_direct_parent() {
        let harness = build_agent_test_harness(TestLlmBehavior::Succeed {
            content: "完成。".to_string(),
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

        let root_ctx = ToolContext::new(
            parent.session_id.clone().into(),
            project.path().to_path_buf(),
            CancelToken::new(),
        )
        .with_turn_id("turn-root")
        .with_agent_context(root_execution_event_context("root-agent", "root-profile"));

        let error = harness
            .service
            .send(
                SendAgentParams::ToParent(SendToParentParams {
                    payload: ParentDeliveryPayload::Completed(CompletedParentDeliveryPayload {
                        message: "根节点不应该上行".to_string(),
                        findings: Vec::new(),
                        artifacts: Vec::new(),
                    }),
                }),
                &root_ctx,
            )
            .await
            .expect_err("root agent should not be able to send upward");
        assert!(error.to_string().contains("no direct parent"));

        let events = harness
            .session_runtime
            .replay_stored_events(&SessionId::from(parent.session_id.clone()))
            .await
            .expect("parent events should replay");
        assert!(events.iter().any(|stored| matches!(
            &stored.event.payload,
            StorageEventPayload::AgentCollaborationFact { fact, .. }
                if fact.action == AgentCollaborationActionKind::Delivery
                    && fact.outcome == AgentCollaborationOutcomeKind::Rejected
                    && fact.reason_code.as_deref() == Some("missing_direct_parent")
        )));
    }

    #[tokio::test]
    async fn send_to_parent_from_resumed_child_routes_to_current_parent_turn() {
        let harness = build_agent_test_harness(TestLlmBehavior::Succeed {
            content: "完成。".to_string(),
        })
        .expect("test harness should build");
        let project = tempfile::tempdir().expect("tempdir should be created");
        let parent = harness
            .session_runtime
            .create_session(project.path().display().to_string())
            .await
            .expect("parent session should be created");
        let (child_agent_id, parent_session_id) =
            spawn_direct_child(&harness, &parent.session_id, project.path()).await;
        let parent_ctx = ToolContext::new(
            parent_session_id.into(),
            project.path().to_path_buf(),
            CancelToken::new(),
        )
        .with_turn_id("turn-parent-2")
        .with_agent_context(root_execution_event_context("root-agent", "root-profile"));

        harness
            .service
            .send(
                SendAgentParams::ToChild(SendToChildParams {
                    agent_id: child_agent_id.clone().into(),
                    message: "继续整理并向我汇报".to_string(),
                    context: None,
                }),
                &parent_ctx,
            )
            .await
            .expect("send should resume idle child");

        let resumed_child = harness
            .kernel
            .get_handle(&child_agent_id)
            .await
            .expect("resumed child handle should exist");
        let child_ctx = ToolContext::new(
            resumed_child
                .child_session_id
                .clone()
                .expect("child session id should exist"),
            project.path().to_path_buf(),
            CancelToken::new(),
        )
        .with_turn_id("turn-child-report-2")
        .with_agent_context(subrun_event_context(&resumed_child));
        let metrics_before = harness.metrics.snapshot();

        let result = harness
            .service
            .send(
                SendAgentParams::ToParent(SendToParentParams {
                    payload: ParentDeliveryPayload::Completed(CompletedParentDeliveryPayload {
                        message: "继续推进后的显式上报".to_string(),
                        findings: Vec::new(),
                        artifacts: Vec::new(),
                    }),
                }),
                &child_ctx,
            )
            .await
            .expect("resumed child should be able to send upward");

        assert!(result.delivery_id().is_some());
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
                "explicit upstream send should trigger parent wake and drain delivery queue"
            );
            sleep(Duration::from_millis(20)).await;
        }

        let parent_events = harness
            .session_runtime
            .replay_stored_events(&SessionId::from(parent.session_id.clone()))
            .await
            .expect("parent events should replay");
        assert!(parent_events.iter().any(|stored| matches!(
            &stored.event.payload,
            StorageEventPayload::ChildSessionNotification { notification, .. }
                if stored.event.turn_id.as_deref() == Some("turn-parent-2")
                    && notification.child_ref.sub_run_id() == &resumed_child.sub_run_id
                    && notification.child_ref.lineage_kind
                        == astrcode_core::ChildSessionLineageKind::Resume
                    && notification.delivery.as_ref().is_some_and(|delivery| {
                        delivery.origin == astrcode_core::ParentDeliveryOrigin::Explicit
                            && delivery.payload.message() == "继续推进后的显式上报"
                    })
        )));
        assert!(
            !parent_events.iter().any(|stored| matches!(
                &stored.event.payload,
                StorageEventPayload::ChildSessionNotification { notification, .. }
                    if stored.event.turn_id.as_deref() == Some("turn-parent")
                        && notification.delivery.as_ref().is_some_and(|delivery| {
                            delivery.payload.message() == "继续推进后的显式上报"
                        })
            )),
            "resumed child delivery must target the current parent turn instead of the stale \
             spawn turn"
        );
        assert!(
            parent_events.iter().any(|stored| matches!(
                &stored.event.payload,
                StorageEventPayload::AgentMailboxQueued { payload }
                    if payload.envelope.message == "继续推进后的显式上报"
            )),
            "explicit upstream send should enqueue the same delivery for parent wake consumption"
        );
        assert!(
            parent_events.iter().any(|stored| matches!(
                &stored.event.payload,
                StorageEventPayload::UserMessage { content, .. }
                    if content.contains("delivery_id: child-send:")
                        && content.contains("message: 继续推进后的显式上报")
            )),
            "parent wake prompt should consume the explicit upstream delivery"
        );
        let metrics = harness.metrics.snapshot();
        assert!(
            metrics.execution_diagnostics.parent_reactivation_requested
                - metrics_before
                    .execution_diagnostics
                    .parent_reactivation_requested
                >= 1
        );
        assert!(
            metrics.execution_diagnostics.parent_reactivation_succeeded
                - metrics_before
                    .execution_diagnostics
                    .parent_reactivation_succeeded
                >= 1
        );
        assert!(
            metrics.execution_diagnostics.delivery_buffer_wake_succeeded
                - metrics_before
                    .execution_diagnostics
                    .delivery_buffer_wake_succeeded
                >= 1
        );
    }

    #[tokio::test]
    async fn send_to_parent_rejects_when_direct_parent_is_terminated() {
        let harness = build_agent_test_harness(TestLlmBehavior::Succeed {
            content: "完成。".to_string(),
        })
        .expect("test harness should build");
        let project = tempfile::tempdir().expect("tempdir should be created");
        let parent = harness
            .session_runtime
            .create_session(project.path().display().to_string())
            .await
            .expect("parent session should be created");
        let (child_agent_id, _) =
            spawn_direct_child(&harness, &parent.session_id, project.path()).await;
        let child_handle = harness
            .kernel
            .get_handle(&child_agent_id)
            .await
            .expect("child handle should exist");

        let _ = harness
            .kernel
            .agent_control()
            .set_lifecycle(
                "root-agent",
                astrcode_core::AgentLifecycleStatus::Terminated,
            )
            .await;

        let child_ctx = ToolContext::new(
            child_handle
                .child_session_id
                .clone()
                .expect("child session id should exist"),
            project.path().to_path_buf(),
            CancelToken::new(),
        )
        .with_turn_id("turn-child-report")
        .with_agent_context(subrun_event_context(&child_handle));

        let error = harness
            .service
            .send(
                SendAgentParams::ToParent(SendToParentParams {
                    payload: ParentDeliveryPayload::Completed(CompletedParentDeliveryPayload {
                        message: "父级已终止".to_string(),
                        findings: Vec::new(),
                        artifacts: Vec::new(),
                    }),
                }),
                &child_ctx,
            )
            .await
            .expect_err("terminated parent should reject upward send");
        assert!(error.to_string().contains("terminated"));

        let parent_events = harness
            .session_runtime
            .replay_stored_events(&SessionId::from(parent.session_id.clone()))
            .await
            .expect("parent events should replay");
        assert!(parent_events.iter().any(|stored| matches!(
            &stored.event.payload,
            StorageEventPayload::AgentCollaborationFact { fact, .. }
                if fact.action == AgentCollaborationActionKind::Delivery
                    && fact.outcome == AgentCollaborationOutcomeKind::Rejected
                    && fact.reason_code.as_deref() == Some("parent_terminated")
        )));
    }

    #[tokio::test]
    async fn close_reports_cascade_scope_for_descendants() {
        let harness = build_agent_test_harness(TestLlmBehavior::Succeed {
            content: "完成。".to_string(),
        })
        .expect("test harness should build");
        let project = tempfile::tempdir().expect("tempdir should be created");
        let parent = harness
            .session_runtime
            .create_session(project.path().display().to_string())
            .await
            .expect("parent session should be created");
        let (child_agent_id, parent_session_id) =
            spawn_direct_child(&harness, &parent.session_id, project.path()).await;

        let child_handle = harness
            .kernel
            .agent()
            .get_handle(&child_agent_id)
            .await
            .expect("child handle should exist");
        let child_ctx = ToolContext::new(
            child_handle
                .child_session_id
                .clone()
                .expect("child session id should exist"),
            project.path().to_path_buf(),
            CancelToken::new(),
        )
        .with_turn_id("turn-child-1")
        .with_agent_context(subrun_event_context(&child_handle));
        let _grandchild = harness
            .service
            .launch(
                SpawnAgentParams {
                    r#type: Some("reviewer".to_string()),
                    description: "进一步检查".to_string(),
                    prompt: "请进一步检查测试覆盖".to_string(),
                    context: None,
                    capability_grant: None,
                },
                &child_ctx,
            )
            .await
            .expect("grandchild spawn should succeed");

        let parent_ctx = ToolContext::new(
            parent_session_id.into(),
            project.path().to_path_buf(),
            CancelToken::new(),
        )
        .with_turn_id("turn-parent-close")
        .with_agent_context(root_execution_event_context("root-agent", "root-profile"));

        let result = harness
            .service
            .close(
                CloseAgentParams {
                    agent_id: child_agent_id.into(),
                },
                &parent_ctx,
            )
            .await
            .expect("close should succeed");

        assert_eq!(result.cascade(), Some(true));
        assert!(
            result
                .summary()
                .is_some_and(|summary| summary.contains("1 个后代"))
        );
    }
}
