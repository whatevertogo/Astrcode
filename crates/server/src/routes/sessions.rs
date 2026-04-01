//! # 会话路由处理器
//!
//! 处理会话相关的 HTTP 请求和 SSE 事件流。

use std::convert::Infallible;
use std::time::Duration;

use astrcode_protocol::http::{
    CreateSessionRequest, DeleteProjectResultDto, PromptAcceptedResponse, PromptRequest,
    SessionListItem, SessionMessageDto,
};
use astrcode_runtime::{PromptAccepted, SessionReplaySource};
use async_stream::stream;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;

use crate::auth::require_auth;
use crate::mapper::{
    format_event_id, parse_event_id, to_session_list_item, to_session_message_dto, to_sse_event,
};
use crate::{ApiError, AppState, SESSION_CURSOR_HEADER_NAME};

/// 删除项目查询参数
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeleteProjectQuery {
    working_dir: String,
}

/// 会话事件查询参数
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionEventsQuery {
    after_event_id: Option<String>,
    token: Option<String>,
}

pub(crate) async fn create_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateSessionRequest>,
) -> Result<Json<SessionListItem>, ApiError> {
    require_auth(&state, &headers, None)?;
    let meta = state
        .service
        .create_session(request.working_dir)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(to_session_list_item(meta)))
}

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
            cursor
                .parse::<axum::http::HeaderValue>()
                .map_err(|error| ApiError {
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    message: error.to_string(),
                })?,
        );
    }
    Ok(response)
}

pub(crate) async fn submit_prompt(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Json(request): Json<PromptRequest>,
) -> Result<(StatusCode, Json<PromptAcceptedResponse>), ApiError> {
    require_auth(&state, &headers, None)?;
    let accepted: PromptAccepted = state
        .service
        .submit_prompt(&session_id, request.text)
        .await
        .map_err(ApiError::from)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(PromptAcceptedResponse {
            turn_id: accepted.turn_id,
        }),
    ))
}

pub(crate) async fn interrupt_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    require_auth(&state, &headers, None)?;
    state
        .service
        .interrupt(&session_id)
        .await
        .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn delete_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    require_auth(&state, &headers, None)?;
    state
        .service
        .delete_session(&session_id)
        .await
        .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn delete_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<DeleteProjectQuery>,
) -> Result<Json<DeleteProjectResultDto>, ApiError> {
    require_auth(&state, &headers, None)?;
    let result = state
        .service
        .delete_project(&query.working_dir)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(DeleteProjectResultDto {
        success_count: result.success_count,
        failed_session_ids: result.failed_session_ids,
    }))
}

pub(crate) async fn session_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Query(query): Query<SessionEventsQuery>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    require_auth(&state, &headers, query.token.as_deref())?;
    let last_event_id = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string())
        .or(query.after_event_id);
    let mut replay = state
        .service
        .replay(&session_id, last_event_id.as_deref())
        .await
        .map_err(ApiError::from)?;
    let mut last_sent = last_event_id.as_deref().and_then(parse_event_id);
    let service = state.service.clone();
    let session_id_for_stream = session_id.clone();

    let event_stream = stream! {
        for record in replay.history {
            if let Some(id) = parse_event_id(&record.event_id) {
                last_sent = Some(id);
            }
            yield Ok::<Event, Infallible>(to_sse_event(record));
        }

        loop {
            match replay.receiver.recv().await {
                Ok(record) => {
                    let Some(current_id) = parse_event_id(&record.event_id) else {
                        continue;
                    };
                    if let Some(last_id) = last_sent {
                        if current_id <= last_id {
                            continue;
                        }
                    }
                    last_sent = Some(current_id);
                    yield Ok::<Event, Infallible>(to_sse_event(record));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // SSE lag 恢复：broadcast channel 有界容量（默认 1024），
                    // 如果客户端消费太慢会被 Channel 丢弃消息。此时需要重新
                    // 从磁盘回放历史事件，以 last_sent 为游标起点，补齐丢失
                    // 的事件后继续订阅。如果回放也失败则断开连接。
                    let cursor = last_sent.map(format_event_id);
                    match service
                        .replay(&session_id_for_stream, cursor.as_deref())
                        .await
                    {
                        Ok(recovered) => {
                            for record in &recovered.history {
                                if let Some(id) = parse_event_id(&record.event_id) {
                                    last_sent = Some(id);
                                }
                                yield Ok::<Event, Infallible>(to_sse_event(record.clone()));
                            }
                            replay = recovered;
                        }
                        Err(_) => break,
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };

    Ok(Sse::new(event_stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keepalive"),
    ))
}
