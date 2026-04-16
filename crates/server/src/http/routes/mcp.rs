//! # MCP 管理 API 路由
//!
//! 提供 MCP 状态查询、审批，以及服务端配置管理入口。

use astrcode_application::{
    McpActionSummary, McpConfigScope, McpServerStatusSummary, RegisterMcpServerInput,
};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use serde::{Deserialize, Serialize};

use crate::{ApiError, AppState, auth::require_auth};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct McpServerStatus {
    pub name: String,
    pub scope: String,
    pub enabled: bool,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub tool_count: usize,
    pub prompt_count: usize,
    pub resource_count: usize,
    pub pending_approval: bool,
    pub server_signature: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct McpStatusResponse {
    pub servers: Vec<McpServerStatus>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SignatureRequest {
    pub server_signature: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReconnectRequest {
    pub name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemoveServerRequest {
    pub scope: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SetEnabledRequest {
    pub scope: String,
    pub name: String,
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpsertServerRequest {
    pub scope: String,
    pub name: String,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub init_timeout_secs: Option<u64>,
    #[serde(default)]
    pub max_reconnect_attempts: Option<u32>,
    pub transport: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub(crate) struct McpActionResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

pub(crate) async fn get_mcp_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<McpStatusResponse>), ApiError> {
    require_auth(&state, &headers, None)?;
    let servers = state
        .app
        .mcp()
        .list_status_summary()
        .await
        .into_iter()
        .map(McpServerStatus::from)
        .collect();
    Ok((StatusCode::OK, Json(McpStatusResponse { servers })))
}

pub(crate) async fn approve_mcp_server(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SignatureRequest>,
) -> Result<(StatusCode, Json<McpActionResponse>), ApiError> {
    require_auth(&state, &headers, None)?;
    state
        .app
        .mcp()
        .approve_server(&request.server_signature)
        .await
        .map_err(ApiError::from)?;
    Ok(ok_response(StatusCode::OK))
}

pub(crate) async fn reject_mcp_server(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SignatureRequest>,
) -> Result<(StatusCode, Json<McpActionResponse>), ApiError> {
    require_auth(&state, &headers, None)?;
    state
        .app
        .mcp()
        .reject_server(&request.server_signature)
        .await
        .map_err(ApiError::from)?;
    Ok(ok_response(StatusCode::OK))
}

pub(crate) async fn reconnect_mcp_server(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ReconnectRequest>,
) -> Result<(StatusCode, Json<McpActionResponse>), ApiError> {
    require_auth(&state, &headers, None)?;
    state
        .app
        .mcp()
        .reconnect_server(&request.name)
        .await
        .map_err(ApiError::from)?;
    Ok(ok_response(StatusCode::OK))
}

pub(crate) async fn reset_project_mcp_choices(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<McpActionResponse>), ApiError> {
    require_auth(&state, &headers, None)?;
    state
        .app
        .mcp()
        .reset_project_choices()
        .await
        .map_err(ApiError::from)?;
    Ok(ok_response(StatusCode::OK))
}

pub(crate) async fn upsert_mcp_server(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<UpsertServerRequest>,
) -> Result<(StatusCode, Json<McpActionResponse>), ApiError> {
    require_auth(&state, &headers, None)?;
    let input = RegisterMcpServerInput {
        name: request.name,
        scope: parse_scope(&request.scope)?,
        enabled: request.enabled.unwrap_or(true),
        timeout_secs: request.timeout_secs.unwrap_or(120),
        init_timeout_secs: request.init_timeout_secs.unwrap_or(30),
        max_reconnect_attempts: request.max_reconnect_attempts.unwrap_or(5),
        transport_config: request.transport,
    };
    state
        .app
        .mcp()
        .upsert_config(input)
        .await
        .map_err(ApiError::from)?;
    Ok(ok_response(StatusCode::OK))
}

pub(crate) async fn remove_mcp_server(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<RemoveServerRequest>,
) -> Result<(StatusCode, Json<McpActionResponse>), ApiError> {
    require_auth(&state, &headers, None)?;
    state
        .app
        .mcp()
        .remove_config(parse_scope(&request.scope)?, &request.name)
        .await
        .map_err(ApiError::from)?;
    Ok(ok_response(StatusCode::OK))
}

pub(crate) async fn set_mcp_server_enabled(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SetEnabledRequest>,
) -> Result<(StatusCode, Json<McpActionResponse>), ApiError> {
    require_auth(&state, &headers, None)?;
    state
        .app
        .mcp()
        .set_enabled(parse_scope(&request.scope)?, &request.name, request.enabled)
        .await
        .map_err(ApiError::from)?;
    Ok(ok_response(StatusCode::OK))
}

fn ok_response(status: StatusCode) -> (StatusCode, Json<McpActionResponse>) {
    let summary = McpActionSummary::ok();
    (
        status,
        Json(McpActionResponse {
            ok: summary.ok,
            message: summary.message,
        }),
    )
}

fn parse_scope(scope: &str) -> Result<McpConfigScope, ApiError> {
    match scope {
        "user" => Ok(McpConfigScope::User),
        "project" => Ok(McpConfigScope::Project),
        "local" => Ok(McpConfigScope::Local),
        other => Err(ApiError::bad_request(format!(
            "unsupported MCP scope '{}'",
            other
        ))),
    }
}

impl From<McpServerStatusSummary> for McpServerStatus {
    fn from(value: McpServerStatusSummary) -> Self {
        Self {
            name: value.name,
            scope: value.scope,
            enabled: value.enabled,
            status: value.status,
            error: value.error,
            tool_count: value.tool_count,
            prompt_count: value.prompt_count,
            resource_count: value.resource_count,
            pending_approval: value.pending_approval,
            server_signature: value.server_signature,
        }
    }
}
