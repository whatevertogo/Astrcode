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
    routes::sessions::{filter::SessionEventFilterQuery, validate_session_path_id},
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
    let filter_spec = filter_query.into_runtime_filter_spec()?;
    let snapshot = state
        .service
        .sessions()
        .history_filtered(&session_id, filter_spec)
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
        .service
        .sessions()
        .view(&session_id, filter_spec)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(SessionViewResponseDto {
        focus_events: snapshot
            .focus_history
            .into_iter()
            .map(|record| to_agent_event_envelope(record.event))
            .collect(),
        direct_children_events: snapshot
            .direct_children_history
            .into_iter()
            .map(|record| to_agent_event_envelope(record.event))
            .collect(),
        cursor: snapshot.cursor,
        phase: to_phase_dto(snapshot.phase),
    }))
}

/// 将路径中可能带 "session-" 前缀的 ID 统一剥离后返回 canonical 形式。
/// 用于与 open_session_id 等持久化字段做匹配：持久化值可能带前缀，也可能不带。
#[allow(dead_code)]
fn normalize_session_id_for_compare(raw: &str) -> String {
    raw.strip_prefix("session-").unwrap_or(raw).to_string()
}

#[cfg(test)]
mod tests {
    use super::normalize_session_id_for_compare;

    #[test]
    fn normalize_session_id_strips_session_prefix() {
        assert_eq!(
            normalize_session_id_for_compare("session-2026-04-09-abc"),
            "2026-04-09-abc"
        );
    }

    #[test]
    fn normalize_session_id_preserves_canonical_form() {
        assert_eq!(
            normalize_session_id_for_compare("2026-04-09-abc"),
            "2026-04-09-abc"
        );
    }

    #[test]
    fn normalize_session_id_strips_only_first_prefix() {
        // "session-session-abc" 只剥离第一个 "session-" 前缀
        assert_eq!(
            normalize_session_id_for_compare("session-session-abc"),
            "session-abc"
        );
    }

    #[test]
    fn normalize_session_id_empty_string() {
        assert_eq!(normalize_session_id_for_compare(""), "");
    }
}
