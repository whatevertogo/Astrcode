use astrcode_core::AgentEvent;
use astrcode_protocol::http::{
    ChildAgentRefDto, ChildSessionLineageKindDto, ChildSessionNotificationDto,
    ChildSessionNotificationKindDto, ChildSessionViewProjectionDto, ChildSessionViewResponseDto,
    ParentChildSummaryListResponseDto, SessionHistoryResponseDto, SessionListItem,
};
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
    Ok(history
        .into_iter()
        .filter(|record| filter.matches(record))
        .filter(parent_timeline_event_visible)
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

/// 从父会话历史中提取所有 `ChildSessionNotification` 事件，
/// 投影为父视图摘要列表。父视图只消费摘要，不消费 child 原始事件流。
pub(crate) async fn parent_child_summary_list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<Json<ParentChildSummaryListResponseDto>, ApiError> {
    require_auth(&state, &headers, None)?;
    let session_id = validate_session_path_id(&session_id)?;
    let snapshot = state
        .service
        .sessions()
        .history(&session_id)
        .await
        .map_err(ApiError::from)?;

    let items: Vec<ChildSessionNotificationDto> = snapshot
        .history
        .into_iter()
        .filter_map(|record| extract_child_notification_dto(record.event))
        .collect();

    Ok(Json(ParentChildSummaryListResponseDto { items }))
}

/// 将路径中可能带 "session-" 前缀的 ID 统一剥离后返回 canonical 形式。
/// 用于与 open_session_id 等持久化字段做匹配：持久化值可能带前缀，也可能不带。
fn normalize_session_id_for_compare(raw: &str) -> String {
    raw.strip_prefix("session-").unwrap_or(raw).to_string()
}

/// 返回指定 child session 的可读视图投影。
/// 投影只包含可消费的摘要信息，不含 raw JSON 或内部 inbox envelope。
pub(crate) async fn child_session_view(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((session_id, child_session_id)): Path<(String, String)>,
) -> Result<Json<ChildSessionViewResponseDto>, ApiError> {
    require_auth(&state, &headers, None)?;
    let session_id = validate_session_path_id(&session_id)?;
    let child_session_id_canonical = validate_session_path_id(&child_session_id)?;
    let snapshot = state
        .service
        .sessions()
        .history(&session_id)
        .await
        .map_err(ApiError::from)?;

    // 在父会话历史中找到与目标 child session 匹配的最新通知
    let latest_notification = snapshot.history.iter().rev().find_map(|record| {
        match &record.event {
            AgentEvent::ChildSessionNotification { notification, .. } => {
                // open_session_id 可能带 "session-" 前缀，需要统一比较
                let canonical_open =
                    normalize_session_id_for_compare(&notification.open_session_id);
                if canonical_open == child_session_id_canonical {
                    Some(notification.clone())
                } else {
                    None
                }
            },
            _ => None,
        }
    });

    let child_ref = latest_notification
        .as_ref()
        .map(|n| to_child_ref_dto(n.child_ref.clone()))
        .unwrap_or_else(|| ChildAgentRefDto {
            agent_id: String::new(),
            session_id: session_id.clone(),
            sub_run_id: String::new(),
            parent_agent_id: None,
            lineage_kind: ChildSessionLineageKindDto::Spawn,
            status: "unknown".to_string(),
            openable: false,
            open_session_id: child_session_id.clone(),
        });

    let has_final_reply = latest_notification
        .as_ref()
        .map(|n| n.final_reply_excerpt.is_some())
        .unwrap_or(false);

    let status = latest_notification
        .as_ref()
        .map(|n| to_agent_status_string(n.status))
        .unwrap_or_else(|| "unknown".to_string());

    let summary = latest_notification
        .as_ref()
        .map(|n| n.summary.clone())
        .unwrap_or_default();

    let title = child_ref.agent_id.clone();
    let view = ChildSessionViewProjectionDto {
        child_ref,
        title,
        status,
        summary_items: if summary.is_empty() {
            Vec::new()
        } else {
            vec![summary]
        },
        latest_tool_activity: Vec::new(),
        has_final_reply,
        child_session_id,
        has_descriptor_lineage: latest_notification.is_some(),
    };

    Ok(Json(ChildSessionViewResponseDto { view }))
}

/// 从 AgentEvent 中提取 ChildSessionNotification 的 DTO 投影。
fn extract_child_notification_dto(event: AgentEvent) -> Option<ChildSessionNotificationDto> {
    match event {
        AgentEvent::ChildSessionNotification { notification, .. } => {
            Some(to_child_notification_dto(notification))
        },
        _ => None,
    }
}

fn to_child_notification_dto(
    notification: astrcode_core::ChildSessionNotification,
) -> ChildSessionNotificationDto {
    ChildSessionNotificationDto {
        notification_id: notification.notification_id,
        child_ref: to_child_ref_dto(notification.child_ref),
        kind: to_child_notification_kind_dto(notification.kind),
        summary: notification.summary,
        status: to_agent_status_string(notification.status),
        open_session_id: notification.open_session_id,
        source_tool_call_id: notification.source_tool_call_id,
        final_reply_excerpt: notification.final_reply_excerpt,
    }
}

fn to_child_ref_dto(child_ref: astrcode_core::ChildAgentRef) -> ChildAgentRefDto {
    ChildAgentRefDto {
        agent_id: child_ref.agent_id,
        session_id: child_ref.session_id,
        sub_run_id: child_ref.sub_run_id,
        parent_agent_id: child_ref.parent_agent_id,
        lineage_kind: match child_ref.lineage_kind {
            astrcode_core::ChildSessionLineageKind::Spawn => ChildSessionLineageKindDto::Spawn,
            astrcode_core::ChildSessionLineageKind::Fork => ChildSessionLineageKindDto::Fork,
            astrcode_core::ChildSessionLineageKind::Resume => ChildSessionLineageKindDto::Resume,
        },
        status: to_agent_status_string(child_ref.status),
        openable: child_ref.openable,
        open_session_id: child_ref.open_session_id,
    }
}

fn to_child_notification_kind_dto(
    kind: astrcode_core::ChildSessionNotificationKind,
) -> ChildSessionNotificationKindDto {
    match kind {
        astrcode_core::ChildSessionNotificationKind::Started => {
            ChildSessionNotificationKindDto::Started
        },
        astrcode_core::ChildSessionNotificationKind::ProgressSummary => {
            ChildSessionNotificationKindDto::ProgressSummary
        },
        astrcode_core::ChildSessionNotificationKind::Delivered => {
            ChildSessionNotificationKindDto::Delivered
        },
        astrcode_core::ChildSessionNotificationKind::Waiting => {
            ChildSessionNotificationKindDto::Waiting
        },
        astrcode_core::ChildSessionNotificationKind::Resumed => {
            ChildSessionNotificationKindDto::Resumed
        },
        astrcode_core::ChildSessionNotificationKind::Closed => {
            ChildSessionNotificationKindDto::Closed
        },
        astrcode_core::ChildSessionNotificationKind::Failed => {
            ChildSessionNotificationKindDto::Failed
        },
    }
}

fn to_agent_status_string(status: astrcode_core::AgentStatus) -> String {
    match status {
        astrcode_core::AgentStatus::Pending => "pending".to_string(),
        astrcode_core::AgentStatus::Running => "running".to_string(),
        astrcode_core::AgentStatus::Completed => "completed".to_string(),
        astrcode_core::AgentStatus::Cancelled => "cancelled".to_string(),
        astrcode_core::AgentStatus::Failed => "failed".to_string(),
    }
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
