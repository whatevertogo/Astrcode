use astrcode_protocol::http::{SessionHistoryResponseDto, SessionListItem, SessionViewResponseDto};
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
        filter::{SessionEventFilterQuery, record_matches_filter},
        validate_session_path_id,
    },
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
    let filter_spec = filter_query.into_runtime_filter_spec()?;
    let snapshot = state
        .app
        .session_history(&session_id)
        .await
        .map_err(ApiError::from)?;
    let events = snapshot
        .history
        .into_iter()
        .filter(|record| {
            filter_spec
                .as_ref()
                .map(|spec| record_matches_filter(record, spec))
                .unwrap_or(true)
        })
        .map(|record| to_agent_event_envelope(record.event))
        .collect();

    Ok(Json(SessionHistoryResponseDto {
        events,
        cursor: snapshot.cursor,
        phase: to_phase_dto(snapshot.phase),
    }))
}

pub(crate) async fn session_view(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Query(filter_query): Query<SessionEventFilterQuery>,
) -> Result<Json<SessionViewResponseDto>, ApiError> {
    require_auth(&state, &headers, None)?;
    let session_id = validate_session_path_id(&session_id)?;
    let filter_spec = filter_query.into_runtime_filter_spec()?;
    let snapshot = state
        .app
        .session_view(&session_id)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(SessionViewResponseDto {
        focus_events: snapshot
            .focus_history
            .into_iter()
            .filter(|record| {
                filter_spec
                    .as_ref()
                    .map(|spec| record_matches_filter(record, spec))
                    .unwrap_or(true)
            })
            .map(|record| to_agent_event_envelope(record.event))
            .collect(),
        direct_children_events: snapshot
            .direct_children_history
            .into_iter()
            .filter(|record| {
                filter_spec
                    .as_ref()
                    .map(|spec| record_matches_filter(record, spec))
                    .unwrap_or(true)
            })
            .map(|record| to_agent_event_envelope(record.event))
            .collect(),
        cursor: snapshot.cursor,
        phase: to_phase_dto(snapshot.phase),
    }))
}
