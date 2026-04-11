//! 协作工具方法：send / wait / close / resume / deliver / fork。
//!
//! 这些方法实现父子 agent 之间的协作协议，
//! 均挂载在 `AgentExecutionServiceHandle` 上作为工具调用的后端处理。

use std::{sync::Arc, time::Instant};

use astrcode_core::{
    AgentEventContext, AgentInboxEnvelope, AgentLifecycleStatus, AgentMailboxEnvelope, AgentStatus,
    AstrError, CancelToken, ChildAgentRef, ChildSessionLineageKind, ChildSessionNotificationKind,
    CloseAgentParams, CollaborationResult, CollaborationResultKind, DeliverToParentParams,
    InboxEnvelopeKind, InvocationKind, LineageSnapshot, MailboxDiscardedPayload,
    MailboxQueuedPayload, ObserveAgentResult, ObserveParams, ResolvedExecutionLimitsSnapshot,
    ResolvedSubagentContextOverrides, ResumeAgentParams, SendAgentParams, SpawnAgentParams,
    StorageEvent, StorageEventPayload, SubRunHandle, SubRunResult, SubRunStorageMode, ToolContext,
    ToolEventSink, UserMessageOrigin, WaitAgentParams, WaitUntil,
};
use astrcode_runtime_execution::{
    DeliveryBufferStage, LineageMismatchKind, build_background_subrun_handoff,
    build_child_session_notification, build_resumed_child_agent_state, build_subrun_started_event,
    derive_child_execution_owner,
};
use astrcode_runtime_session::{
    SessionStateEventSink, append_mailbox_discarded, append_mailbox_queued,
};

use super::{root::AgentExecutionServiceHandle, subagent::SpawnedSubagentExecution};
use crate::service::{ServiceError, ServiceResult};

impl AgentExecutionServiceHandle {
    // ─── resumeChildSession ─────────────────────────────────

