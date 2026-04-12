//! 子 Agent 会话恢复：复用同一 child session 继续协作。
//!
//! 当 `send` 目标为 Idle 子 agent 时，routing 层调用 `resume_child_session`
//! 来恢复该子 agent 的执行——这属于 execution 边界（创建新 turn），
//! 不属于 agent 编排边界（决定"发给谁"）。

use std::{sync::Arc, time::Instant};

use astrcode_core::{
    AgentEventContext, AgentLifecycleStatus, AstrError, CancelToken, ChildSessionLineageKind,
    ChildSessionNotificationKind, InvocationKind, LineageSnapshot, ResolvedExecutionLimitsSnapshot,
    ResolvedSubagentContextOverrides, SpawnAgentParams, StorageEvent, StorageEventPayload,
    SubRunHandle, SubRunResult, SubRunStorageMode, ToolContext, ToolEventSink, UserMessageOrigin,
};
use astrcode_runtime_execution::{
    LineageMismatchKind, build_background_subrun_handoff, build_child_session_notification,
    build_resumed_child_agent_state, build_subrun_started_event, derive_child_execution_owner,
};
use astrcode_runtime_session::SessionStateEventSink;

use super::{root::AgentExecutionServiceHandle, subagent::SpawnedSubagentExecution};
use crate::service::{ServiceError, ServiceResult};

impl AgentExecutionServiceHandle {
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

        let child = self
            .runtime
            .agent_control
            .get(agent_id)
            .await
            .ok_or_else(|| ServiceError::InvalidInput(format!("agent '{agent_id}' not found")))?;

        if !child.lifecycle.occupies_slot() && !child.lifecycle.is_final() {
            // Idle 但不在 Terminated 状态——可以通过 resume 重新激活
        } else if child.lifecycle.is_final() {
            return Err(ServiceError::InvalidInput(format!(
                "agent '{}' is terminated and cannot be resumed",
                agent_id
            )));
        } else if child.lifecycle.occupies_slot() {
            return Err(ServiceError::InvalidInput(format!(
                "agent '{}' is still occupying a slot (current: {:?})",
                agent_id, child.lifecycle
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
            None,
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
        child_node.status = AgentLifecycleStatus::Running;
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
            AgentLifecycleStatus::Running,
            None,
        );
        let _ = parent_event_sink
            .emit(StorageEvent {
                turn_id: Some(parent.parent_turn_id.clone()),
                agent: child_agent.clone(),
                payload: StorageEventPayload::ChildSessionNotification {
                    notification: resumed_notification,
                    timestamp: Some(chrono::Utc::now()),
                },
            })
            .map_err(|e| {
                log::warn!("resume resumed_notification emit 失败: {e}");
                e
            });

        let resume_message = message.unwrap_or_else(|| "继续执行".to_string());
        let _ = active_sink
            .emit(StorageEvent {
                turn_id: Some(child_turn_id.clone()),
                agent: child_agent.clone(),
                payload: StorageEventPayload::UserMessage {
                    content: resume_message.clone(),
                    timestamp: chrono::Utc::now(),
                    origin: UserMessageOrigin::User,
                },
            })
            .map_err(|e| {
                log::warn!("resume bootstrap user message emit 失败: {e}");
                e
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
            self.prepare_scoped_execution(
                InvocationKind::SubRun,
                &profile,
                &SpawnAgentParams {
                    r#type: Some(profile.id.clone()),
                    description: "resume".to_string(),
                    prompt: resume_message.clone(),
                    context: None,
                },
                self.snapshot_execution_surface().await,
                Some(&parent.parent_snapshot),
            )
            .await?
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
            lifecycle: AgentLifecycleStatus::Running,
            last_turn_outcome: None,
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
            child.parent_sub_run_id.clone(),
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
        let _ = parent_event_sink
            .emit(StorageEvent {
                turn_id: Some(parent_turn_id.to_string()),
                agent,
                payload: StorageEventPayload::Error {
                    message: format!("{code}: {message}"),
                    timestamp: Some(chrono::Utc::now()),
                },
            })
            .map_err(|e| {
                log::warn!("resume failure event emit 失败: {e}");
                e
            });
        ServiceError::Conflict(format!("{code}: {message}"))
    }
}
