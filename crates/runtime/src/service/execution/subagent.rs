use std::{sync::Arc, time::Instant};

use astrcode_core::{
    AgentEventContext, AstrError, CancelToken, InvocationKind, StorageEvent, SubRunOutcome,
    SubRunResult, SubRunStorageMode, ToolContext, ToolEventSink, UserMessageOrigin,
};
use astrcode_runtime_agent_loop::ChildExecutionTracker;
use astrcode_runtime_execution::{
    build_child_agent_state, build_result_artifacts, build_result_findings,
    derive_child_execution_owner, ensure_subagent_mode, summarize_child_result,
};
use astrcode_runtime_session::SessionStateEventSink;

use super::root::AgentExecutionServiceHandle;
use crate::service::{ServiceError, ServiceResult};

impl AgentExecutionServiceHandle {
    pub async fn execute_subagent(
        &self,
        params: astrcode_runtime_agent_tool::RunAgentParams,
        ctx: &ToolContext,
    ) -> ServiceResult<SubRunResult> {
        let parent_turn_id = ctx.turn_id().ok_or_else(|| {
            ServiceError::InvalidInput("runAgent requires a parent turn id".to_string())
        })?;
        let event_sink = ctx.event_sink().ok_or_else(|| {
            ServiceError::InvalidInput(
                "runAgent requires a tool event sink in the current runtime".to_string(),
            )
        })?;
        let runtime = &self.runtime;
        let profiles = runtime.agent_profiles();
        let profile = profiles.get(&params.name).cloned().ok_or_else(|| {
            ServiceError::InvalidInput(format!("unknown agent profile '{}'", params.name))
        })?;
        ensure_subagent_mode(&profile)?;
        let parent_state = runtime.ensure_session_loaded(ctx.session_id()).await?;
        let parent_snapshot = parent_state
            .snapshot_projected_state()
            .map_err(|error| ServiceError::Internal(AstrError::Internal(error.to_string())))?;
        let prepared_execution = self.prepare_scoped_execution(
            InvocationKind::SubRun,
            &profile,
            &params,
            self.snapshot_execution_surface().await,
            Some(&parent_snapshot),
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
        let child_loop = Arc::clone(&prepared_execution.loop_);
        let child_session_meta = match child_storage_mode {
            SubRunStorageMode::SharedSession => None,
            SubRunStorageMode::IndependentSession => Some(
                runtime
                    .create_session(ctx.working_dir())
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

        let parent_agent_id_for_control = if matches!(
            ctx.agent_context().invocation_kind,
            Some(InvocationKind::RootExecution)
        ) {
            None
        } else {
            ctx.agent_context().agent_id.clone()
        };
        let child = runtime
            .agent_control
            .spawn_with_storage(
                &profile,
                target_session_id.clone(),
                child_session_id.clone(),
                Some(parent_turn_id.to_string()),
                parent_agent_id_for_control,
                child_storage_mode,
            )
            .await
            .map_err(|error| ServiceError::Conflict(error.to_string()))?;
        let _ = runtime.agent_control.mark_running(&child.agent_id).await;
        let child_cancel = runtime
            .agent_control
            .cancel_token(&child.agent_id)
            .await
            .unwrap_or_else(CancelToken::new);
        let child_turn_id = format!("{}-child-{}", parent_turn_id, uuid::Uuid::new_v4());
        let child_execution_owner = derive_child_execution_owner(ctx, parent_turn_id, &child);
        let child_agent = AgentEventContext::sub_run(
            child.agent_id.clone(),
            parent_turn_id.to_string(),
            profile.id.clone(),
            child.sub_run_id.clone(),
            child.storage_mode,
            child.child_session_id.clone(),
        );
        let child_state = build_child_agent_state(
            &target_session_id,
            ctx.working_dir().to_path_buf(),
            &child_task,
        );
        let parent_event_sink = event_sink.clone();
        let active_sink: Arc<dyn ToolEventSink> =
            if matches!(child.storage_mode, SubRunStorageMode::IndependentSession) {
                let child_state = runtime.ensure_session_loaded(&target_session_id).await?;
                Arc::new(SessionStateEventSink::new(child_state).map_err(|error| {
                    ServiceError::Internal(AstrError::Internal(error.to_string()))
                })?)
            } else {
                event_sink
            };

        parent_event_sink.emit(StorageEvent::SubRunStarted {
            turn_id: Some(parent_turn_id.to_string()),
            agent: child_agent.clone(),
            resolved_overrides: prepared_execution.execution_spec.resolved_overrides.clone(),
            resolved_limits: prepared_execution.execution_spec.resolved_limits.clone(),
            timestamp: Some(chrono::Utc::now()),
        })?;

        active_sink.emit(StorageEvent::UserMessage {
            turn_id: Some(child_turn_id.clone()),
            agent: child_agent.clone(),
            content: child_task,
            timestamp: chrono::Utc::now(),
            origin: UserMessageOrigin::User,
        })?;

        let mut tracker = ChildExecutionTracker::new(
            prepared_execution.execution_spec.resolved_limits.max_steps,
            prepared_execution
                .execution_spec
                .resolved_limits
                .token_budget,
        );
        let started_at = Instant::now();
        let outcome = child_loop
            .run_turn_with_agent_context_and_owner(
                &child_state,
                &child_turn_id,
                &mut |event| {
                    if ctx.cancel().is_cancelled() {
                        child_cancel.cancel();
                    }
                    tracker.observe(&event, &child_cancel);
                    active_sink.emit(event)
                },
                child_cancel.clone(),
                child_agent.clone(),
                child_execution_owner,
            )
            .await;

        let result = match outcome {
            Ok(astrcode_runtime_agent_loop::TurnOutcome::Completed) => {
                let _ = runtime.agent_control.mark_completed(&child.agent_id).await;
                SubRunResult {
                    status: if tracker.token_limit_hit() || tracker.step_limit_hit() {
                        SubRunOutcome::TokenExceeded
                    } else {
                        SubRunOutcome::Completed
                    },
                    summary: summarize_child_result(
                        tracker.last_summary(),
                        tracker.token_limit_hit(),
                        tracker.step_limit_hit(),
                        started_at.elapsed().as_millis() as u64,
                        "子 Agent 已完成任务。",
                    ),
                    artifacts: build_result_artifacts(&child),
                    findings: build_result_findings(&prepared_execution.execution_spec),
                }
            },
            Ok(astrcode_runtime_agent_loop::TurnOutcome::Cancelled) => {
                let _ = runtime.agent_control.cancel(&child.agent_id).await;
                let status = if tracker.token_limit_hit() || tracker.step_limit_hit() {
                    SubRunOutcome::TokenExceeded
                } else {
                    SubRunOutcome::Aborted
                };
                SubRunResult {
                    status,
                    summary: summarize_child_result(
                        tracker.last_summary(),
                        tracker.token_limit_hit(),
                        tracker.step_limit_hit(),
                        started_at.elapsed().as_millis() as u64,
                        "子 Agent 被中止。",
                    ),
                    artifacts: build_result_artifacts(&child),
                    findings: build_result_findings(&prepared_execution.execution_spec),
                }
            },
            Ok(astrcode_runtime_agent_loop::TurnOutcome::Error { message }) => {
                let _ = runtime.agent_control.mark_failed(&child.agent_id).await;
                SubRunResult {
                    status: SubRunOutcome::Failed {
                        error: message.clone(),
                    },
                    summary: summarize_child_result(
                        tracker.last_summary(),
                        tracker.token_limit_hit(),
                        tracker.step_limit_hit(),
                        started_at.elapsed().as_millis() as u64,
                        &format!("子 Agent 执行失败：{message}"),
                    ),
                    artifacts: build_result_artifacts(&child),
                    findings: build_result_findings(&prepared_execution.execution_spec),
                }
            },
            Err(error) => {
                let _ = runtime.agent_control.mark_failed(&child.agent_id).await;
                SubRunResult {
                    status: SubRunOutcome::Failed {
                        error: error.to_string(),
                    },
                    summary: summarize_child_result(
                        tracker.last_summary(),
                        tracker.token_limit_hit(),
                        tracker.step_limit_hit(),
                        started_at.elapsed().as_millis() as u64,
                        &format!("子 Agent 执行失败：{error}"),
                    ),
                    artifacts: build_result_artifacts(&child),
                    findings: build_result_findings(&prepared_execution.execution_spec),
                }
            },
        };

        let duration = started_at.elapsed();
        runtime.observability.record_subrun_execution(
            duration,
            &result.status,
            child_storage_mode,
            tracker.step_count(),
            tracker.estimated_tokens_used(),
        );

        parent_event_sink.emit(StorageEvent::SubRunFinished {
            turn_id: Some(parent_turn_id.to_string()),
            agent: child_agent,
            result: result.clone(),
            step_count: tracker.step_count(),
            estimated_tokens: tracker.estimated_tokens_used(),
            timestamp: Some(chrono::Utc::now()),
        })?;

        Ok(result)
    }
}
