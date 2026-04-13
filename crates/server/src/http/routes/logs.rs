//! 客户端日志上报端点。
//!
//! 前端统一通过此接口把日志发送到服务端：
//! - 仅认证后请求可访问；使用 API token 鉴权
//! - 直接按日志级别转发到 Rust 日志系统，借助服务端统一 logger 统一落盘

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;

use crate::{ApiError, AppState, auth::require_auth};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LogPayload {
    level: String,
    source: String,
    scope: Option<String>,
    message: String,
    details: Option<Vec<serde_json::Value>>,
}

pub(crate) async fn ingest(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<LogPayload>,
) -> Result<StatusCode, ApiError> {
    require_auth(&state, &headers, None)?;

    let requested_level = payload.level.to_lowercase();
    let scope = payload.scope.as_deref().unwrap_or("frontend");
    let level = match requested_level.as_str() {
        "error" => log::Level::Error,
        "warn" | "warning" => log::Level::Warn,
        "info" => log::Level::Info,
        "debug" => log::Level::Debug,
        "trace" => log::Level::Trace,
        _ => return Err(ApiError::bad_request("invalid log level".to_string())),
    };
    let scoped_level = match payload.scope.as_deref() {
        Some("model") if matches!(level, log::Level::Warn | log::Level::Error) => log::Level::Info,
        Some("backend") => level,
        _ => level,
    };

    let details = payload
        .details
        .map(|details| serde_json::to_string(&details).unwrap_or_else(|_| "[]".to_string()))
        .unwrap_or_default();
    let source = payload.source.trim();
    let message = payload.message.trim();
    let printable_message = if message.is_empty() {
        "(empty message)"
    } else {
        message
    };

    if details.is_empty() {
        log::log!(
            target: "frontend",
            scoped_level,
            "frontend:{}:{} {}",
            source,
            scope,
            printable_message
        );
    } else {
        log::log!(
            target: "frontend",
            scoped_level,
            "frontend:{}:{} {} {}",
            source,
            scope,
            printable_message,
            details
        );
    }

    Ok(StatusCode::NO_CONTENT)
}
