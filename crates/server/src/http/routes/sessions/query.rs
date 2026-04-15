use astrcode_protocol::http::SessionListItem;
use axum::{Json, extract::State, http::HeaderMap};

use crate::{ApiError, AppState, auth::require_auth, mapper::to_session_list_item};

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
        .map(to_session_list_item)
        .collect();
    Ok(Json(sessions))
}
