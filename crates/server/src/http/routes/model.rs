//! # 模型路由处理器
//!
//! 处理模型相关的 HTTP 请求：
//! - `GET /api/models/current` — 获取当前激活的模型信息
//! - `GET /api/models` — 列出所有可用的模型选项
//! - `POST /api/models/test` — 测试指定模型的连接

use astrcode_protocol::http::{
    CurrentModelInfoDto, ModelOptionDto, TestConnectionRequest, TestResultDto,
};
use axum::{Json, extract::State, http::HeaderMap};

use crate::{
    ApiError, AppState,
    auth::require_auth,
    mapper::{list_model_options, resolve_current_model},
};

/// 获取当前激活的模型信息。
///
/// 返回当前配置中选择的 profile 名称、模型名称和提供者类型。
pub(crate) async fn get_current_model(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<CurrentModelInfoDto>, ApiError> {
    require_auth(&state, &headers, None)?;
    let config = state.app.config().get_config().await;
    Ok(Json(resolve_current_model(&config)?))
}

/// 列出所有可用的模型选项。
///
/// 遍历配置中所有 profile 的模型，返回扁平化的列表，
/// 前端据此渲染模型选择器。
pub(crate) async fn list_models(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ModelOptionDto>>, ApiError> {
    require_auth(&state, &headers, None)?;
    let config = state.app.config().get_config().await;
    Ok(Json(list_model_options(&config)))
}

/// 测试模型连接。
///
/// 向指定的 profile 和模型发送测试请求，验证连接是否正常。
/// 返回测试结果，包含成功状态、提供者、模型名称和可能的错误信息。
pub(crate) async fn test_model_connection(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<TestConnectionRequest>,
) -> Result<Json<TestResultDto>, ApiError> {
    require_auth(&state, &headers, None)?;
    let result = state
        .app
        .config()
        .test_connection(&request.profile_name, &request.model)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(result))
}
