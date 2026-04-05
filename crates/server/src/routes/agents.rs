//! # Agent 路由
//!
//! 预埋未来 `/api/v1/agents/*` 入口：
//! - 现在先提供列表查询
//! - execute 端点先返回结构化的未启用响应，避免协议面继续漂移

use astrcode_protocol::http::{AgentExecuteResponseDto, AgentProfileDto};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};

use crate::{ApiError, AppState, auth::require_auth, mapper::to_agent_profile_dto};

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
) -> Result<(StatusCode, Json<AgentExecuteResponseDto>), ApiError> {
    require_auth(&state, &headers, None)?;
    Ok((
        StatusCode::NOT_IMPLEMENTED,
        Json(AgentExecuteResponseDto {
            accepted: false,
            message: format!(
                "direct agent execution for '{}' is not enabled yet; use runAgent inside a \
                 session turn for now",
                agent_id
            ),
        }),
    ))
}
