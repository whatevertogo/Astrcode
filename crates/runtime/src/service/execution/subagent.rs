use std::{sync::Arc, time::Instant};

use astrcode_core::{
    AgentEventContext, AgentProfile, AgentState, AgentStatus, AstrError, CancelToken,
    ChildSessionNotificationKind, ChildSessionStatusSource, ExecutionOwner, InvocationKind,
    ResolvedExecutionLimitsSnapshot, ResolvedSubagentContextOverrides, SpawnAgentParams,
    StorageEvent, StorageEventPayload, SubRunHandle, SubRunResult, SubRunStorageMode, ToolContext,
    ToolEventSink, UserMessageOrigin,
};
use astrcode_runtime_agent_loop::{AgentLoop, ChildExecutionTracker, TurnOutcome};
use astrcode_runtime_execution::{
    ChildLifecycleStage, PreparedAgentExecution, build_background_subrun_handoff,
    build_child_agent_state, build_child_session_node, build_child_session_notification,
    build_subrun_failure, build_subrun_finished_event, build_subrun_handoff,
    build_subrun_started_event, child_delivery_outcome_label, derive_child_execution_owner,
    ensure_subagent_mode,
};
use astrcode_runtime_session::{SessionState, SessionStateEventSink};

use super::{root::AgentExecutionServiceHandle, status::project_child_terminal_delivery};
use crate::service::{ServiceError, ServiceResult};

pub(super) struct ParentExecutionContext {
    pub(super) parent_session_id: String,
    pub(super) parent_turn_id: String,
    pub(super) parent_state: Arc<SessionState>,
    pub(super) parent_snapshot: AgentState,
    pub(super) parent_agent_id_for_control: Option<String>,
}

struct PreparedSubagentExecution {
    profile: AgentProfile,
    prepared_execution: PreparedAgentExecution<Arc<AgentLoop>>,
    child_storage_mode: SubRunStorageMode,
    child_task: String,
}

pub(super) struct SpawnedSubagentExecution {
    pub(super) child: SubRunHandle,
    pub(super) child_node: astrcode_core::ChildSessionNode,
    pub(super) child_agent: AgentEventContext,
    pub(super) child_turn_id: String,
    pub(super) child_task: String,
    pub(super) child_execution_owner: ExecutionOwner,
    pub(super) child_state: AgentState,
    pub(super) child_loop: Arc<AgentLoop>,
    pub(super) child_cancel: CancelToken,
    pub(super) child_storage_mode: SubRunStorageMode,
    pub(super) parent_session_id: String,
    pub(super) parent_turn_id: String,
    pub(super) parent_state: Arc<SessionState>,
    pub(super) parent_tool_call_id: Option<String>,
    pub(super) parent_event_sink: Arc<dyn ToolEventSink>,
    pub(super) active_sink: Arc<dyn ToolEventSink>,
    pub(super) resolved_overrides: ResolvedSubagentContextOverrides,
    pub(super) resolved_limits: ResolvedExecutionLimitsSnapshot,
}

impl AgentExecutionServiceHandle {
    pub async fn launch_subagent(
        &self,
        params: SpawnAgentParams,
        ctx: &ToolContext,
    ) -> ServiceResult<SubRunResult> {
        params.validate().map_err(ServiceError::from)?;
        let profile = self.resolve_profile(&params, ctx.working_dir()).await?;
        let parent = self.resolve_parent_execution(ctx).await?;
        self.launch_background(params, profile, parent, ctx).await
    }

    pub(super) async fn resolve_profile(
        &self,
        params: &SpawnAgentParams,
        working_dir: &std::path::Path,
    ) -> ServiceResult<AgentProfile> {
        let profile_id = params.r#type.as_deref().unwrap_or("explore");
        let profile = self
            .load_profiles_for_working_dir(working_dir)
            .await?
            .get(profile_id)
            .cloned()
            .ok_or_else(|| {
                ServiceError::InvalidInput(format!("unknown agent profile '{profile_id}'"))
            })?;
        ensure_subagent_mode(&profile)?;
        Ok(profile)
    }

