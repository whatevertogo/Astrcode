use std::{sync::Arc, time::Instant};

use astrcode_core::{
    AgentEventContext, AgentProfile, AgentState, AstrError, CancelToken, ExecutionOwner,
    InvocationKind, ResolvedExecutionLimitsSnapshot, ResolvedSubagentContextOverrides,
    SpawnAgentParams, StorageEvent, SubRunHandle, SubRunOutcome, SubRunResult, SubRunStorageMode,
    ToolContext, ToolEventSink, UserMessageOrigin,
};
use astrcode_runtime_agent_loop::{AgentLoop, ChildExecutionTracker, TurnOutcome};
use astrcode_runtime_execution::{
    PreparedAgentExecution, build_background_subrun_handoff, build_child_agent_state,
    build_subrun_failure, build_subrun_finished_event, build_subrun_handoff,
    build_subrun_started_event, derive_child_execution_owner, ensure_subagent_mode,
};
use astrcode_runtime_session::{SessionState, SessionStateEventSink};

use super::root::AgentExecutionServiceHandle;
use crate::service::{ServiceError, ServiceResult};

struct ParentExecutionContext {
    parent_turn_id: String,
    parent_state: Arc<SessionState>,
    parent_snapshot: AgentState,
    parent_agent_id_for_control: Option<String>,
}

struct PreparedSubagentExecution {
    profile: AgentProfile,
    prepared_execution: PreparedAgentExecution<Arc<AgentLoop>>,
    child_storage_mode: SubRunStorageMode,
    child_task: String,
}

struct SpawnedSubagentExecution {
    child: SubRunHandle,
    child_agent: AgentEventContext,
    child_turn_id: String,
    child_task: String,
    child_execution_owner: ExecutionOwner,
    child_state: AgentState,
    child_loop: Arc<AgentLoop>,
    child_cancel: CancelToken,
    child_storage_mode: SubRunStorageMode,
    parent_turn_id: String,
    parent_tool_call_id: Option<String>,
    parent_event_sink: Arc<dyn ToolEventSink>,
    active_sink: Arc<dyn ToolEventSink>,
    resolved_overrides: ResolvedSubagentContextOverrides,
    resolved_limits: ResolvedExecutionLimitsSnapshot,
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

    async fn resolve_profile(
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

    async fn resolve_parent_execution(
        &self,
        ctx: &ToolContext,
    ) -> ServiceResult<ParentExecutionContext> {
        let parent_turn_id = ctx.turn_id().ok_or_else(|| {
            ServiceError::InvalidInput("spawnAgent requires a parent turn id".to_string())
        })?;
        let parent_state = self.runtime.ensure_session_loaded(ctx.session_id()).await?;
        let parent_snapshot = parent_state
            .snapshot_projected_state()
            .map_err(|error| ServiceError::Internal(AstrError::Internal(error.to_string())))?;
        let parent_agent_id_for_control = if matches!(
            ctx.agent_context().invocation_kind,
            Some(InvocationKind::RootExecution)
        ) {
            None
        } else {
            ctx.agent_context().agent_id.clone()
        };

        Ok(ParentExecutionContext {
            parent_turn_id: parent_turn_id.to_string(),
            parent_state,
            parent_snapshot,
            parent_agent_id_for_control,
        })
    }

    async fn launch_background(
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
            status: SubRunOutcome::Running,
            handoff: Some(build_background_subrun_handoff(&spawned.child)),
            failure: None,
        };

        let service = self.clone();
        let handle = tokio::spawn(async move {
            let started_at = Instant::now();
            let (outcome, tracker) = service.run_child_loop(&spawned).await;
            // 故意忽略：spawned task 中无法传播错误，失败已通过内部日志记录
            let _ = service
                .finalize_child_execution(spawned, tracker, started_at, outcome)
                .await;
        });
        // 保存 JoinHandle 以便 shutdown 时 abort。
        astrcode_core::support::with_lock_recovery(
            &self.runtime.active_subagent_handles,
            "RuntimeService.active_subagent_handles",
            |guard| guard.push(handle),
        );

        Ok(running_result)
    }