    /// 恢复已完成的子会话，复用同一 child session 继续协作。
    ///
    /// 与 `launch_subagent` 不同，resume 必须基于 child session durable replay 恢复，
    /// 并为同一个 child session mint 新的执行实例，而不是从空状态重新 spawn。
    pub async fn resume_child_session(
        &self,
        agent_id: &str,
        message: Option<String>,
        ctx: &ToolContext,
    ) -> ServiceResult<(SubRunHandle, SubRunResult)> {
        let parent = self.resolve_parent_execution(ctx).await?;
        let parent_event_sink: Arc<dyn ToolEventSink> = Arc::new(
            SessionStateEventSink::new(Arc::clone(&parent.parent_state))
                .map_err(|error| ServiceError::Internal(AstrError::Internal(error.to_string())))?,
        );

        // 查找现有的 child handle
        let child = self
            .runtime
            .agent_control
            .get(agent_id)
            .await
            .ok_or_else(|| ServiceError::InvalidInput(format!("agent '{agent_id}' not found")))?;

        // 只有终态 agent 可以被恢复
        if !child.status.is_final() {
            return Err(ServiceError::InvalidInput(format!(
                "agent '{}' is not in a final state (current: {:?})",
                agent_id, child.status
            )));
        }

        let existing_node = parent
            .parent_state
            .child_session_node(&child.sub_run_id)
            .map_err(|error| ServiceError::Internal(AstrError::Internal(error.to_string())))?
            .ok_or_else(|| {
                self.emit_resume_failure(
                    &parent_event_sink,
                    &parent.parent_turn_id,
                    self.resume_agent_context(&parent.parent_turn_id, &child),
                    "lineage_mismatch_descriptor_missing",
                    format!(
                        "resume rejected: child agent '{}' is missing durable child-session \
                         lineage in parent session '{}'",
                        child.agent_id, parent.parent_session_id
                    ),
                )
            })?;

        if !matches!(child.storage_mode, SubRunStorageMode::IndependentSession) {
            return Err(self.emit_resume_failure(
                &parent_event_sink,
                &parent.parent_turn_id,
                self.resume_agent_context(&parent.parent_turn_id, &child),
                "unsafe_resume_rejected",
                format!(
                    "resume rejected: child agent '{}' does not have an independent child session \
                     durable history",
                    child.agent_id
                ),
            ));
        }

        let Some(target_session_id) = child.child_session_id.clone() else {
            return Err(self.emit_lineage_mismatch(
                &parent_event_sink,
                &parent.parent_turn_id,
                self.resume_agent_context(&parent.parent_turn_id, &child),
                LineageMismatchKind::ChildSession,
                "lineage_mismatch_child_session",
                format!(
                    "resume rejected: child agent '{}' is missing child_session_id for durable \
                     replay",
                    child.agent_id
                ),
            ));
        };

        if existing_node.parent_session_id != parent.parent_session_id {
            return Err(self.emit_lineage_mismatch(
                &parent_event_sink,
                &parent.parent_turn_id,
                self.resume_agent_context(&parent.parent_turn_id, &child),
                LineageMismatchKind::ParentSession,
                "lineage_mismatch_parent_session",
                format!(
                    "resume rejected: child agent '{}' belongs to parent session '{}', not '{}'",
                    child.agent_id, existing_node.parent_session_id, parent.parent_session_id
                ),
            ));
        }

        if existing_node.parent_agent_id != parent.parent_agent_id_for_control {
            return Err(self.emit_lineage_mismatch(
                &parent_event_sink,
                &parent.parent_turn_id,
                self.resume_agent_context(&parent.parent_turn_id, &child),
                LineageMismatchKind::ParentAgent,
                "lineage_mismatch_parent_agent",
                format!(
                    "resume rejected: child agent '{}' parent ownership does not match current \
                     caller",
                    child.agent_id
                ),
            ));
        }

        if existing_node.child_session_id != target_session_id {
            return Err(self.emit_lineage_mismatch(
                &parent_event_sink,
                &parent.parent_turn_id,
                self.resume_agent_context(&parent.parent_turn_id, &child),
                LineageMismatchKind::ChildSession,
                "lineage_mismatch_child_session",
                format!(
                    "resume rejected: child agent '{}' points to child session '{}' but durable \
                     node expects '{}'",
                    child.agent_id, target_session_id, existing_node.child_session_id
                ),
            ));
        }

        let child_session_state = self
            .runtime
            .ensure_session_loaded(&target_session_id)
            .await
            .map_err(|error| {
                self.emit_resume_failure(
                    &parent_event_sink,
                    &parent.parent_turn_id,
                    self.resume_agent_context(&parent.parent_turn_id, &child),
                    "damaged_child_history",
                    format!(
                        "resume rejected: failed to load child session '{}' durable history: {}",
                        target_session_id, error
                    ),
                )
            })?;
        let replayed_state = child_session_state
            .snapshot_projected_state()
            .map_err(|error| {
                self.emit_resume_failure(
                    &parent_event_sink,
                    &parent.parent_turn_id,
                    self.resume_agent_context(&parent.parent_turn_id, &child),
                    "damaged_child_history",
                    format!(
                        "resume rejected: failed to rebuild child session '{}' visible state: {}",
                        target_session_id, error
                    ),
                )
            })?;
        if replayed_state.session_id.is_empty()
            || astrcode_runtime_session::normalize_session_id(&replayed_state.session_id)
                != astrcode_runtime_session::normalize_session_id(&target_session_id)
            || replayed_state.messages.is_empty()
        {
            return Err(self.emit_resume_failure(
                &parent_event_sink,
                &parent.parent_turn_id,
                self.resume_agent_context(&parent.parent_turn_id, &child),
                "unsafe_resume_rejected",
                format!(
                    "resume rejected: child session '{}' does not contain enough durable replay \
                     state",
                    target_session_id
                ),
            ));
        }

        // 通过 agent_control 恢复为新的执行实例。
        let resumed = self
            .runtime
            .agent_control
            .resume(agent_id)
            .await
            .ok_or_else(|| {
                ServiceError::InvalidInput(format!(
                    "agent '{}' cannot be resumed (not in a final state)",
                    agent_id
                ))
            })?;

        let child_cancel = self
            .runtime
            .agent_control
            .cancel_token(&resumed.agent_id)
            .await
            .unwrap_or_else(CancelToken::new);

        let child_turn_id = format!("{}-child-{}", parent.parent_turn_id, uuid::Uuid::new_v4());
        let child_agent = AgentEventContext::sub_run(
            resumed.agent_id.clone(),
            parent.parent_turn_id.clone(),
            resumed.agent_profile.clone(),
            resumed.sub_run_id.clone(),
            resumed.storage_mode,
            resumed.child_session_id.clone(),
        );
        let active_sink: Arc<dyn ToolEventSink> = Arc::new(
            SessionStateEventSink::new(Arc::clone(&child_session_state))
                .map_err(|error| ServiceError::Internal(AstrError::Internal(error.to_string())))?,
        );

        let mut child_node = existing_node.clone();
        child_node.agent_id = resumed.agent_id.clone();
        child_node.sub_run_id = resumed.sub_run_id.clone();
        child_node.lineage_kind = ChildSessionLineageKind::Resume;
        child_node.status = AgentStatus::Running;
        child_node.created_by_tool_call_id = ctx.tool_call_id().map(ToString::to_string);
        child_node.lineage_snapshot = Some(LineageSnapshot {
            source_agent_id: child.agent_id.clone(),
            source_session_id: target_session_id.clone(),
            source_sub_run_id: Some(child.sub_run_id.clone()),
        });
        parent
            .parent_state
            .upsert_child_session_node(child_node.clone())
            .map_err(|error| ServiceError::Internal(AstrError::Internal(error.to_string())))?;

        if let Err(error) = parent_event_sink.emit(build_subrun_started_event(
            &parent.parent_turn_id,
            child_agent.clone(),
            &resumed,
            ctx.tool_call_id().map(ToString::to_string),
            ResolvedSubagentContextOverrides::default(),
            ResolvedExecutionLimitsSnapshot::default(),
        )) {
            return Err(ServiceError::Internal(AstrError::Internal(format!(
                "failed to persist resumed SubRunStarted for child agent '{}' (subRunId='{}'): {}",
                resumed.agent_id, resumed.sub_run_id, error
            ))));
        }

        let resumed_notification = build_child_session_notification(
            &child_node,
            format!("child-resumed:{}", resumed.sub_run_id),
            ChildSessionNotificationKind::Resumed,
            format!("子 Agent {} 已恢复。", resumed.agent_id),
            AgentStatus::Running,
            None,
        );
        let _ = parent_event_sink.emit(StorageEvent {
            turn_id: Some(parent.parent_turn_id.clone()),
            agent: child_agent.clone(),
            payload: StorageEventPayload::ChildSessionNotification {
                notification: resumed_notification,
                timestamp: Some(chrono::Utc::now()),
            },
        });

        let resume_message = message.unwrap_or_else(|| "继续执行".to_string());
        let _ = active_sink.emit(StorageEvent {
            turn_id: Some(child_turn_id.clone()),
            agent: child_agent.clone(),
            payload: StorageEventPayload::UserMessage {
                content: resume_message.clone(),
                timestamp: chrono::Utc::now(),
                origin: UserMessageOrigin::User,
            },
        });

        let child_state = build_resumed_child_agent_state(replayed_state, &resume_message);

        let child_loop = {
            let profile = self
                .load_profiles_for_working_dir(ctx.working_dir())
                .await?
                .get(&resumed.agent_profile)
                .cloned()
                .ok_or_else(|| {
                    ServiceError::InvalidInput(format!(
                        "agent profile '{}' not found for resume",
                        resumed.agent_profile
                    ))
                })?;
            let request =
                astrcode_runtime_execution::AgentExecutionRequest::from_spawn_agent_params(
                    &SpawnAgentParams {
                        r#type: Some(profile.id.clone()),
                        description: "resume".to_string(),
                        prompt: resume_message.clone(),
                        context: None,
                    },
                    None,
                );
            self.prepare_scoped_execution_request(
                InvocationKind::SubRun,
                &profile,
                request,
                self.snapshot_execution_surface().await,
                Some(&parent.parent_snapshot),
            )?
            .loop_
        };

        let execution = SpawnedSubagentExecution {
            child: resumed.clone(),
            child_node,
            child_agent,
            child_turn_id,
            child_task: resume_message,
            child_execution_owner: derive_child_execution_owner(
                ctx,
                &parent.parent_turn_id,
                &resumed,
            ),
            child_state,
            child_loop,
            child_cancel,
            child_storage_mode: child.storage_mode,
            parent_session_id: parent.parent_session_id.clone(),
            parent_turn_id: parent.parent_turn_id.clone(),
            parent_state: Arc::clone(&parent.parent_state),
            parent_tool_call_id: ctx.tool_call_id().map(ToString::to_string),
            parent_event_sink,
            active_sink,
            resolved_overrides: ResolvedSubagentContextOverrides::default(),
            resolved_limits: ResolvedExecutionLimitsSnapshot::default(),
        };

        let running_result = SubRunResult {
            status: AgentStatus::Running,
            handoff: Some(build_background_subrun_handoff(
                &execution.child,
                &execution.parent_session_id,
            )),
            failure: None,
        };

        let service = self.clone();
        let handle = tokio::spawn(async move {
            let started_at = Instant::now();
            let (outcome, tracker) = service.run_child_loop(&execution).await;
            if let Err(error) = service
                .finalize_child_execution(execution, tracker, started_at, outcome)
                .await
            {
                log::error!("failed to finalize resumed child execution: {}", error);
            }
        });
        self.runtime.lifecycle().register_subagent_task(handle);

        Ok((resumed, running_result))
    }