    pub(super) async fn resolve_parent_execution(
        &self,
        ctx: &ToolContext,
    ) -> ServiceResult<ParentExecutionContext> {
        let parent_turn_id = ctx.turn_id().ok_or_else(|| {
            ServiceError::InvalidInput("spawn requires a parent turn id".to_string())
        })?;
        let parent_state = self.runtime.ensure_session_loaded(ctx.session_id()).await?;
        let parent_snapshot = parent_state
            .snapshot_projected_state()
            .map_err(|error| ServiceError::Internal(AstrError::Internal(error.to_string())))?;
        // 四工具模型：root agent 也注册在控制树中（depth=0），
        // 所以 root 下创建 child 时，parent_agent_id 就是 root 自己的 agent_id。
        // 旧模型中 root 执行不提供 agent_id，导致 child 无父级引用。
        let parent_agent_id_for_control = ctx.agent_context().agent_id.clone();

        Ok(ParentExecutionContext {
            parent_session_id: ctx.session_id().to_string(),
            parent_turn_id: parent_turn_id.to_string(),
            parent_state,
            parent_snapshot,
            parent_agent_id_for_control,
        })
    }

    pub(super) async fn launch_background(
        &self,
        params: SpawnAgentParams,
        profile: AgentProfile,
        parent: ParentExecutionContext,
        ctx: &ToolContext,
    ) -> ServiceResult<SubRunResult> {
        let prepared = self.prepare_child(&profile, &params, &parent).await?;
        let spawned = self.spawn_child(prepared, &parent, ctx).await?;
        self.emit_child_started_or_fail(&spawned).await?;

        let running_result = SubRunResult {
            status: AgentStatus::Running,
            handoff: Some(build_background_subrun_handoff(
                &spawned.child,
                &spawned.parent_session_id,
            )),
            failure: None,
        };

        let service = self.clone();
        let handle = tokio::spawn(async move {
            let started_at = Instant::now();
            let (outcome, tracker) = service.run_child_loop(&spawned).await;
            if let Err(error) = service
                .finalize_child_execution(spawned, tracker, started_at, outcome)
                .await
            {
                log::error!("failed to finalize child execution: {}", error);
            }
        });
        // 保存 JoinHandle 以便 shutdown 时 abort。
        self.runtime.lifecycle().register_subagent_task(handle);

        Ok(running_result)
    }

    async fn prepare_child(
        &self,
        profile: &AgentProfile,
        params: &SpawnAgentParams,
        parent: &ParentExecutionContext,
    ) -> ServiceResult<PreparedSubagentExecution> {
        let prepared_execution = self
            .prepare_scoped_execution(
                InvocationKind::SubRun,
                profile,
                params,
                self.snapshot_execution_surface().await,
                Some(&parent.parent_snapshot),
            )
            .await?;
        let child_storage_mode = prepared_execution
            .execution_spec
            .resolved_overrides
            .storage_mode;
        let child_task = prepared_execution
            .execution_spec
            .resolved_context_snapshot
            .task_payload
            .clone();

        Ok(PreparedSubagentExecution {
            profile: profile.clone(),
            prepared_execution,
            child_storage_mode,
            child_task,
        })
    }