    async fn prepare_child(
        &self,
        profile: &AgentProfile,
        params: &SpawnAgentParams,
        parent: &ParentExecutionContext,
    ) -> ServiceResult<PreparedSubagentExecution> {
        let prepared_execution = self.prepare_scoped_execution(
            InvocationKind::SubRun,
            profile,
            params,
            self.snapshot_execution_surface().await,
            Some(&parent.parent_snapshot),
        )?;
        let child_storage_mode = prepared_execution
            .execution_spec
            .resolved_overrides
            .storage_mode;
        let child_task = prepared_execution
            .execution_spec
            .resolved_context_snapshot
            .composed_task
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
        let child_session_meta = match prepared.child_storage_mode {
            SubRunStorageMode::SharedSession => None,
            SubRunStorageMode::IndependentSession => Some(
                self.runtime
                    .sessions()
                    .create(ctx.working_dir())
                    .await
                    .map_err(|error| ServiceError::Conflict(error.to_string()))?,
            ),
        };
        let target_session_id = child_session_meta
            .as_ref()
            .map(|meta| meta.session_id.clone())
            .unwrap_or_else(|| ctx.session_id().to_string());
        let child_session_id = child_session_meta
            .as_ref()
            .map(|meta| meta.session_id.clone());

        let child = self
            .runtime
            .agent_control
            .spawn_with_storage(
                &prepared.profile,
                target_session_id.clone(),
                child_session_id.clone(),
                Some(parent.parent_turn_id.clone()),
                parent.parent_agent_id_for_control.clone(),
                prepared.child_storage_mode,
            )
            .await
            .map_err(|error| ServiceError::Conflict(error.to_string()))?;
        // 故意忽略：子代理状态标记失败不应阻断启动流程
        let _ = self
            .runtime
            .agent_control
            .mark_running(&child.agent_id)
            .await;
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
        let child_state = build_child_agent_state(
            &target_session_id,
            ctx.working_dir().to_path_buf(),
            &prepared.child_task,
        );
        let (parent_event_sink, active_sink) = self
            .build_event_sinks(parent, &target_session_id, prepared.child_storage_mode)
            .await?;

        Ok(SpawnedSubagentExecution {
            child,
            child_agent,
            child_turn_id,
            child_task: prepared.child_task,
            child_execution_owner,
            child_state,
            child_loop: Arc::clone(&prepared.prepared_execution.loop_),
            child_cancel,
            child_storage_mode: prepared.child_storage_mode,
            parent_turn_id: parent.parent_turn_id.clone(),
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
            return Err(ServiceError::Internal(error));
        }

        if let Err(error) = execution.active_sink.emit(StorageEvent::UserMessage {
            turn_id: Some(execution.child_turn_id.clone()),
            agent: execution.child_agent.clone(),
            content: execution.child_task.clone(),
            timestamp: chrono::Utc::now(),
            origin: UserMessageOrigin::User,
        }) {
            return Err(ServiceError::Internal(error));
        }

        Ok(())
    }

    async fn emit_child_started_or_fail(
        &self,
        execution: &SpawnedSubagentExecution,
    ) -> ServiceResult<()> {
        if let Err(error) = self.emit_child_started(execution) {
            // 故意忽略：标记失败时已在处理另一个错误，不能覆盖
            let _ = self
                .runtime
                .agent_control
                .mark_failed(&execution.child.agent_id)
                .await;
            return Err(error);
        }
        Ok(())
    }

    async fn run_child_loop(
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

    async fn finalize_child_execution(
        &self,
        execution: SpawnedSubagentExecution,
        tracker: ChildExecutionTracker,
        started_at: Instant,
        outcome: astrcode_core::Result<TurnOutcome>,
    ) -> ServiceResult<SubRunResult> {
        let result = match outcome {
            Ok(TurnOutcome::Completed) => {
                // 故意忽略：子代理状态更新失败不应覆盖执行结果
                let _ = self
                    .runtime
                    .agent_control
                    .mark_completed(&execution.child.agent_id)
                    .await;
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
                        SubRunOutcome::TokenExceeded
                    } else {
                        SubRunOutcome::Completed
                    },
                    handoff: Some(build_subrun_handoff(
                        &execution.child,
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
                // 故意忽略：取消子代理失败不应阻断结果处理
                let _ = self
                    .runtime
                    .agent_control
                    .cancel(&execution.child.agent_id)
                    .await;
                let status = if tracker.token_limit_hit() || tracker.step_limit_hit() {
                    SubRunOutcome::TokenExceeded
                } else {
                    SubRunOutcome::Aborted
                };
                let cancelled_fallback =
                    "子 Agent 已中止。请展开子执行查看已产生的工具和思考流。".to_string();
                SubRunResult {
                    status,
                    handoff: Some(build_subrun_handoff(
                        &execution.child,
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
                // 故意忽略：子代理状态更新失败不应覆盖执行结果
                let _ = self
                    .runtime
                    .agent_control
                    .mark_failed(&execution.child.agent_id)
                    .await;
                let error = AstrError::Internal(message);
                SubRunResult {
                    status: SubRunOutcome::Failed,
                    handoff: None,
                    failure: Some(build_subrun_failure(&error)),
                }
            },
            Err(error) => {
                let _ = self
                    .runtime
                    .agent_control
                    .mark_failed(&execution.child.agent_id)
                    .await;
                SubRunResult {
                    status: SubRunOutcome::Failed,
                    handoff: None,
                    failure: Some(build_subrun_failure(&error)),
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
                execution.child_agent,
                &execution.child,
                execution.parent_tool_call_id,
                result.clone(),
                tracker.step_count(),
                tracker.estimated_tokens_used(),
            ))?;

        Ok(result)
    }
}