    fn resume_agent_context(
        &self,
        parent_turn_id: &str,
        child: &SubRunHandle,
    ) -> AgentEventContext {
        AgentEventContext::sub_run(
            child.agent_id.clone(),
            parent_turn_id.to_string(),
            child.agent_profile.clone(),
            child.sub_run_id.clone(),
            child.storage_mode,
            child.child_session_id.clone(),
        )
    }

    fn emit_lineage_mismatch(
        &self,
        parent_event_sink: &Arc<dyn ToolEventSink>,
        parent_turn_id: &str,
        agent: AgentEventContext,
        kind: LineageMismatchKind,
        code: &str,
        message: String,
    ) -> ServiceError {
        self.runtime.observability.record_lineage_mismatch(kind);
        log::warn!(
            "resume lineage mismatch detected: kind='{}', {}",
            kind.as_str(),
            message
        );
        self.emit_resume_failure(parent_event_sink, parent_turn_id, agent, code, message)
    }

    fn emit_resume_failure(
        &self,
        parent_event_sink: &Arc<dyn ToolEventSink>,
        parent_turn_id: &str,
        agent: AgentEventContext,
        code: &str,
        message: String,
    ) -> ServiceError {
        let _ = parent_event_sink.emit(StorageEvent {
            turn_id: Some(parent_turn_id.to_string()),
            agent,
            payload: StorageEventPayload::Error {
                message: format!("{code}: {message}"),
                timestamp: Some(chrono::Utc::now()),
            },
        });
        ServiceError::Conflict(format!("{code}: {message}"))
    }

