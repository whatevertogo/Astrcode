//! # Debug-only Debug Workbench 读取面
//!
//! 仅在 debug 构建中暴露，供独立 Debug Workbench 读取全局 overview、timeline、
//! session trace 与 agent tree。

use astrcode_protocol::http::{
    RuntimeDebugOverviewDto, RuntimeDebugTimelineDto, SessionDebugAgentsDto, SessionDebugTraceDto,
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};

use crate::{
    ApiError, AppState,
    auth::require_auth,
    mapper::{
        to_runtime_debug_overview_dto, to_runtime_debug_timeline_dto, to_session_debug_agents_dto,
        to_session_debug_trace_dto,
    },
};

pub(crate) async fn get_runtime_overview(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<RuntimeDebugOverviewDto>, ApiError> {
    require_auth(&state, &headers, None)?;
    Ok(Json(to_runtime_debug_overview_dto(
        state.debug_workbench.runtime_overview(),
    )))
}

pub(crate) async fn get_runtime_timeline(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<RuntimeDebugTimelineDto>, ApiError> {
    require_auth(&state, &headers, None)?;
    Ok(Json(to_runtime_debug_timeline_dto(
        state.debug_workbench.runtime_timeline(),
    )))
}

pub(crate) async fn get_session_trace(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<Json<SessionDebugTraceDto>, ApiError> {
    require_auth(&state, &headers, None)?;
    let trace = state.debug_workbench.session_trace(&session_id).await?;
    Ok(Json(to_session_debug_trace_dto(trace)))
}

pub(crate) async fn get_session_agents(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<Json<SessionDebugAgentsDto>, ApiError> {
    require_auth(&state, &headers, None)?;
    let agents = state.debug_workbench.session_agents(&session_id).await?;
    Ok(Json(to_session_debug_agents_dto(agents)))
}