    async fn spawn_child(
        &self,
        prepared: PreparedSubagentExecution,
        parent: &ParentExecutionContext,
        ctx: &ToolContext,
    ) -> ServiceResult<SpawnedSubagentExecution> {
        // 为什么强制 IndependentSession：四工具模型要求所有新 spawn 的子 agent
        // 拥有独立的 durable event log，SharedSession 已被废弃。
        let child_storage_mode = SubRunStorageMode::IndependentSession;
        let child_session_meta = self
            .runtime
            .sessions()
            .create(ctx.working_dir())
            .await
            .map_err(|error| {
                ServiceError::Conflict(format!(
                    "failed to create independent child session for parentTurn='{}': {}",
                    parent.parent_turn_id, error
                ))
            })?;
        let target_session_id = child_session_meta.session_id.clone();
        let child_session_id = Some(child_session_meta.session_id.clone());

        let child = self
            .runtime
            .agent_control
            .spawn_with_storage(
                &prepared.profile,
                target_session_id.clone(),
                child_session_id.clone(),
                parent.parent_turn_id.clone(),
                parent.parent_agent_id_for_control.clone(),
                child_storage_mode,
            )
            .await
            .map_err(|error| ServiceError::Conflict(error.to_string()))?;
        // 故意忽略：子代理状态标记失败不应阻断启动流程
        if self
            .runtime
            .agent_control
            .mark_running(&child.agent_id)
            .await
            .is_none()
        {
            log::warn!(
                "mark_running 返回 None，agent {} 可能未注册进控制树",
                child.agent_id
            );
        }
        let child_cancel = self
            .runtime
            .agent_control
            .cancel_token(&child.agent_id)
            .await
            .unwrap_or_else(CancelToken::new);
        let child_turn_id = format!("{}-child-{}", parent.parent_turn_id, uuid::Uuid::new_v4());
        let child_execution_owner =
            derive_child_execution_owner(ctx, &parent.parent_turn_id, &child);
        let child_agent = AgentEventContext::sub_run(
            child.agent_id.clone(),
            parent.parent_turn_id.clone(),
            prepared.profile.id.clone(),
            child.sub_run_id.clone(),
            child.storage_mode,
            child.child_session_id.clone(),
        );
        let child_node = build_child_session_node(
            &child,
            &parent.parent_session_id,
            &parent.parent_turn_id,
            ctx.tool_call_id().map(ToString::to_string),
        );
        parent
            .parent_state
            .upsert_child_session_node(child_node.clone())
            .map_err(|error| ServiceError::Internal(AstrError::Internal(error.to_string())))?;
        self.runtime
            .observability
            .record_child_lifecycle(ChildLifecycleStage::Spawned);
        let child_state = build_child_agent_state(
            &target_session_id,
            ctx.working_dir().to_path_buf(),
            &prepared.child_task,
        );
        let (parent_event_sink, active_sink) = self
            .build_event_sinks(parent, &target_session_id, child_storage_mode)
            .await?;

        Ok(SpawnedSubagentExecution {
            child,
            child_node,
            child_agent,
            child_turn_id,
            child_task: prepared.child_task,
            child_execution_owner,
            child_state,
            child_loop: Arc::clone(&prepared.prepared_execution.loop_),
            child_cancel,
            child_storage_mode: prepared.child_storage_mode,
            parent_session_id: parent.parent_session_id.clone(),
            parent_turn_id: parent.parent_turn_id.clone(),
            parent_state: Arc::clone(&parent.parent_state),
            parent_tool_call_id: ctx.tool_call_id().map(ToString::to_string),
            parent_event_sink,
            active_sink,
            resolved_overrides: prepared
                .prepared_execution
                .execution_spec
                .resolved_overrides,
            resolved_limits: prepared.prepared_execution.execution_spec.resolved_limits,
        })
    }

    async fn build_event_sinks(
        &self,
        parent: &ParentExecutionContext,
        target_session_id: &str,
        child_storage_mode: SubRunStorageMode,
    ) -> ServiceResult<(Arc<dyn ToolEventSink>, Arc<dyn ToolEventSink>)> {
        let parent_event_sink: Arc<dyn ToolEventSink> = Arc::new(
            SessionStateEventSink::new(Arc::clone(&parent.parent_state))
                .map_err(|error| ServiceError::Internal(AstrError::Internal(error.to_string())))?,
        );
        let active_sink: Arc<dyn ToolEventSink> =
            if matches!(child_storage_mode, SubRunStorageMode::IndependentSession) {
                let child_state = self
                    .runtime
                    .ensure_session_loaded(target_session_id)
                    .await?;
                Arc::new(SessionStateEventSink::new(child_state).map_err(|error| {
                    ServiceError::Internal(AstrError::Internal(error.to_string()))
                })?)
            } else {
                parent_event_sink.clone()
            };
        Ok((parent_event_sink, active_sink))
    }

