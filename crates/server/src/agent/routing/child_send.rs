use super::*;

impl AgentOrchestrationService {
    /// resume 失败时恢复之前 drain 出的 inbox 信封。
    ///
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
    ) -> Result<T, AgentOrchestrationError> {
        self.restore_pending_inbox(agent_id, pending).await;
        Err(AgentOrchestrationError::Internal(message))
    }

    /// 如果子 agent 处于 Idle 且不占据并发槽位（如 Resume lineage），
    /// 则尝试 resume 它以处理新消息，而非排队等待。
    pub(in crate::agent) async fn resume_idle_child_if_needed(
        &self,
        child: &SubRunHandle,
        params: &SendToChildParams,
        ctx: &astrcode_tool_contract::ToolContext,
        collaboration: &ToolCollaborationContext,
        lifecycle: Option<AgentLifecycleStatus>,
    ) -> Result<Option<CollaborationResult>, AgentOrchestrationError> {
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
        let runtime = match self
            .resolve_runtime_config_for_session(child_session_id)
            .await
        {
            Ok(runtime) => runtime,
            Err(error) => {
                return self
                    .restore_pending_inbox_and_fail(
                        &child.agent_id,
                        pending,
                        format!(
                            "agent '{}' resume runtime resolution failed: {error}",
                            params.agent_id
                        ),
                    )
                    .await;
            },
        };
        let working_dir = match self
            .session_runtime
            .get_session_working_dir(child_session_id)
            .await
        {
            Ok(working_dir) => working_dir,
            Err(error) => {
                return self
                    .restore_pending_inbox_and_fail(
                        &child.agent_id,
                        pending,
                        format!(
                            "agent '{}' resume working directory resolution failed: {error}",
                            params.agent_id
                        ),
                    )
                    .await;
            },
        };
        let resumed_turn_id = format!("turn-{}", chrono::Utc::now().timestamp_millis());
        let resumed_input = ResumedChildGovernanceInput {
            session_id: child_session_id.to_string(),
            turn_id: resumed_turn_id.clone(),
            working_dir,
            mode_id: collaboration.mode_id().clone().into(),
            runtime: runtime.clone(),
            resolved_limits: reused_handle.resolved_limits.clone(),
            delegation: Some(resume_delegation.clone()),
            message: params.message.clone(),
            context: params.context.clone(),
            busy_policy: GovernanceBusyPolicy::RejectOnBusy,
        };
        let surface = match self.governance_surface.resumed_child_surface(resumed_input) {
            Ok(surface) => surface,
            Err(error) => {
                return self
                    .restore_pending_inbox_and_fail(
                        &child.agent_id,
                        pending,
                        format!(
                            "agent '{}' resume governance failed: {error}",
                            params.agent_id
                        ),
                    )
                    .await;
            },
        };
        match self
            .session_runtime
            .try_submit_prompt_for_agent_with_turn_id(
                child_session_id,
                resumed_turn_id.clone().into(),
                resume_message.clone(),
                surface.runtime.clone(),
                surface.into_submission(
                    subrun_event_context(&reused_handle),
                    collaboration.source_tool_call_id(),
                ),
            )
            .await
        {
            Ok(Some(accepted)) => accepted,
            Ok(None) => {
                self.restore_pending_inbox(&child.agent_id, pending).await;
                return Ok(None);
            },
            Err(error) => {
                return self
                    .restore_pending_inbox_and_fail(
                        &child.agent_id,
                        pending,
                        format!(
                            "agent '{}' resume prompt submission failed: {error}",
                            params.agent_id
                        ),
                    )
                    .await;
            },
        };

        let child_ref = self.build_child_ref_from_handle(&reused_handle).await;
        self.record_fact_best_effort(
            collaboration.runtime(),
            collaboration
                .fact(
                    AgentCollaborationActionKind::Send,
                    AgentCollaborationOutcomeKind::Delivered,
                )
                .child(&reused_handle)
                .summary("message delivered by reusing idle child"),
        )
        .await;

        Ok(Some(CollaborationResult::Sent {
            continuation: Some(astrcode_core::ExecutionContinuation::child_agent(
                self.project_child_ref_status(child_ref).await,
            )),
            delivery_id: None,
            summary: Some(format!(
                "子 Agent {} 已恢复空闲执行上下文并开始处理新消息。",
                params.agent_id
            )),
            delegation: Some(resume_delegation),
        }))
    }

    /// 当子 agent 正在运行时，将消息排入 input queue 并持久化 durable InputQueued 事件。
    pub(in crate::agent) async fn queue_message_for_active_child(
        &self,
        child: &SubRunHandle,
        params: &SendToChildParams,
        ctx: &astrcode_tool_contract::ToolContext,
        collaboration: &ToolCollaborationContext,
    ) -> Result<CollaborationResult, AgentOrchestrationError> {
        let delivery_id = format!(
            "send:{}:{}",
            ctx.turn_id().unwrap_or("unknown-turn"),
            ctx.tool_call_id().unwrap_or("tool-call-missing")
        );
        let envelope = AgentInboxEnvelope {
            delivery_id: delivery_id.clone(),
            from_agent_id: ctx
                .agent_context()
                .agent_id
                .clone()
                .map(|id| id.to_string())
                .unwrap_or_default(),
            to_agent_id: child.agent_id.to_string(),
            kind: InboxEnvelopeKind::ParentMessage,
            message: params.message.clone(),
            context: params.context.clone(),
            is_final: false,
            summary: None,
            findings: Vec::new(),
            artifacts: Vec::new(),
        };
        self.kernel
            .deliver(&child.agent_id, envelope.clone())
            .await
            .ok_or_else(|| {
                AgentOrchestrationError::NotFound(format!(
                    "agent '{}' not found while queueing message",
                    params.agent_id
                ))
            })?;
        self.append_durable_input_queue(child, &envelope, ctx)
            .await
            .map_err(AgentOrchestrationError::from)?;

        let child_ref = self.build_child_ref_from_handle(child).await;
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
            continuation: Some(astrcode_core::ExecutionContinuation::child_agent(
                self.project_child_ref_status(child_ref).await,
            )),
            delivery_id: Some(delivery_id.into()),
            summary: Some(format!(
                "子 Agent {} 正在运行；消息已进入 input queue 排队，待当前工作完成后处理。",
                params.agent_id
            )),
            delegation: child.delegation.clone(),
        })
    }
}

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
        .map(parent_delivery::render_parent_message_envelope)
        .collect::<Vec<_>>();
    parts.push(parent_delivery::render_parent_message_input(
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
