//! # Tool 路由
//!
//! 预埋未来 `/api/v1/tools/*` 入口：
//! - 现在先提供 runtime 当前工具列表
//! - execute 端点先返回结构化的未启用响应

use astrcode_protocol::http::{ToolDescriptorDto, ToolExecuteResponseDto};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};

use crate::{ApiError, AppState, auth::require_auth, mapper::to_tool_descriptor_dto};

pub(crate) async fn list_tools(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ToolDescriptorDto>>, ApiError> {
    require_auth(&state, &headers, None)?;
    let tools = state
        .service
        .tools()
        .list_tools()
        .await
        .into_iter()
        .map(to_tool_descriptor_dto)
        .collect::<Vec<_>>();
    Ok(Json(tools))
}

pub(crate) async fn execute_tool(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(tool_id): Path<String>,
) -> Result<(StatusCode, Json<ToolExecuteResponseDto>), ApiError> {
    require_auth(&state, &headers, None)?;
    Ok((
        StatusCode::NOT_IMPLEMENTED,
        Json(ToolExecuteResponseDto {
            accepted: false,
            message: format!(
                "direct tool execution for '{}' is not enabled yet; use a session turn or \
                 spawnAgent for now",
                tool_id
            ),
        }),
    ))
}
