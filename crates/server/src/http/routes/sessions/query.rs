use astrcode_protocol::http::{ModeSummaryDto, SessionListItem, SessionModeStateDto};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};

use crate::{
    ApiError, AppState, auth::require_auth, mapper::to_session_list_item,
    routes::sessions::validate_session_path_id,
};

pub(crate) async fn list_sessions(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<SessionListItem>>, ApiError> {
    require_auth(&state, &headers, None)?;
    let sessions = state
        .app
        .list_sessions()
        .await
        .map_err(ApiError::from)?
        .into_iter()
        .map(astrcode_application::summarize_session_meta)
        .map(to_session_list_item)
        .collect();
    Ok(Json(sessions))
}

pub(crate) async fn list_modes(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ModeSummaryDto>>, ApiError> {
    require_auth(&state, &headers, None)?;
    let modes = state
        .app
        .list_modes()
        .await
        .map_err(ApiError::from)?
        .into_iter()
        .map(|summary| ModeSummaryDto {
            id: summary.id.to_string(),
            name: summary.name,
            description: summary.description,
        })
        .collect();
    Ok(Json(modes))
}

pub(crate) async fn get_session_mode(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<Json<SessionModeStateDto>, ApiError> {
    require_auth(&state, &headers, None)?;
    let session_id = validate_session_path_id(&session_id)?;
    let mode = state
        .app
        .session_mode_state(&session_id)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(SessionModeStateDto {
        current_mode_id: mode.current_mode_id.to_string(),
        last_mode_changed_at: mode
            .last_mode_changed_at
            .map(astrcode_application::format_local_rfc3339),
    }))
}
