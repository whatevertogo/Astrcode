//! # 配置路由处理器
//!
//! 处理配置相关的 HTTP 请求：
//! - `GET /api/config` — 获取配置视图（含 profile 列表和当前选择）
//! - `POST /api/config/active-selection` — 保存活跃的 profile/model 选择

use astrcode_core::format_local_rfc3339;
use astrcode_protocol::http::{ConfigReloadResponse, ConfigView, SaveActiveSelectionRequest};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};

use crate::{
    ApiError, AppState,
    auth::require_auth,
    mapper::{build_config_view, to_runtime_status_dto},
};

/// 获取当前配置视图。
///
/// 返回包含所有 profile 列表、当前激活的 profile/model、
/// 配置文件路径和可能的警告信息的完整配置视图。
pub(crate) async fn get_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ConfigView>, ApiError> {
    require_auth(&state, &headers, None)?;
    let config = state.app.config().get_config().await;
    let config_path = state
        .app
        .config()
        .config_path()
        .to_string_lossy()
        .to_string();
    Ok(Json(build_config_view(&config, config_path)?))
}

/// 保存活跃的 profile 和 model 选择。
///
/// 将用户选择的 profile 和 model 持久化到配置文件，
/// 后续会话将使用此选择作为默认值。
/// 成功时返回 204 No Content。
pub(crate) async fn save_active_selection(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SaveActiveSelectionRequest>,
) -> Result<StatusCode, ApiError> {
    require_auth(&state, &headers, None)?;
    state
        .app
        .config()
        .save_active_selection(request.active_profile, request.active_model)
        .await
        .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

/// 从磁盘重新加载配置。
///
/// 成功后会通过治理入口重读配置并刷新当前 capability surface。
/// 成功时返回 202 Accepted、重载后的配置视图和当前运行时快照。
pub(crate) async fn reload_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<ConfigReloadResponse>), ApiError> {
    require_auth(&state, &headers, None)?;
    let reloaded = state.governance.reload().await.map_err(ApiError::from)?;
    let config = state.app.config().get_config().await;
    let config_path = state
        .app
        .config()
        .config_path()
        .to_string_lossy()
        .to_string();
    let config_view = build_config_view(&config, config_path)?;

    Ok((
        StatusCode::ACCEPTED,
        Json(ConfigReloadResponse {
            reloaded_at: format_local_rfc3339(reloaded.reloaded_at),
            config: config_view,
            status: to_runtime_status_dto(reloaded.snapshot),
        }),
    ))
}