    // ─── 所有权验证 ──────────────────────────────────────────

    /// 验证调用者是否为目标子 agent 的直接父级。
    ///
    /// 协作工具必须验证所有权，防止任意 agent 操作非子级 agent。
    pub(super) fn verify_caller_owns_child(
        &self,
        ctx: &ToolContext,
        child_handle: &SubRunHandle,
    ) -> ServiceResult<()> {
        let caller_agent_id = ctx.agent_context().agent_id.as_deref();
        let child_parent_id = child_handle.parent_agent_id.as_deref();

        match (caller_agent_id, child_parent_id) {
            // 调用者有 agent_id，必须匹配子 agent 的 parent_agent_id
            (Some(caller), Some(parent)) if caller == parent => Ok(()),
            // 子 agent 没有父（顶层），只有根执行可以操作
            (None, None) => Ok(()),
            // 所有权不匹配
            _ => Err(ServiceError::InvalidInput(format!(
                "caller '{}' does not own agent '{}' (parent: {})",
                caller_agent_id.unwrap_or("<root>"),
                child_handle.agent_id,
                child_parent_id.unwrap_or("<none>")
            ))),
        }
    }

    // ─── sendAgent ──────────────────────────────────────────

    /// 向子 Agent 追加消息。
    ///
    /// 四工具模型约束：
    /// - 目标为 Terminated → 直接报错，不入 mailbox
    /// - 目标为 Idle → 消息入队后触发目标下一轮
    /// - 目标为 Running → 消息只入队，不插入当前轮
    pub async fn send_to_child(
        &self,
        params: SendAgentParams,
        ctx: &ToolContext,
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

        // 四工具模型：Terminated 的 agent 拒收任何新 send
        let lifecycle = self
            .runtime
            .agent_control
            .get_lifecycle(&params.agent_id)
            .await;
        if matches!(
            lifecycle,
            Some(astrcode_core::AgentLifecycleStatus::Terminated)
        ) {
            return Err(ServiceError::InvalidInput(format!(
                "agent '{}' has been terminated and cannot receive new messages",
                params.agent_id
            )));
        }

        if matches!(lifecycle, Some(astrcode_core::AgentLifecycleStatus::Idle))
            && child.status.is_final()
        {
            let pending = self
                .runtime
                .agent_control
                .drain_inbox(&child.agent_id)
                .await
                .unwrap_or_default();
            let resume_message = compose_reusable_child_message(&pending, &params);
            let (reused_handle, _) = self
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
                parent_agent_id: None,
                cascade: None,
                closed_root_agent_id: None,
                failure: None,
            });
        }

        let delivery_id = format!("delivery-{}", uuid::Uuid::new_v4());
        let envelope = AgentInboxEnvelope {
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
            parent_agent_id: None,
            cascade: None,
            closed_root_agent_id: None,
            failure: None,
        })
    }

    // ─── waitAgent ──────────────────────────────────────────

    /// 等待子 Agent 到达可消费状态。
    pub async fn wait_for_child(
        &self,
        params: WaitAgentParams,
        ctx: &ToolContext,
    ) -> ServiceResult<CollaborationResult> {
        params.validate().map_err(ServiceError::from)?;

        let handle = match params.until {
            WaitUntil::Final => self
                .runtime
                .agent_control
                .wait(&params.agent_id)
                .await
                .ok_or_else(|| {
                    ServiceError::NotFound(format!(
                        "agent '{}' not found or already finalized",
                        params.agent_id
                    ))
                })?,
            WaitUntil::NextDelivery => self
                .runtime
                .agent_control
                .wait_for_inbox(&params.agent_id)
                .await
                .ok_or_else(|| {
                    ServiceError::NotFound(format!(
                        "agent '{}' not found for inbox wait",
                        params.agent_id
                    ))
                })?,
        };

        self.verify_caller_owns_child(ctx, &handle)?;

        let child_ref = self.build_child_ref_from_handle(&handle).await;
        let summary = match handle.status {
            AgentStatus::Completed => "子 Agent 已完成".to_string(),
            AgentStatus::Failed => "子 Agent 执行失败".to_string(),
            AgentStatus::Cancelled => "子 Agent 已取消".to_string(),
            _ => format!("子 Agent 状态: {:?}", handle.status),
        };

        log::info!(
            "waitAgent: wait resolved for child agent '{}' (subRunId='{}', status={:?})",
            params.agent_id,
            handle.sub_run_id,
            handle.status
        );

        Ok(CollaborationResult {
            accepted: true,
            kind: CollaborationResultKind::WaitResolved,
            agent_ref: Some(child_ref),
            delivery_id: None,
            summary: Some(summary),
            parent_agent_id: None,
            cascade: None,
            closed_root_agent_id: None,
            failure: None,
        })
    }

    // ─── closeAgent ─────────────────────────────────────────

    /// 关闭子 Agent（四工具模型 close 语义）。
    ///
    /// 统一使用 subtree terminate 语义：
    /// 1. 对整棵子树设置 lifecycle = Terminated
    /// 2. 触发 cancel token 中断正在运行的 turn
    /// 3. 级联到所有后代
    pub async fn close_child(
        &self,
        params: CloseAgentParams,
        ctx: &ToolContext,
    ) -> ServiceResult<CollaborationResult> {
        params.validate().map_err(ServiceError::from)?;

        // 先获取 handle 验证所有权，再执行关闭
        let target = self
            .runtime
            .agent_control
            .get(&params.agent_id)
            .await
            .ok_or_else(|| {
                ServiceError::NotFound(format!("agent '{}' not found", params.agent_id))
            })?;
        self.verify_caller_owns_child(ctx, &target)?;

        // 收集子树大小用于摘要（terminate_subtree 前统计）
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

        // 使用四工具模型的 terminate_subtree 替代旧 cancel
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

        log::info!(
            "close: child agent '{}' closed (subtree_size={})",
            params.agent_id,
            subtree_count
        );

        Ok(CollaborationResult {
            accepted: true,
            kind: CollaborationResultKind::Closed,
            agent_ref: None,
            delivery_id: None,
            summary: Some(summary),
            parent_agent_id: cancelled.parent_agent_id,
            cascade: Some(true),
            closed_root_agent_id: Some(cancelled.agent_id.clone()),
            failure: None,
        })
    }

    // ─── resumeAgent ────────────────────────────────────────

    /// 恢复子 Agent。
    pub async fn resume_child(
        &self,
        params: ResumeAgentParams,
        ctx: &ToolContext,
    ) -> ServiceResult<CollaborationResult> {
        params.validate().map_err(ServiceError::from)?;

        // 验证调用者所有权
        let target = self
            .runtime
            .agent_control
            .get(&params.agent_id)
            .await
            .ok_or_else(|| {
                ServiceError::NotFound(format!("agent '{}' not found", params.agent_id))
            })?;
        self.verify_caller_owns_child(ctx, &target)?;

        // 使用已有的 resume_child_session 方法
        let (resumed_handle, result) = self
            .resume_child_session(&params.agent_id, params.message.clone(), ctx)
            .await?;

        log::info!(
            "resumeAgent: child agent '{}' resumed with message_len={}",
            params.agent_id,
            params.message.as_ref().map(|m| m.len()).unwrap_or(0)
        );

        let child_ref = Some(ChildAgentRef {
            agent_id: resumed_handle.agent_id.clone(),
            session_id: resumed_handle.session_id.clone(),
            sub_run_id: resumed_handle.sub_run_id.clone(),
            parent_agent_id: resumed_handle.parent_agent_id.clone(),
            lineage_kind: ChildSessionLineageKind::Resume,
            status: AgentStatus::Running,
            open_session_id: resumed_handle
                .child_session_id
                .clone()
                .unwrap_or_else(|| resumed_handle.session_id.clone()),
        });

        Ok(CollaborationResult {
            accepted: true,
            kind: CollaborationResultKind::Resumed,
            agent_ref: child_ref,
            delivery_id: None,
            summary: Some(result.handoff.map(|h| h.summary).unwrap_or_default()),
            parent_agent_id: None,
            cascade: None,
            closed_root_agent_id: None,
            failure: None,
        })
    }

    // ─── deliverToParent ────────────────────────────────────

    /// 向直接父 Agent 交付结果。
    ///
    /// 强制直接父路由：只投递到 handle 中记录的 parent_agent_id，
    /// 不允许越级投递到祖父或更上层 agent。
    /// 交付是一次性的：drain_inbox 消费后不再可重复消费。
    pub async fn deliver_to_parent(
        &self,
        params: DeliverToParentParams,
        ctx: &ToolContext,
    ) -> ServiceResult<CollaborationResult> {
        params.validate().map_err(ServiceError::from)?;

        // 查找当前 agent 的父 agent ID：从 agent_control 获取 handle 的 parent_agent_id
        let current_agent_id = ctx.agent_context().agent_id.clone().ok_or_else(|| {
            ServiceError::InvalidInput("deliverToParent requires an agent context".to_string())
        })?;

        let current_handle = self
            .runtime
            .agent_control
            .get(&current_agent_id)
            .await
            .ok_or_else(|| {
                ServiceError::NotFound(format!("agent '{}' not found", current_agent_id))
            })?;

        // 直接父路由：只能向 handle 中记录的直接 parent 投递
        let parent_agent_id = current_handle.parent_agent_id.clone().ok_or_else(|| {
            ServiceError::InvalidInput(
                "deliverToParent can only be called by a child agent".to_string(),
            )
        })?;

        // 验证父 agent 确实存在于当前注册表中，防止向已移除的 agent 投递
        let parent_handle = self
            .runtime
            .agent_control
            .get(&parent_agent_id)
            .await
            .ok_or_else(|| {
                ServiceError::NotFound(format!(
                    "direct parent agent '{}' not found — delivery rejected",
                    parent_agent_id
                ))
            })?;

        // 验证路由一致性：parent 的 children 应包含当前 agent
        // 使用 ancestor_chain 确认当前 agent 确实是 parent 的直接子节点
        let ancestor_chain = self
            .runtime
            .agent_control
            .ancestor_chain(&current_agent_id)
            .await;
        if ancestor_chain.len() < 2 {
            return Err(ServiceError::InvalidInput(
                "deliverToParent requires at least a two-level agent chain".to_string(),
            ));
        }
        // 祖先链的第一个是自身，第二个应是直接父
        if ancestor_chain[1].agent_id != parent_agent_id {
            return Err(ServiceError::InvalidInput(format!(
                "direct parent routing mismatch: expected '{}', found '{}' in ancestor chain",
                parent_agent_id, ancestor_chain[1].agent_id
            )));
        }

        let delivery_id = format!("delivery-{}", uuid::Uuid::new_v4());

        let envelope = astrcode_core::AgentInboxEnvelope {
            delivery_id: delivery_id.clone(),
            from_agent_id: current_agent_id.clone(),
            to_agent_id: parent_agent_id.clone(),
            kind: InboxEnvelopeKind::ChildDelivery,
            message: params
                .final_reply
                .clone()
                .unwrap_or_else(|| params.summary.clone()),
            context: None,
            is_final: params.final_reply.is_some(),
            summary: Some(params.summary.clone()),
            findings: params.findings.clone(),
            artifacts: params.artifacts.clone(),
        };

        self.runtime
            .agent_control
            .push_inbox(&parent_agent_id, envelope)
            .await
            .ok_or_else(|| {
                ServiceError::NotFound(format!(
                    "parent agent '{}' inbox not available for delivery",
                    parent_agent_id
                ))
            })?;
        self.runtime
            .observability
            .record_delivery_buffer(DeliveryBufferStage::Queued);

        let _ = parent_handle; // 父 handle 已用于验证，释放借用

        log::info!(
            "deliverToParent: child '{}' delivered to direct parent '{}' (deliveryId='{}', \
             isFinal={})",
            current_agent_id,
            parent_agent_id,
            delivery_id,
            params.final_reply.is_some()
        );

        Ok(CollaborationResult {
            accepted: true,
            kind: CollaborationResultKind::Delivered,
            agent_ref: None,
            delivery_id: Some(delivery_id),
            summary: Some(format!(
                "结果已交付给直接父 Agent {}（一次性消费）",
                parent_agent_id
            )),
            parent_agent_id: Some(parent_agent_id),
            cascade: None,
            closed_root_agent_id: None,
            failure: None,
        })
    }

    // ─── observe ────────────────────────────────────────────

    /// 获取目标 child agent 的增强快照（四工具模型 observe）。
    ///
    /// 只返回直接子 agent 的快照，融合三层信息：
    /// - live lifecycle + last_turn_outcome（来自 agent_control）
    /// - 对话投影 phase/turn_count/last_output（来自 SessionState 投影）
    /// - mailbox pending count（来自 durable replay）
    pub async fn observe_child(
        &self,
        params: ObserveParams,
        ctx: &ToolContext,
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

        // 读取 live lifecycle 状态
        let lifecycle_status = self
            .runtime
            .agent_control
            .get_lifecycle(&params.agent_id)
            .await
            .unwrap_or(AgentLifecycleStatus::Pending);

        let last_turn_outcome = self
            .runtime
            .agent_control
            .get_turn_outcome(&params.agent_id)
            .await
            .flatten();

        // 读取对话投影（phase/turn_count/last_output）
        let open_session_id = child
            .child_session_id
            .clone()
            .unwrap_or_else(|| child.session_id.clone());
        let session_state = self.runtime.ensure_session_loaded(&open_session_id).await?;
        let projected = session_state
            .snapshot_projected_state()
            .map_err(|error| ServiceError::Internal(AstrError::Internal(error.to_string())))?;

        // 读取 mailbox pending count
        let pending_message_count = session_state
            .mailbox_projection_for_agent(&params.agent_id)
            .map(|p| p.pending_message_count())
            .unwrap_or(0);

        let observe_result = ObserveAgentResult {
            agent_id: child.agent_id.clone(),
            sub_run_id: child.sub_run_id.clone(),
            session_id: child.session_id.clone(),
            open_session_id,
            parent_agent_id: child.parent_agent_id.clone().unwrap_or_default(),
            lifecycle_status,
            last_turn_outcome,
            phase: format!("{:?}", projected.phase),
            turn_count: projected.turn_count as u32,
            pending_message_count,
            last_output: extract_last_output(&projected.messages),
        };

        log::info!(
            "observe: snapshot for child agent '{}' (lifecycle={:?}, pending={})",
            params.agent_id,
            lifecycle_status,
            pending_message_count
        );

        Ok(CollaborationResult {
            accepted: true,
            kind: CollaborationResultKind::Sent, // 复用 Sent 作为 observe result kind
            agent_ref: Some(
                self.project_child_ref_status(self.build_child_ref_from_handle(&child).await)
                    .await,
            ),
            delivery_id: None,
            summary: Some(serde_json::to_string(&observe_result).unwrap_or_default()),
            parent_agent_id: None,
            cascade: None,
            closed_root_agent_id: None,
            failure: None,
        })
    }

    // ─── 辅助方法 ───────────────────────────────────────────

    /// 从 SubRunHandle 构建稳定 ChildAgentRef。
    pub(super) async fn build_child_ref_from_handle(&self, handle: &SubRunHandle) -> ChildAgentRef {
        self.build_child_ref_with_lineage(handle, ChildSessionLineageKind::Spawn)
            .await
    }

    /// 从 SubRunHandle 构建带指定 lineage kind 的稳定 ChildAgentRef。
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
            status: handle.status,
            open_session_id: handle
                .child_session_id
                .clone()
                .unwrap_or_else(|| handle.session_id.clone()),
        }
    }

    /// 用 lifecycle/outcome 修正过渡期协作面暴露的旧状态值。
    async fn project_child_ref_status(&self, mut child_ref: ChildAgentRef) -> ChildAgentRef {
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
                project_collaboration_status(lifecycle, last_turn_outcome, child_ref.status);
        }
        child_ref
    }

    // ─── forkChildSession ───────────────────────────────────

    /// Fork 一个子会话，复用现有的 `launch_subagent` 基础设施。
    ///
    /// 与 `launch_subagent` 共享相同的 session 创建和事件持久化路径，
    /// 但在 `ChildSessionNode` 中设置 `lineage_kind: Fork` 并填充 `lineage_snapshot`，
    /// 以标记来源上下文而非创建第二套生命周期系统。
    pub async fn fork_child_session(
        &self,
        params: SpawnAgentParams,
        source_agent_id: String,
        source_session_id: String,
        source_sub_run_id: Option<String>,
        ctx: &ToolContext,
    ) -> ServiceResult<SubRunResult> {
        params.validate().map_err(ServiceError::from)?;
        let profile = self.resolve_profile(&params, ctx.working_dir()).await?;
        let parent = self.resolve_parent_execution(ctx).await?;
        let result = self.launch_background(params, profile, parent, ctx).await?;

        // 找到刚创建的 child，更新其 ChildSessionNode 的 lineage 信息
        if let Some(ref handoff) = result.handoff {
            if let Some(sub_run_id) = handoff.artifacts.first().map(|a| a.id.as_str()) {
                let session_state = self.runtime.ensure_session_loaded(ctx.session_id()).await?;
                if let Some(mut node) = session_state
                    .child_session_node(sub_run_id)
                    .map_err(|e| ServiceError::Internal(AstrError::Internal(e.to_string())))?
                {
                    node.lineage_kind = ChildSessionLineageKind::Fork;
                    node.lineage_snapshot = Some(astrcode_core::LineageSnapshot {
                        source_agent_id,
                        source_session_id,
                        source_sub_run_id,
                    });
                    session_state
                        .upsert_child_session_node(node)
                        .map_err(|e| ServiceError::Internal(AstrError::Internal(e.to_string())))?;
                }
            }
        }

        Ok(result)
    }
}

