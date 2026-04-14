//! # Agent 路由
//!
//! 提供 Agent Profile 查询、根执行入口和子会话状态查询。
//! 所有路由通过 `App` 的稳定用例接口访问，不直接依赖 kernel 内部结构。

use std::path::PathBuf;

use astrcode_application::{
    AgentEventContext, AgentLifecycleStatus, AgentTurnOutcome, InvocationKind,
    ResolvedExecutionLimitsSnapshot, ResolvedSubagentContextOverrides, StorageEventPayload,
    StoredEvent, SubRunFailure, SubRunFailureCode, SubRunHandoff, SubRunResult, SubRunStatusView,
};
use astrcode_protocol::http::{
    AgentExecuteRequestDto, AgentExecuteResponseDto, AgentLifecycleDto, AgentProfileDto,
    AgentTurnOutcomeDto, ArtifactRefDto, ExecutionControlDto, ResolvedExecutionLimitsDto,
    ResolvedSubagentContextOverridesDto, SubRunFailureCodeDto, SubRunFailureDto, SubRunHandoffDto,
    SubRunOutcomeDto, SubRunResultDto, SubRunStatusDto, SubRunStatusSourceDto,
    SubRunStorageModeDto,
};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use serde::Serialize;

use crate::{ApiError, AppState, auth::require_auth, routes::sessions};

fn to_execution_control(
    control: Option<ExecutionControlDto>,
) -> Option<astrcode_application::ExecutionControl> {
    control.map(|control| astrcode_application::ExecutionControl {
        max_steps: control.max_steps,
        manual_compact: control.manual_compact,
    })
}

pub(crate) async fn list_agents(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<AgentProfileDto>>, ApiError> {
    require_auth(&state, &headers, None)?;
    let profiles = state
        .app
        .list_global_agent_profiles()
        .map_err(ApiError::from)?
        .into_iter()
        .map(|profile| {
            crate::mapper::to_agent_profile_dto(crate::mapper::AgentProfileSummary {
                id: profile.id,
                name: profile.name,
                description: profile.description,
                mode: profile.mode,
                allowed_tools: profile.allowed_tools,
                disallowed_tools: profile.disallowed_tools,
            })
        })
        .collect();
    Ok(Json(profiles))
}

pub(crate) async fn execute_agent(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
    Json(request): Json<AgentExecuteRequestDto>,
) -> Result<(StatusCode, Json<AgentExecuteResponseDto>), ApiError> {
    require_auth(&state, &headers, None)?;
    let working_dir = request
        .working_dir
        .map(PathBuf::from)
        .ok_or_else(|| ApiError::bad_request("workingDir is required".to_string()))?;
    let accepted = state
        .app
        .execute_root_agent(astrcode_application::RootExecutionRequest {
            agent_id: agent_id.clone(),
            working_dir: working_dir.to_string_lossy().to_string(),
            task: request.task,
            context: request.context,
            control: to_execution_control(request.control),
            context_overrides: crate::mapper::from_subagent_context_overrides_dto(
                request.context_overrides,
            ),
        })
        .await
        .map_err(ApiError::from)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(AgentExecuteResponseDto {
            accepted: true,
            message: format!(
                "agent '{}' execution accepted; subscribe to /api/sessions/{}/events for progress",
                agent_id, accepted.session_id
            ),
            session_id: Some(accepted.session_id.to_string()),
            turn_id: Some(accepted.turn_id.to_string()),
            agent_id: accepted.agent_id.map(|value| value.to_string()),
        }),
    ))
}

