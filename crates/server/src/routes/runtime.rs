use astrcode_protocol::http::{RuntimeReloadResponseDto, RuntimeStatusDto};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;

use crate::auth::require_auth;
use crate::mapper::to_runtime_status_dto;
use crate::{ApiError, AppState};

pub(crate) async fn get_runtime_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<RuntimeStatusDto>, ApiError> {
    require_auth(&state, &headers, None)?;
    Ok(Json(to_runtime_status_dto(
        state.runtime_governance.snapshot().await,
    )))
}

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
