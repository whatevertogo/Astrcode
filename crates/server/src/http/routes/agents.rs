//! # Agent 路由
//!
//! 提供 Agent Profile 查询、根执行入口和子会话状态查询。
//! 所有路由通过 `App` 的稳定用例接口访问，不直接依赖 kernel 内部结构。

use std::path::PathBuf;

use astrcode_application::{AgentExecuteSummary, RootExecutionRequest};
use astrcode_protocol::http::{
    AgentExecuteRequestDto, AgentExecuteResponseDto, AgentProfileDto, ExecutionControlDto,
    SubRunStatusDto,
};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use serde::Serialize;

use crate::{
    ApiError, AppState,
    auth::require_auth,
    mapper::{
        from_subagent_context_overrides_dto, to_agent_execute_response_dto, to_subrun_status_dto,
    },
    routes::sessions,
};

fn to_execution_control(
    control: Option<ExecutionControlDto>,
) -> Option<astrcode_application::ExecutionControl> {
    control
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
    let summary: AgentExecuteSummary = state
        .app
        .execute_root_agent_summary(RootExecutionRequest {
            agent_id: agent_id.clone(),
            working_dir: working_dir.to_string_lossy().to_string(),
            task: request.task,
            context: request.context,
            control: to_execution_control(request.control),
            context_overrides: from_subagent_context_overrides_dto(request.context_overrides),
        })
        .await
        .map_err(ApiError::from)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(to_agent_execute_response_dto(summary)),
    ))
}

pub(crate) async fn get_subrun_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((session_id, sub_run_id)): Path<(String, String)>,
) -> Result<Json<SubRunStatusDto>, ApiError> {
    require_auth(&state, &headers, None)?;
    let session_id = sessions::validate_session_path_id(&session_id)?;
    let summary = state
        .app
        .get_subrun_status_summary(&session_id, &sub_run_id)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(to_subrun_status_dto(summary)))
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CloseAgentResponse {
    closed_agent_ids: Vec<String>,
}