    fn emit_child_started(&self, execution: &SpawnedSubagentExecution) -> ServiceResult<()> {
        if let Err(error) = execution.parent_event_sink.emit(build_subrun_started_event(
            &execution.parent_turn_id,
            execution.child_agent.clone(),
            &execution.child,
            execution.parent_tool_call_id.clone(),
            execution.resolved_overrides.clone(),
            execution.resolved_limits.clone(),
        )) {
            return Err(ServiceError::Internal(AstrError::Internal(format!(
                "failed to persist SubRunStarted for child agent '{}' (subRunId='{}'): {}",
                execution.child.agent_id, execution.child.sub_run_id, error
            ))));
        }

        let started_notification = build_child_session_notification(
            &execution.child_node,
            format!("child-started:{}", execution.child.sub_run_id),
            ChildSessionNotificationKind::Started,
            format!("子 Agent {} 已启动。", execution.child.agent_id),
            execution.child_node.status,
            None,
        );
        if let Err(error) = execution.parent_event_sink.emit(StorageEvent {
            turn_id: Some(execution.parent_turn_id.clone()),
            agent: execution.child_agent.clone(),
            payload: StorageEventPayload::ChildSessionNotification {
                notification: started_notification,
                timestamp: Some(chrono::Utc::now()),
            },
        }) {
            return Err(ServiceError::Internal(AstrError::Internal(format!(
                "failed to persist child-started notification for subRunId='{}': {}",
                execution.child.sub_run_id, error
            ))));
        }

        if let Err(error) = execution.active_sink.emit(StorageEvent {
            turn_id: Some(execution.child_turn_id.clone()),
            agent: execution.child_agent.clone(),
            payload: StorageEventPayload::UserMessage {
                content: execution.child_task.clone(),
                timestamp: chrono::Utc::now(),
                origin: UserMessageOrigin::User,
            },
        }) {
            return Err(ServiceError::Internal(AstrError::Internal(format!(
                "failed to persist child bootstrap user message for subRunId='{}': {}",
                execution.child.sub_run_id, error
            ))));
        }

        log::info!(
            "spawned child session: parentSession='{}', parentTurn='{}', childAgent='{}', \
             subRunId='{}', openSession='{}'",
            execution.parent_session_id,
            execution.parent_turn_id,
            execution.child.agent_id,
            execution.child.sub_run_id,
            execution.child_node.child_session_id
        );
        self.runtime
            .observability
            .record_child_lifecycle(ChildLifecycleStage::StartedPersisted);

        Ok(())
    }

    async fn emit_child_started_or_fail(
        &self,
        execution: &SpawnedSubagentExecution,
    ) -> ServiceResult<()> {
        if let Err(error) = self.emit_child_started(execution) {
            // 标记失败时已在处理另一个错误，不能覆盖
            let _ = self
                .runtime
                .agent_control
                .mark_failed(&execution.child.agent_id)
                .await;
            return Err(error);
        }
        Ok(())
    }

    pub(super) async fn run_child_loop(
        &self,
        execution: &SpawnedSubagentExecution,
    ) -> (astrcode_core::Result<TurnOutcome>, ChildExecutionTracker) {
        // TODO: 未来可能需要从 resolved_limits 中获取 max_steps 和 token_budget
        let mut tracker = ChildExecutionTracker::new(None, None);
        let outcome = execution
            .child_loop
            .run_turn_with_agent_context_and_owner(
                &execution.child_state,
                &execution.child_turn_id,
                &mut |event| {
                    tracker.observe(&event, &execution.child_cancel);
                    execution.active_sink.emit(event)
                },
                execution.child_cancel.clone(),
                execution.child_agent.clone(),
                execution.child_execution_owner.clone(),
            )
            .await;

        (outcome, tracker)
    }

