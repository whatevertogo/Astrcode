//! agent 协作路由与权限校验。
//!
//! 从旧 runtime/service/agent/routing.rs 迁入，去掉对 RuntimeService 的依赖，
//! 改为通过 Kernel + SessionRuntime 完成所有操作。

use astrcode_core::{
    AgentCollaborationActionKind, AgentCollaborationOutcomeKind, AgentInboxEnvelope,
    AgentLifecycleStatus, ChildAgentRef, ChildSessionLineageKind, CloseAgentParams,
    CollaborationResult, CollaborationResultKind, InboxEnvelopeKind, MailboxDiscardedPayload,
    MailboxQueuedPayload, SendAgentParams, SubRunHandle,
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

    /// 向子 Agent 追加消息（send 协作工具的业务逻辑）。
    pub async fn send_to_child(
        &self,
        params: SendAgentParams,
        ctx: &astrcode_core::ToolContext,
    ) -> Result<CollaborationResult, super::AgentOrchestrationError> {
        let collaboration = self.tool_collaboration_context(ctx)?;
        params
            .validate()
            .map_err(super::AgentOrchestrationError::from)?;

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

        Ok(CollaborationResult {
            accepted: true,
            kind: CollaborationResultKind::Closed,
            agent_ref: None,
            delivery_id: None,
            summary: Some(summary),
            observe_result: None,
            delegation: None,
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
        let lifecycle = self.kernel.get_lifecycle(&child_ref.agent_id).await;
        let last_turn_outcome = self.kernel.get_turn_outcome(&child_ref.agent_id).await;
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
        params: &SendAgentParams,
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

        let Some(reused_handle) = self.kernel.resume(&params.agent_id).await else {
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
            ctx.turn_id()
                .unwrap_or(&reused_handle.parent_turn_id)
                .to_string(),
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
        Ok(Some(CollaborationResult {
            accepted: true,
            kind: CollaborationResultKind::Sent,
            agent_ref: Some(self.project_child_ref_status(child_ref).await),
            delivery_id: None,
            summary: Some(format!(
                "子 Agent {} 已恢复，并开始处理新的具体指令。",
                params.agent_id
            )),
            observe_result: None,
            delegation: reused_handle.delegation.clone(),
            cascade: None,
            closed_root_agent_id: None,
            failure: None,
        }))
    }

    async fn queue_message_for_active_child(
        &self,
        child: &SubRunHandle,
        params: &SendAgentParams,
        ctx: &astrcode_core::ToolContext,
        collaboration: &super::ToolCollaborationContext,
    ) -> Result<CollaborationResult, super::AgentOrchestrationError> {
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

        Ok(CollaborationResult {
            accepted: true,
            kind: CollaborationResultKind::Sent,
            agent_ref: Some(self.project_child_ref_status(child_ref).await),
            delivery_id: Some(delivery_id),
            summary: Some(format!(
                "子 Agent {} 正在运行；消息已进入 mailbox 排队，待当前工作完成后处理。",
                params.agent_id
            )),
            observe_result: None,
            delegation: child.delegation.clone(),
            cascade: None,
            closed_root_agent_id: None,
            failure: None,
        })
    }
}

/// 将待处理的 inbox 信封与新的 send 输入拼接为 resume 消息。
fn compose_reusable_child_message(
    pending: &[astrcode_core::AgentInboxEnvelope],
    params: &astrcode_core::SendAgentParams,
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
            .unwrap_or_else(|| child.session_id.clone());

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
            .unwrap_or_else(|| ctx.session_id().to_string());

        let payload = MailboxQueuedPayload {
            envelope: astrcode_core::AgentMailboxEnvelope {
                delivery_id: envelope.delivery_id.clone(),
                from_agent_id: envelope.from_agent_id.clone(),
                to_agent_id: envelope.to_agent_id.clone(),
                message: render_parent_message_input(
                    &envelope.message,
                    envelope.context.as_deref(),
                ),
                queued_at: chrono::Utc::now(),
                sender_lifecycle_status,
                sender_last_turn_outcome,
                sender_open_session_id,
            },
        };

        self.session_runtime
            .append_agent_mailbox_queued(
                &target_session_id,
                ctx.turn_id().unwrap_or(&child.parent_turn_id),
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
                    target_agent_id: handle.agent_id.clone(),
                    delivery_ids: pending_delivery_ids,
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
    use std::time::Duration;

    use astrcode_core::{
        AgentCollaborationActionKind, AgentCollaborationOutcomeKind, CancelToken, CloseAgentParams,
        ObserveParams, SendAgentParams, SessionId, SpawnAgentParams, StorageEventPayload,
        ToolContext,
        agent::executor::{CollaborationExecutor, SubAgentExecutor},
    };
    use tokio::time::sleep;

    use super::super::{root_execution_event_context, subrun_event_context};
    use crate::{
        AgentKernelPort,
        agent::test_support::{TestLlmBehavior, build_agent_test_harness},
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
                SendAgentParams {
                    agent_id: child_agent_id.clone(),
                    message: "继续".to_string(),
                    context: None,
                },
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
                    agent_id: child_agent_id,
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
                SendAgentParams {
                    agent_id: child_agent_id,
                    message: "请继续整理结论".to_string(),
                    context: None,
                },
                &parent_ctx,
            )
            .await
            .expect("send should succeed");

        assert_eq!(result.delivery_id, None);
        assert!(
            result
                .summary
                .as_deref()
                .is_some_and(|summary| summary.contains("已恢复"))
        );
        assert_eq!(
            result
                .delegation
                .as_ref()
                .map(|metadata| metadata.responsibility_summary.as_str()),
            Some("检查 crates"),
            "resumed child should keep the original responsibility branch metadata"
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
                SendAgentParams {
                    agent_id: child_agent_id,
                    message: "继续第二轮".to_string(),
                    context: Some("只看 CI".to_string()),
                },
                &parent_ctx,
            )
            .await
            .expect("send should succeed");

        assert!(result.delivery_id.is_some());
        assert!(
            result
                .summary
                .as_deref()
                .is_some_and(|summary| summary.contains("mailbox 排队"))
        );
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
                .expect("child session id should exist")
                .into(),
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
                    agent_id: child_agent_id,
                },
                &parent_ctx,
            )
            .await
            .expect("close should succeed");

        assert_eq!(result.cascade, Some(true));
        assert!(
            result
                .summary
                .as_deref()
                .is_some_and(|summary| summary.contains("1 个后代"))
        );
    }
}
