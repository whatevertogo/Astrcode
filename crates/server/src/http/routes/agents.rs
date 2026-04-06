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

use crate::{
    ApiError, AppState,
    auth::require_auth,
    mapper::{from_subagent_context_overrides_dto, to_agent_profile_dto, to_subrun_status_dto},
    routes::sessions::validate_session_path_id,
};

pub(crate) async fn list_agents(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<AgentProfileDto>>, ApiError> {
    require_auth(&state, &headers, None)?;
    let profiles = state
        .service
        .agent_execution_service()
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
    let working_dir = match request.working_dir {
        Some(working_dir) => PathBuf::from(working_dir),
        None => std::env::current_dir().map_err(|error| ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: format!("failed to resolve current working directory: {error}"),
        })?,
    };
    let accepted = state
        .service
        .agent_execution_service()
        .execute_root_agent(
            agent_id.clone(),
            request.task,
            request.context,
            request.max_steps,
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
            agent_id: Some(accepted.agent_id),
        }),
    ))
}

pub(crate) async fn get_subrun_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((session_id, sub_run_id)): Path<(String, String)>,
) -> Result<Json<SubRunStatusDto>, ApiError> {
    require_auth(&state, &headers, None)?;
    let session_id = validate_session_path_id(&session_id)?;
    let snapshot = state
        .service
        .agent_execution_service()
        .get_subrun_status(&session_id, &sub_run_id)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(to_subrun_status_dto(snapshot)))
}

pub(crate) async fn cancel_subrun(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((session_id, sub_run_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    require_auth(&state, &headers, None)?;
    let session_id = validate_session_path_id(&session_id)?;
    let sub_run_id = validate_subrun_path_id(&sub_run_id)?;
    state
        .service
        .agent_execution_service()
        .cancel_subrun(&session_id, &sub_run_id)
        .await
        .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

fn validate_subrun_path_id(raw_sub_run_id: &str) -> Result<String, ApiError> {
    let trimmed = raw_sub_run_id.trim();
    let is_valid = !trimmed.is_empty()
        && trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');

    if is_valid {
        Ok(trimmed.to_string())
    } else {
        Err(ApiError {
            status: StatusCode::BAD_REQUEST,
            message: format!("invalid sub-run id: {raw_sub_run_id}"),
        })
    }
}