pub(crate) async fn get_subrun_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((session_id, sub_run_id)): Path<(String, String)>,
) -> Result<Json<SubRunStatusDto>, ApiError> {
    require_auth(&state, &headers, None)?;
    let session_id = sessions::validate_session_path_id(&session_id)?;

    // 先尝试通过 agent_id 查询稳定视图
    if let Some(view) = state
        .app
        .get_subrun_status(&sub_run_id)
        .await
        .map_err(ApiError::from)?
    {
        return Ok(Json(to_subrun_status_dto(view, session_id)));
    }

    // 再尝试通过 session 查找根 agent
    if let Some(view) = state
        .app
        .get_root_agent_status(&session_id)
        .await
        .map_err(ApiError::from)?
    {
        if view.sub_run_id == sub_run_id {
            return Ok(Json(to_subrun_status_dto(view, session_id)));
        }
        return Err(ApiError {
            status: StatusCode::NOT_FOUND,
            message: format!(
                "subrun '{}' not found in session '{}'",
                sub_run_id, session_id
            ),
        });
    }

    if let Some(view) = durable_subrun_status(&state, &session_id, &sub_run_id).await? {
        return Ok(Json(view));
    }

    // 兜底：返回默认值（兼容无 agent 的 session）
    Ok(Json(SubRunStatusDto {
        sub_run_id,
        tool_call_id: None,
        source: SubRunStatusSourceDto::Live,
        agent_id: "root-agent".to_string(),
        agent_profile: "default".to_string(),
        session_id,
        child_session_id: None,
        depth: 0,
        parent_agent_id: None,
        parent_sub_run_id: None,
        storage_mode: SubRunStorageModeDto::IndependentSession,
        lifecycle: AgentLifecycleDto::Idle,
        last_turn_outcome: None,
        result: None,
        step_count: None,
        estimated_tokens: None,
        resolved_overrides: None,
        resolved_limits: Some(ResolvedExecutionLimitsDto {
            allowed_tools: Vec::new(),
            max_steps: None,
        }),
    }))
}

async fn durable_subrun_status(
    state: &AppState,
    parent_session_id: &str,
    requested_subrun_id: &str,
) -> Result<Option<SubRunStatusDto>, ApiError> {
    let child_sessions = state
        .app
        .list_sessions()
        .await
        .map_err(ApiError::from)?
        .into_iter()
        .filter(|meta| meta.parent_session_id.as_deref() == Some(parent_session_id))
        .collect::<Vec<_>>();

    for child_session in child_sessions {
        let stored_events = state
            .app
            .session_stored_events(&child_session.session_id)
            .await
            .map_err(ApiError::from)?;
        if let Some(snapshot) = project_durable_subrun_status(
            parent_session_id,
            &child_session.session_id,
            requested_subrun_id,
            &stored_events,
        ) {
            return Ok(Some(snapshot));
        }
    }

    Ok(None)
}

/// 关闭指定 agent 及其子树。
///
/// 通过 `App` 的稳定控制合同执行，不直接访问 kernel agent_tree。
pub(crate) async fn close_agent(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((session_id, agent_id)): Path<(String, String)>,
) -> Result<Json<CloseAgentResponse>, ApiError> {
    require_auth(&state, &headers, None)?;
    let session_id = sessions::validate_session_path_id(&session_id)?;
    let result = state
        .app
        .close_agent(&session_id, &agent_id)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(CloseAgentResponse {
        closed_agent_ids: result.closed_agent_ids,
    }))
}

/// 将稳定视图转换为协议 DTO。
fn to_subrun_status_dto(view: SubRunStatusView, session_id: String) -> SubRunStatusDto {
    SubRunStatusDto {
        sub_run_id: view.sub_run_id,
        tool_call_id: None,
        source: SubRunStatusSourceDto::Live,
        agent_id: view.agent_id,
        agent_profile: view.agent_profile,
        session_id,
        child_session_id: view.child_session_id,
        depth: view.depth,
        parent_agent_id: view.parent_agent_id,
        parent_sub_run_id: None,
        storage_mode: SubRunStorageModeDto::IndependentSession,
        lifecycle: to_lifecycle_dto(view.lifecycle),
        last_turn_outcome: view.last_turn_outcome.map(to_turn_outcome_dto),
        result: None,
        step_count: None,
        estimated_tokens: None,
        resolved_overrides: None,
        resolved_limits: Some(ResolvedExecutionLimitsDto {
            allowed_tools: view.resolved_limits.allowed_tools,
            max_steps: view.resolved_limits.max_steps,
        }),
    }
}

#[derive(Debug, Clone)]
struct DurableSubRunStatusProjection {
    sub_run_id: String,
    tool_call_id: Option<String>,
    agent_id: String,
    agent_profile: String,
    child_session_id: String,
    depth: usize,
    parent_agent_id: Option<String>,
    parent_sub_run_id: Option<String>,
    lifecycle: AgentLifecycleStatus,
    last_turn_outcome: Option<AgentTurnOutcome>,
    result: Option<SubRunResult>,
    step_count: Option<u32>,
    estimated_tokens: Option<u64>,
    resolved_overrides: Option<ResolvedSubagentContextOverrides>,
    resolved_limits: ResolvedExecutionLimitsSnapshot,
}

