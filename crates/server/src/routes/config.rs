//! # 配置路由处理器
//!
//! 处理配置相关的 HTTP 请求：
//! - `GET /api/config` — 获取配置视图（含 profile 列表和当前选择）
//! - `POST /api/config/active-selection` — 保存活跃的 profile/model 选择

use astrcode_protocol::http::{ConfigView, SaveActiveSelectionRequest};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};

use crate::{ApiError, AppState, auth::require_auth, mapper::build_config_view};

/// 获取当前配置视图。
///
/// 返回包含所有 profile 列表、当前激活的 profile/model、
/// 配置文件路径和可能的警告信息的完整配置视图。
pub(crate) async fn get_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ConfigView>, ApiError> {
    require_auth(&state, &headers, None)?;
    let config = state.service.get_config().await;
    let config_path = state
        .service
        .current_config_path()
        .await
        .map_err(ApiError::from)?
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
        .service
        .save_active_selection(request.active_profile, request.active_model)
        .await
        .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}
