//! # Agent 路由
//!
//! 提供 Agent Profile 查询、根执行入口和子会话状态查询。

use std::path::PathBuf;

use astrcode_protocol::http::{
    AgentExecuteRequestDto, AgentExecuteResponseDto, AgentProfileDto, SubRunStatusDto,
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
    mapper::{from_subagent_context_overrides_dto, to_agent_profile_dto, to_subrun_status_dto},
    routes::sessions,
};

pub(crate) async fn list_agents(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<AgentProfileDto>>, ApiError> {
    require_auth(&state, &headers, None)?;
    let profiles = state
        .service
        .execution()
        .list_profiles()
        .into_iter()
        .map(to_agent_profile_dto)
        .collect::<Vec<_>>();
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
        .service
        .execution()
        .execute_root_agent(
            agent_id.clone(),
            request.task,
            request.context,
            from_subagent_context_overrides_dto(request.context_overrides),
            working_dir,
        )
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
            session_id: Some(accepted.session_id),
            turn_id: Some(accepted.turn_id),
            agent_id: accepted.agent_id,
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
    let snapshot = state
        .service
        .execution()
        .get_subrun_status(&session_id, &sub_run_id)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(to_subrun_status_dto(snapshot)))
}

/// 关闭指定 agent 及其子树。
///
/// 替代旧的 `cancel_subrun` 路由。新路由按 agent_id 而非 sub_run_id 定位，
/// 始终级联关闭子树。
pub(crate) async fn close_agent(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((session_id, agent_id)): Path<(String, String)>,
) -> Result<Json<CloseAgentResponse>, ApiError> {
    require_auth(&state, &headers, None)?;
    let session_id = sessions::validate_session_path_id(&session_id)?;
    let closed_ids = state
        .service
        .execution()
        .close_agent_subtree(&session_id, &agent_id, true)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(CloseAgentResponse {
        closed_agent_ids: closed_ids,
    }))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CloseAgentResponse {
    closed_agent_ids: Vec<String>,
}