fn project_durable_subrun_status(
    parent_session_id: &str,
    child_session_id: &str,
    requested_subrun_id: &str,
    stored_events: &[StoredEvent],
) -> Option<SubRunStatusDto> {
    let mut projection: Option<DurableSubRunStatusProjection> = None;

    for stored in stored_events {
        let agent = &stored.event.agent;
        if !matches_requested_subrun(agent, requested_subrun_id) {
            continue;
        }

        match &stored.event.payload {
            StorageEventPayload::SubRunStarted {
                tool_call_id,
                resolved_overrides,
                resolved_limits,
                ..
            } => {
                projection = Some(DurableSubRunStatusProjection {
                    sub_run_id: agent
                        .sub_run_id
                        .clone()
                        .unwrap_or_else(|| requested_subrun_id.to_string()),
                    tool_call_id: tool_call_id.clone(),
                    agent_id: agent
                        .agent_id
                        .clone()
                        .unwrap_or_else(|| requested_subrun_id.to_string()),
                    agent_profile: agent
                        .agent_profile
                        .clone()
                        .unwrap_or_else(|| "unknown".to_string()),
                    child_session_id: child_session_id.to_string(),
                    depth: 1,
                    parent_agent_id: None,
                    parent_sub_run_id: agent.parent_sub_run_id.clone(),
                    lifecycle: AgentLifecycleStatus::Running,
                    last_turn_outcome: None,
                    result: None,
                    step_count: None,
                    estimated_tokens: None,
                    resolved_overrides: Some(resolved_overrides.clone()),
                    resolved_limits: resolved_limits.clone(),
                });
            },
            StorageEventPayload::SubRunFinished {
                tool_call_id,
                result,
                step_count,
                estimated_tokens,
                ..
            } => {
                let entry = projection.get_or_insert_with(|| DurableSubRunStatusProjection {
                    sub_run_id: agent
                        .sub_run_id
                        .clone()
                        .unwrap_or_else(|| requested_subrun_id.to_string()),
                    tool_call_id: None,
                    agent_id: agent
                        .agent_id
                        .clone()
                        .unwrap_or_else(|| requested_subrun_id.to_string()),
                    agent_profile: agent
                        .agent_profile
                        .clone()
                        .unwrap_or_else(|| "unknown".to_string()),
                    child_session_id: child_session_id.to_string(),
                    depth: 1,
                    parent_agent_id: None,
                    parent_sub_run_id: agent.parent_sub_run_id.clone(),
                    lifecycle: result.lifecycle,
                    last_turn_outcome: result.last_turn_outcome,
                    result: None,
                    step_count: None,
                    estimated_tokens: None,
                    resolved_overrides: None,
                    resolved_limits: ResolvedExecutionLimitsSnapshot::default(),
                });
                entry.tool_call_id = tool_call_id.clone().or_else(|| entry.tool_call_id.clone());
                entry.lifecycle = result.lifecycle;
                entry.last_turn_outcome = result.last_turn_outcome;
                entry.result = Some(result.clone());
                entry.step_count = Some(*step_count);
                entry.estimated_tokens = Some(*estimated_tokens);
            },
            _ => {},
        }
    }

    projection.map(|projection| SubRunStatusDto {
        sub_run_id: projection.sub_run_id,
        tool_call_id: projection.tool_call_id,
        source: SubRunStatusSourceDto::Durable,
        agent_id: projection.agent_id,
        agent_profile: projection.agent_profile,
        session_id: parent_session_id.to_string(),
        child_session_id: Some(projection.child_session_id),
        depth: projection.depth,
        parent_agent_id: projection.parent_agent_id,
        parent_sub_run_id: projection.parent_sub_run_id,
        storage_mode: SubRunStorageModeDto::IndependentSession,
        lifecycle: to_lifecycle_dto(projection.lifecycle),
        last_turn_outcome: projection.last_turn_outcome.map(to_turn_outcome_dto),
        result: projection.result.map(to_subrun_result_dto),
        step_count: projection.step_count,
        estimated_tokens: projection.estimated_tokens,
        resolved_overrides: projection.resolved_overrides.map(to_resolved_overrides_dto),
        resolved_limits: Some(ResolvedExecutionLimitsDto {
            allowed_tools: projection.resolved_limits.allowed_tools,
            max_steps: projection.resolved_limits.max_steps,
        }),
    })
}

fn matches_requested_subrun(agent: &AgentEventContext, requested_subrun_id: &str) -> bool {
    if agent.invocation_kind != Some(InvocationKind::SubRun) {
        return false;
    }

    agent.sub_run_id.as_deref() == Some(requested_subrun_id)
        || agent.agent_id.as_deref() == Some(requested_subrun_id)
}

