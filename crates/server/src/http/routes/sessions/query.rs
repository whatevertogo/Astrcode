use astrcode_protocol::http::{SessionHistoryResponseDto, SessionListItem};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};

use crate::{
    ApiError, AppState,
    auth::require_auth,
    mapper::{to_agent_event_envelope, to_phase_dto, to_session_list_item},
    routes::sessions::{
        filter::{SessionEventFilter, SessionEventFilterQuery, SessionEventFilterSpec},
        validate_session_path_id,
    },
};

pub(crate) async fn list_sessions(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<SessionListItem>>, ApiError> {
    require_auth(&state, &headers, None)?;
    let sessions = state
        .service
        .sessions()
        .list()
        .await
        .map_err(ApiError::from)?
        .into_iter()
        .map(to_session_list_item)
        .collect();
    Ok(Json(sessions))
}

pub(crate) async fn session_history(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Query(filter_query): Query<SessionEventFilterQuery>,
) -> Result<Json<SessionHistoryResponseDto>, ApiError> {
    require_auth(&state, &headers, None)?;
    let session_id = validate_session_path_id(&session_id)?;
    let filter_spec = SessionEventFilterSpec::from_query(filter_query)?;
    let snapshot = state
        .service
        .sessions()
        .history(&session_id)
        .await
        .map_err(ApiError::from)?;
    let history = filter_history(snapshot.history, filter_spec)?;
    let cursor = history.last().map(|record| record.event_id.clone());

    Ok(Json(SessionHistoryResponseDto {
        events: history
            .into_iter()
            .map(|record| to_agent_event_envelope(record.event))
            .collect(),
        cursor,
        phase: to_phase_dto(snapshot.phase),
    }))
}

fn filter_history(
    history: Vec<astrcode_core::SessionEventRecord>,
    filter_spec: Option<SessionEventFilterSpec>,
) -> Result<Vec<astrcode_core::SessionEventRecord>, ApiError> {
    let Some(filter_spec) = filter_spec else {
        return Ok(history);
    };

    let mut filter = SessionEventFilter::new(filter_spec, &history)?;
    Ok(history
        .into_iter()
        .filter(|record| filter.matches(record))
        .collect())
}