    pub(super) async fn finalize_child_execution(
        &self,
        execution: SpawnedSubagentExecution,
        tracker: ChildExecutionTracker,
        started_at: Instant,
        outcome: astrcode_core::Result<TurnOutcome>,
    ) -> ServiceResult<SubRunResult> {
        let result = match outcome {
            Ok(TurnOutcome::Completed) => {
                // 四工具模型：子 agent 完成一轮后进入 Idle，而非终态。
                // 只有 close 才会进入 Terminated。Idle 状态下父 agent 可以继续 send 新消息。
                let turn_outcome = if tracker.token_limit_hit() || tracker.step_limit_hit() {
                    astrcode_core::AgentTurnOutcome::TokenExceeded
                } else {
                    astrcode_core::AgentTurnOutcome::Completed
                };
                // complete_turn 失败意味着控制平面与实际执行脱节，记录但不阻断
                if self
                    .runtime
                    .agent_control
                    .complete_turn(&execution.child.agent_id, turn_outcome)
                    .await
                    .is_none()
                {
                    log::warn!(
                        "complete_turn 返回 None，agent {} 的 lifecycle 可能与实际脱节",
                        execution.child.agent_id
                    );
                }
                let completion_fallback = if tracker.step_count() > 0 {
                    format!(
                        "子 Agent 已完成 {} \
                         步执行，但没有返回最终总结。请展开子执行查看工具和思考流。",
                        tracker.step_count()
                    )
                } else {
                    "子 Agent 已完成，但没有返回最终总结。请展开子执行查看工具和思考流。"
                        .to_string()
                };
                SubRunResult {
                    status: if tracker.token_limit_hit() || tracker.step_limit_hit() {
                        AgentStatus::TokenExceeded
                    } else {
                        AgentStatus::Completed
                    },
                    handoff: Some(build_subrun_handoff(
                        &execution.child,
                        &execution.parent_session_id,
                        tracker.last_summary(),
                        tracker.token_limit_hit(),
                        tracker.step_limit_hit(),
                        started_at.elapsed().as_millis() as u64,
                        &completion_fallback,
                    )),
                    failure: None,
                }
            },
            Ok(TurnOutcome::Cancelled) => {
                // 四工具模型：被取消（cancel token 触发）也进入 Idle，
                // 而不是直接 Terminated。父 agent 可以判断是否需要 send 或 close。
                let turn_outcome = if tracker.token_limit_hit() || tracker.step_limit_hit() {
                    astrcode_core::AgentTurnOutcome::TokenExceeded
                } else {
                    astrcode_core::AgentTurnOutcome::Cancelled
                };
                // complete_turn 失败意味着控制平面与实际执行脱节，记录但不阻断
                if self
                    .runtime
                    .agent_control
                    .complete_turn(&execution.child.agent_id, turn_outcome)
                    .await
                    .is_none()
                {
                    log::warn!(
                        "complete_turn (cancel) 返回 None，agent {} 的 lifecycle 可能与实际脱节",
                        execution.child.agent_id
                    );
                }
                let status = if tracker.token_limit_hit() || tracker.step_limit_hit() {
                    AgentStatus::TokenExceeded
                } else {
                    AgentStatus::Cancelled
                };
                let cancelled_fallback =
                    "子 Agent 已中止。请展开子执行查看已产生的工具和思考流。".to_string();
                SubRunResult {
                    status,
                    handoff: Some(build_subrun_handoff(
                        &execution.child,
                        &execution.parent_session_id,
                        tracker.last_summary(),
                        tracker.token_limit_hit(),
                        tracker.step_limit_hit(),
                        started_at.elapsed().as_millis() as u64,
                        &cancelled_fallback,
                    )),
                    failure: None,
                }
            },
            Ok(TurnOutcome::Error { message }) => {
                // 四工具模型：错误结束后也进入 Idle（带 Failed outcome），
                // 父 agent 可以选择 send 新指令重试或 close。
                if self
                    .runtime
                    .agent_control
                    .complete_turn(
                        &execution.child.agent_id,
                        astrcode_core::AgentTurnOutcome::Failed,
                    )
                    .await
                    .is_none()
                {
                    log::warn!(
                        "complete_turn (error) 返回 None，agent {} 的 lifecycle 可能与实际脱节",
                        execution.child.agent_id
                    );
                }
                let error = AstrError::Internal(message);
                SubRunResult {
                    status: AgentStatus::Failed,
                    handoff: None,
                    failure: Some(build_subrun_failure(&error)),
                }
            },
            Err(error) => {
                if self
                    .runtime
                    .agent_control
                    .complete_turn(
                        &execution.child.agent_id,
                        astrcode_core::AgentTurnOutcome::Failed,
                    )
                    .await
                    .is_none()
                {
                    log::warn!(
                        "complete_turn (fatal) 返回 None，agent {} 的 lifecycle 可能与实际脱节",
                        execution.child.agent_id
                    );
                }
                let err = AstrError::Internal(error.to_string());
                SubRunResult {
                    status: AgentStatus::Failed,
                    handoff: None,
                    failure: Some(build_subrun_failure(&err)),
                }
            },
        };

        let duration = started_at.elapsed();
        self.runtime.observability.record_subrun_execution(
            duration,
            &result.status,
            execution.child_storage_mode,
            tracker.step_count(),
            tracker.estimated_tokens_used(),
        );

        execution
            .parent_event_sink
            .emit(build_subrun_finished_event(
                &execution.parent_turn_id,
                execution.child_agent.clone(),
                &execution.child,
                execution.parent_tool_call_id,
                result.clone(),
                tracker.step_count(),
                tracker.estimated_tokens_used(),
            ))?;

        let delivery = project_child_terminal_delivery(&result);
        let mut terminal_node = execution.child_node.clone();
        terminal_node.status = delivery.status;
        terminal_node.status_source = ChildSessionStatusSource::Durable;
        execution
            .parent_state
            .upsert_child_session_node(terminal_node.clone())
            .map_err(|error| ServiceError::Internal(AstrError::Internal(error.to_string())))?;
        let terminal_notification = build_child_session_notification(
            &terminal_node,
            format!(
                "child-terminal:{}:{}",
                execution.child.sub_run_id,
                status_label(result.status)
            ),
            delivery.kind,
            delivery.summary,
            delivery.status,
            delivery.final_reply_excerpt,
        );
        execution
            .parent_event_sink
            .emit(StorageEvent {
                turn_id: Some(execution.parent_turn_id.clone()),
                agent: execution.child_agent.clone(),
                payload: StorageEventPayload::ChildSessionNotification {
                    notification: terminal_notification.clone(),
                    timestamp: Some(chrono::Utc::now()),
                },
            })
            .map_err(|error| {
                ServiceError::Internal(AstrError::Internal(format!(
                    "failed to persist child terminal notification for subRunId='{}': {}",
                    execution.child.sub_run_id, error
                )))
            })?;
        self.runtime
            .observability
            .record_child_lifecycle(ChildLifecycleStage::TerminalPersisted);

        self.runtime
            .agent()
            .reactivate_parent_agent_if_idle(
                &execution.parent_session_id,
                &execution.parent_turn_id,
                &terminal_notification,
            )
            .await;

        log::info!(
            "child session terminal delivery persisted: parentSession='{}', childAgent='{}', \
             subRunId='{}', outcome='{}'",
            execution.parent_session_id,
            execution.child.agent_id,
            execution.child.sub_run_id,
            child_delivery_outcome_label(&result)
        );

        Ok(result)
    }
}

fn status_label(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Pending => "pending",
        AgentStatus::Running => "running",
        AgentStatus::Completed => "completed",
        AgentStatus::Cancelled => "cancelled",
        AgentStatus::Failed => "failed",
        AgentStatus::TokenExceeded => "token_exceeded",
    }
}
