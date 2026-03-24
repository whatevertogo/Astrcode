use astrcode_protocol::http::{ConfigView, SaveActiveSelectionRequest};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;

use crate::auth::require_auth;
use crate::mapper::build_config_view;
use crate::{ApiError, AppState};

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
