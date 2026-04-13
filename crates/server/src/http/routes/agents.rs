//! # Agent 路由
//!
//! 提供 Agent Profile 查询、根执行入口和子会话状态查询。
//! 所有路由通过 `App` 的稳定用例接口访问，不直接依赖 kernel 内部结构。

use std::path::PathBuf;

use astrcode_kernel::SubRunStatusView;
use astrcode_protocol::http::{
    AgentExecuteRequestDto, AgentExecuteResponseDto, AgentLifecycleDto, AgentProfileDto,
    AgentTurnOutcomeDto, ExecutionControlDto, SubRunStatusDto, SubRunStatusSourceDto,
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
        resolved_limits: None,
    }))
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
        resolved_limits: None,
    }
}

fn to_lifecycle_dto(status: astrcode_core::AgentLifecycleStatus) -> AgentLifecycleDto {
    use astrcode_core::AgentLifecycleStatus;
    match status {
        AgentLifecycleStatus::Pending => AgentLifecycleDto::Pending,
        AgentLifecycleStatus::Running => AgentLifecycleDto::Running,
        AgentLifecycleStatus::Idle => AgentLifecycleDto::Idle,
        AgentLifecycleStatus::Terminated => AgentLifecycleDto::Terminated,
    }
}

fn to_turn_outcome_dto(outcome: astrcode_core::AgentTurnOutcome) -> AgentTurnOutcomeDto {
    match outcome {
        astrcode_core::AgentTurnOutcome::Completed => AgentTurnOutcomeDto::Completed,
        astrcode_core::AgentTurnOutcome::Cancelled => AgentTurnOutcomeDto::Cancelled,
        astrcode_core::AgentTurnOutcome::TokenExceeded => AgentTurnOutcomeDto::TokenExceeded,
        astrcode_core::AgentTurnOutcome::Failed => AgentTurnOutcomeDto::Failed,
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CloseAgentResponse {
    closed_agent_ids: Vec<String>,
}
