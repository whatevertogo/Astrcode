//! # 运行时路由处理器
//!
//! 处理运行时相关的 HTTP 请求：
//! - `GET /api/runtime/plugins` — 获取运行时插件状态快照
//! - `POST /api/runtime/plugins/reload` — 触发热重载所有运行时插件

use astrcode_protocol::http::{RuntimeReloadResponseDto, RuntimeStatusDto};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;

use crate::auth::require_auth;
use crate::mapper::to_runtime_status_dto;
use crate::{ApiError, AppState};

/// 获取运行时状态快照。
///
/// 返回当前运行时的完整状态，包括插件列表、健康度、
/// 能力描述和运行时指标。
pub(crate) async fn get_runtime_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<RuntimeStatusDto>, ApiError> {
    require_auth(&state, &headers, None)?;
    Ok(Json(to_runtime_status_dto(
        state.runtime_governance.snapshot().await,
    )))
}

/// 重载运行时插件。
///
/// 触发热重载所有已加载的插件，返回重载时间戳和重载后的状态快照。
/// 成功时返回 202 Accepted，表示重载操作已接受并正在执行。
pub(crate) async fn reload_runtime_plugins(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<RuntimeReloadResponseDto>), ApiError> {
    require_auth(&state, &headers, None)?;
    let reloaded = state
        .runtime_governance
        .reload()
        .await
        .map_err(ApiError::from)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(RuntimeReloadResponseDto {
            reloaded_at: reloaded.reloaded_at.to_rfc3339(),
            status: to_runtime_status_dto(reloaded.snapshot),
        }),
    ))
}