/// 从消息列表中提取最后一条 assistant 消息的输出摘要。
fn extract_last_output(messages: &[astrcode_core::LlmMessage]) -> Option<String> {
    messages.iter().rev().find_map(|msg| match msg {
        astrcode_core::LlmMessage::Assistant { content, .. } => {
            if content.is_empty() {
                None
            } else if content.len() > 200 {
                Some(format!("{}...", &content[..200]))
            } else {
                Some(content.clone())
            }
        },
        _ => None,
    })
}

fn project_collaboration_status(
    lifecycle: AgentLifecycleStatus,
    last_turn_outcome: Option<astrcode_core::AgentTurnOutcome>,
    fallback: AgentStatus,
) -> AgentStatus {
    match lifecycle {
        AgentLifecycleStatus::Pending => AgentStatus::Pending,
        AgentLifecycleStatus::Running => AgentStatus::Running,
        AgentLifecycleStatus::Idle => match last_turn_outcome {
            Some(astrcode_core::AgentTurnOutcome::Completed) => AgentStatus::Completed,
            Some(astrcode_core::AgentTurnOutcome::Failed) => AgentStatus::Failed,
            Some(astrcode_core::AgentTurnOutcome::Cancelled) => AgentStatus::Cancelled,
            Some(astrcode_core::AgentTurnOutcome::TokenExceeded) => AgentStatus::TokenExceeded,
            None => fallback,
        },
        AgentLifecycleStatus::Terminated => AgentStatus::Cancelled,
    }
}

