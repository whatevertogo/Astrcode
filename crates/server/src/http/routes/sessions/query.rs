use astrcode_core::AgentEvent;
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
        return Ok(history
            .into_iter()
            .filter(parent_timeline_event_visible)
            .collect());
    };

    let mut filter = SessionEventFilter::new(filter_spec, &history)?;
    // 用户显式指定 subRunId 时不应用 parent_timeline_event_visible，
    // 因为用户明确请求了特定子执行的完整事件流，包括 boundary 事件。
    Ok(history
        .into_iter()
        .filter(|record| filter.matches(record))
        .collect())
}

/// 父时间线只保留 child boundary facts，不再直接暴露独立 child session 的内部 lifecycle。
fn parent_timeline_event_visible(record: &astrcode_core::SessionEventRecord) -> bool {
    match &record.event {
        AgentEvent::SubRunStarted { agent, .. } | AgentEvent::SubRunFinished { agent, .. } => {
            agent.storage_mode != Some(astrcode_core::SubRunStorageMode::IndependentSession)
        },
        _ => true,
    }
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
