use astrcode_protocol::http::{
    CurrentModelInfoDto, ModelOptionDto, TestConnectionRequest, TestResultDto,
};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;

use crate::auth::require_auth;
use crate::mapper::{list_model_options, resolve_current_model};
use crate::{ApiError, AppState};

pub(crate) async fn get_current_model(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<CurrentModelInfoDto>, ApiError> {
    require_auth(&state, &headers, None)?;
    let config = state.service.get_config().await;
    Ok(Json(resolve_current_model(&config)?))
}

pub(crate) async fn list_models(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ModelOptionDto>>, ApiError> {
    require_auth(&state, &headers, None)?;
    let config = state.service.get_config().await;
    Ok(Json(list_model_options(&config)))
}

pub(crate) async fn test_model_connection(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<TestConnectionRequest>,
) -> Result<Json<TestResultDto>, ApiError> {
    require_auth(&state, &headers, None)?;
    let result = state
        .service
        .test_connection(&request.profile_name, &request.model)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(TestResultDto {
        success: result.success,
        provider: result.provider,
        model: result.model,
        error: result.error,
    }))
}
