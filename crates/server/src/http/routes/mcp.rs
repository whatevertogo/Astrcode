//! # MCP 管理 API 路由
//!
//! 提供 MCP 状态查询、审批，以及服务端配置管理入口。

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use serde::{Deserialize, Serialize};

use crate::{ApiError, AppState, auth::require_auth};

/// 服务器状态信息。
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
    pub transport: UpsertTransportRequest,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub(crate) enum UpsertTransportRequest {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: std::collections::HashMap<String, String>,
    },
    Http {
        url: String,
        #[serde(default)]
        headers: std::collections::HashMap<String, String>,
    },
    Sse {
        url: String,
        #[serde(default)]
        headers: std::collections::HashMap<String, String>,
    },
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
        .service
        .mcp()
        .list_status()
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
        .service
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
        .service
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
        .service
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
        .service
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
    let config = request.into_server_config()?;
    state
        .service
        .mcp()
        .upsert_config(config)
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
        .service
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
        .service
        .mcp()
        .set_enabled(parse_scope(&request.scope)?, &request.name, request.enabled)
        .await
        .map_err(ApiError::from)?;
    Ok(ok_response(StatusCode::OK))
}

fn ok_response(status: StatusCode) -> (StatusCode, Json<McpActionResponse>) {
    (
        status,
        Json(McpActionResponse {
            ok: true,
            message: None,
        }),
    )
}

fn parse_scope(scope: &str) -> Result<astrcode_runtime::McpConfigScope, ApiError> {
    match scope {
        "user" => Ok(astrcode_runtime::McpConfigScope::User),
        "project" => Ok(astrcode_runtime::McpConfigScope::Project),
        "local" => Ok(astrcode_runtime::McpConfigScope::Local),
        other => Err(ApiError::bad_request(format!(
            "unsupported MCP scope '{}'",
            other
        ))),
    }
}

impl UpsertServerRequest {
    fn into_server_config(self) -> Result<astrcode_runtime::McpServerConfig, ApiError> {
        let transport = match self.transport {
            UpsertTransportRequest::Stdio { command, args, env } => {
                astrcode_runtime::McpTransportConfig::Stdio { command, args, env }
            },
            UpsertTransportRequest::Http { url, headers } => {
                astrcode_runtime::McpTransportConfig::StreamableHttp {
                    url,
                    headers,
                    oauth: None,
                }
            },
            UpsertTransportRequest::Sse { url, headers } => {
                astrcode_runtime::McpTransportConfig::Sse {
                    url,
                    headers,
                    oauth: None,
                }
            },
        };

        Ok(astrcode_runtime::McpServerConfig {
            name: self.name,
            transport,
            scope: parse_scope(&self.scope)?,
            enabled: self.enabled.unwrap_or(true),
            timeout_secs: self.timeout_secs.unwrap_or(120),
            init_timeout_secs: self.init_timeout_secs.unwrap_or(30),
            max_reconnect_attempts: self.max_reconnect_attempts.unwrap_or(5),
        })
    }
}

impl From<astrcode_runtime::McpServerStatusSnapshot> for McpServerStatus {
    fn from(value: astrcode_runtime::McpServerStatusSnapshot) -> Self {
        Self {
            name: value.name,
            scope: value.scope,
            enabled: value.enabled,
            status: value.state,
            error: value.error,
            tool_count: value.tool_count,
            prompt_count: value.prompt_count,
            resource_count: value.resource_count,
            pending_approval: value.pending_approval,
            server_signature: value.server_signature,
        }
    }
}
