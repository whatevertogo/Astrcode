use astrcode_protocol::http::{SessionHistoryResponseDto, SessionListItem, SessionMessageDto};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};

use crate::{
    ApiError, AppState, SESSION_CURSOR_HEADER_NAME,
    auth::require_auth,
    mapper::{to_agent_event_envelope, to_phase_dto, to_session_list_item, to_session_message_dto},
};

pub(crate) async fn list_sessions(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<SessionListItem>>, ApiError> {
    require_auth(&state, &headers, None)?;
    let sessions = state
        .service
        .list_sessions_with_meta()
        .await
        .map_err(ApiError::from)?
        .into_iter()
        .map(to_session_list_item)
        .collect();
    Ok(Json(sessions))
}

pub(crate) async fn session_messages(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<Response, ApiError> {
    require_auth(&state, &headers, None)?;
    let (messages, cursor) = state
        .service
        .load_session_snapshot(&session_id)
        .await
        .map_err(ApiError::from)?;
    let payload = messages
        .into_iter()
        .map(to_session_message_dto)
        .collect::<Vec<SessionMessageDto>>();

    let mut response = Json(payload).into_response();
    if let Some(cursor) = cursor {
        response.headers_mut().insert(
            SESSION_CURSOR_HEADER_NAME,
            axum::http::HeaderValue::from_str(&cursor).map_err(|error| ApiError {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                message: error.to_string(),
            })?,
        );
    }
    Ok(response)
}

pub(crate) async fn session_history(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<Json<SessionHistoryResponseDto>, ApiError> {
    require_auth(&state, &headers, None)?;
    let snapshot = state
        .service
        .load_session_history(&session_id)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(SessionHistoryResponseDto {
        events: snapshot
            .history
            .into_iter()
            .map(|record| to_agent_event_envelope(record.event))
            .collect(),
        cursor: snapshot.cursor,
        phase: to_phase_dto(snapshot.phase),
    }))
}
