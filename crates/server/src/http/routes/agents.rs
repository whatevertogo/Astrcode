//! # Agent 路由
//!
//! 提供 Agent Profile 查询、根执行入口和子会话状态查询。

use std::path::PathBuf;

use astrcode_protocol::http::{
    AgentExecuteRequestDto, AgentExecuteResponseDto, AgentLifecycleDto, AgentProfileDto,
    SubRunStatusDto, SubRunStatusSourceDto, SubRunStorageModeDto,
};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use serde::Serialize;

use crate::{ApiError, AppState, auth::require_auth, routes::sessions};

pub(crate) async fn list_agents(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<AgentProfileDto>>, ApiError> {
    require_auth(&state, &headers, None)?;
    Ok(Json(vec![AgentProfileDto {
        id: "root-agent".to_string(),
        name: "Root Agent".to_string(),
        description: "默认根执行代理".to_string(),
        mode: "primary".to_string(),
        allowed_tools: Vec::new(),
        disallowed_tools: Vec::new(),
    }]))
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
    let session = state
        .app
        .create_session(working_dir.to_string_lossy().to_string())
        .await
        .map_err(ApiError::from)?;
    let merged_task = match request.context {
        Some(context) if !context.trim().is_empty() => {
            format!("{}\n\n{}", context.trim(), request.task)
        },
        _ => request.task,
    };
    let accepted = state
        .app
        .submit_prompt(&session.session_id, merged_task)
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
            agent_id: Some(agent_id),
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
/// 替代旧的 `cancel_subrun` 路由。新路由按 agent_id 而非 sub_run_id 定位，
/// 始终级联关闭子树。
pub(crate) async fn close_agent(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((session_id, agent_id)): Path<(String, String)>,
) -> Result<Json<CloseAgentResponse>, ApiError> {
    require_auth(&state, &headers, None)?;
    let session_id = sessions::validate_session_path_id(&session_id)?;
    state
        .app
        .interrupt_session(&session_id)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(CloseAgentResponse {
        closed_agent_ids: vec![agent_id],
    }))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CloseAgentResponse {
    closed_agent_ids: Vec<String>,
}