fn to_resolved_overrides_dto(
    overrides: ResolvedSubagentContextOverrides,
) -> ResolvedSubagentContextOverridesDto {
    ResolvedSubagentContextOverridesDto {
        storage_mode: SubRunStorageModeDto::IndependentSession,
        inherit_system_instructions: overrides.inherit_system_instructions,
        inherit_project_instructions: overrides.inherit_project_instructions,
        inherit_working_dir: overrides.inherit_working_dir,
        inherit_policy_upper_bound: overrides.inherit_policy_upper_bound,
        inherit_cancel_token: overrides.inherit_cancel_token,
        include_compact_summary: overrides.include_compact_summary,
        include_recent_tail: overrides.include_recent_tail,
        include_recovery_refs: overrides.include_recovery_refs,
        include_parent_findings: overrides.include_parent_findings,
        fork_mode: None,
    }
}

fn to_subrun_result_dto(result: SubRunResult) -> SubRunResultDto {
    SubRunResultDto {
        status: to_subrun_outcome_dto(result.lifecycle, result.last_turn_outcome),
        handoff: result.handoff.map(to_subrun_handoff_dto),
        failure: result.failure.map(to_subrun_failure_dto),
    }
}

fn to_subrun_outcome_dto(
    lifecycle: AgentLifecycleStatus,
    last_turn_outcome: Option<AgentTurnOutcome>,
) -> SubRunOutcomeDto {
    match last_turn_outcome {
        Some(AgentTurnOutcome::Completed) => SubRunOutcomeDto::Completed,
        Some(AgentTurnOutcome::Failed) => SubRunOutcomeDto::Failed,
        Some(AgentTurnOutcome::Cancelled) => SubRunOutcomeDto::Aborted,
        Some(AgentTurnOutcome::TokenExceeded) => SubRunOutcomeDto::TokenExceeded,
        None => match lifecycle {
            AgentLifecycleStatus::Terminated => SubRunOutcomeDto::Aborted,
            _ => SubRunOutcomeDto::Running,
        },
    }
}

fn to_subrun_handoff_dto(handoff: SubRunHandoff) -> SubRunHandoffDto {
    SubRunHandoffDto {
        summary: handoff.summary,
        findings: handoff.findings,
        artifacts: handoff
            .artifacts
            .into_iter()
            .map(|artifact| ArtifactRefDto {
                kind: artifact.kind,
                id: artifact.id,
                label: artifact.label,
                session_id: artifact.session_id,
                storage_seq: artifact.storage_seq,
                uri: artifact.uri,
            })
            .collect(),
        delivery: None,
    }
}

fn to_subrun_failure_dto(failure: SubRunFailure) -> SubRunFailureDto {
    SubRunFailureDto {
        code: match failure.code {
            SubRunFailureCode::Transport => SubRunFailureCodeDto::Transport,
            SubRunFailureCode::ProviderHttp => SubRunFailureCodeDto::ProviderHttp,
            SubRunFailureCode::StreamParse => SubRunFailureCodeDto::StreamParse,
            SubRunFailureCode::Interrupted => SubRunFailureCodeDto::Interrupted,
            SubRunFailureCode::Internal => SubRunFailureCodeDto::Internal,
        },
        display_message: failure.display_message,
        technical_message: failure.technical_message,
        retryable: failure.retryable,
    }
}

fn to_lifecycle_dto(status: AgentLifecycleStatus) -> AgentLifecycleDto {
    match status {
        AgentLifecycleStatus::Pending => AgentLifecycleDto::Pending,
        AgentLifecycleStatus::Running => AgentLifecycleDto::Running,
        AgentLifecycleStatus::Idle => AgentLifecycleDto::Idle,
        AgentLifecycleStatus::Terminated => AgentLifecycleDto::Terminated,
    }
}

fn to_turn_outcome_dto(outcome: AgentTurnOutcome) -> AgentTurnOutcomeDto {
    match outcome {
        AgentTurnOutcome::Completed => AgentTurnOutcomeDto::Completed,
        AgentTurnOutcome::Cancelled => AgentTurnOutcomeDto::Cancelled,
        AgentTurnOutcome::TokenExceeded => AgentTurnOutcomeDto::TokenExceeded,
        AgentTurnOutcome::Failed => AgentTurnOutcomeDto::Failed,
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CloseAgentResponse {
    closed_agent_ids: Vec<String>,
}