fn compose_reusable_child_message(
    pending: &[astrcode_core::AgentInboxEnvelope],
    params: &SendAgentParams,
) -> String {
    let mut parts = pending
        .iter()
        .filter(|envelope| matches!(envelope.kind, InboxEnvelopeKind::ParentMessage))
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

impl AgentExecutionServiceHandle {
    async fn append_durable_mailbox_queue(
        &self,
        child: &SubRunHandle,
        envelope: &AgentInboxEnvelope,
        ctx: &ToolContext,
    ) -> ServiceResult<()> {
        let target_session_id = child
            .child_session_id
            .clone()
            .unwrap_or_else(|| child.session_id.clone());
        let target_session = self
            .runtime
            .ensure_session_loaded(&target_session_id)
            .await?;
        let sender_agent_id = ctx.agent_context().agent_id.clone().unwrap_or_default();
        let sender_lifecycle_status = if sender_agent_id.is_empty() {
            AgentLifecycleStatus::Running
        } else {
            self.runtime
                .agent_control
                .get_lifecycle(&sender_agent_id)
                .await
                .unwrap_or(AgentLifecycleStatus::Running)
        };
        let sender_last_turn_outcome = if sender_agent_id.is_empty() {
            None
        } else {
            self.runtime
                .agent_control
                .get_turn_outcome(&sender_agent_id)
                .await
                .flatten()
        };
        let sender_open_session_id = ctx
            .agent_context()
            .child_session_id
            .clone()
            .unwrap_or_else(|| ctx.session_id().to_string());
        let payload = MailboxQueuedPayload {
            envelope: AgentMailboxEnvelope {
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
        let event_agent = AgentEventContext::sub_run(
            child.agent_id.clone(),
            child.parent_turn_id.clone(),
            child.agent_profile.clone(),
            child.sub_run_id.clone(),
            child.storage_mode,
            child.child_session_id.clone(),
        );
        let mut translator = astrcode_core::EventTranslator::new(target_session.current_phase()?);
        append_mailbox_queued(
            &target_session,
            ctx.turn_id().unwrap_or(&child.parent_turn_id),
            event_agent,
            payload,
            &mut translator,
        )
        .await
        .map_err(|error| ServiceError::Internal(AstrError::Internal(error.to_string())))?;
        Ok(())
    }

    async fn append_durable_mailbox_discard_batch(
        &self,
        handles: &[SubRunHandle],
        ctx: &ToolContext,
    ) -> ServiceResult<()> {
        for handle in handles {
            self.append_durable_mailbox_discard(handle, ctx).await?;
        }
        Ok(())
    }

    async fn append_durable_mailbox_discard(
        &self,
        handle: &SubRunHandle,
        ctx: &ToolContext,
    ) -> ServiceResult<()> {
        let target_session_id = handle
            .child_session_id
            .clone()
            .unwrap_or_else(|| handle.session_id.clone());
        let target_session = self
            .runtime
            .ensure_session_loaded(&target_session_id)
            .await?;
        let projection = target_session
            .mailbox_projection_for_agent(&handle.agent_id)
            .map_err(|error| ServiceError::Internal(AstrError::Internal(error.to_string())))?;
        if projection.pending_delivery_ids.is_empty() {
            return Ok(());
        }

        let payload = MailboxDiscardedPayload {
            target_agent_id: handle.agent_id.clone(),
            delivery_ids: projection.pending_delivery_ids,
        };
        // discard payload 自己已经携带 agent_id；这里如果再 flatten 一个 sub_run 上下文，
        // durable JSON 会出现重复的 agentId 字段，重放时会被判为损坏文件。
        let event_agent = AgentEventContext::default();
        let mut translator = astrcode_core::EventTranslator::new(target_session.current_phase()?);
        append_mailbox_discarded(
            &target_session,
            ctx.turn_id().unwrap_or(&handle.parent_turn_id),
            event_agent,
            payload,
            &mut translator,
        )
        .await
        .map_err(|error| ServiceError::Internal(AstrError::Internal(error.to_string())))?;
        Ok(())
    }
}
