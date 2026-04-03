//! # 会话路由处理器
//!
//! 处理会话相关的 HTTP 请求和 SSE 事件流。
//!
//! ## 端点清单
//!
//! - `POST /api/sessions` — 创建新会话
//! - `GET /api/sessions` — 列出所有会话
//! - `GET /api/session-events` — 订阅会话目录事件（SSE）
//! - `GET /api/sessions/:id/messages` — 获取会话消息快照
//! - `POST /api/sessions/:id/prompts` — 提交用户提示
//! - `POST /api/sessions/:id/compact` — 压缩会话上下文
//! - `POST /api/sessions/:id/interrupt` — 中断会话执行
//! - `GET /api/sessions/:id/events` — 订阅会话事件流（SSE，支持断点续传）
//! - `DELETE /api/sessions/:id` — 删除单个会话
//! - `DELETE /api/projects` — 删除整个项目（级联删除所有会话）
//!
//! ## SSE 事件流设计
//!
//! `GET /api/sessions/:id/events` 是核心端点，采用两阶段模式：
//! 1. **回放阶段**：通过 `SessionReplaySource` 从磁盘回放历史事件
//! 2. **实时订阅**：切换到 broadcast channel 接收实时事件
//!
//! 事件 ID 格式为 `{storage_seq}.{subindex}`，客户端通过 `Last-Event-ID`
//! 请求头或 `afterEventId` 查询参数实现断点续传。
//! 如果 broadcast channel 发生 lag（客户端消费太慢），
//! 会自动从磁盘重新回放补齐丢失的事件。

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
    format_event_id, parse_event_id, to_session_catalog_sse_event, to_session_list_item,
    to_session_message_dto, to_sse_event,
};
use crate::{ApiError, AppState, SESSION_CURSOR_HEADER_NAME};

/// 删除项目查询参数。
///
/// 通过 `workingDir` 查询参数指定要删除的项目工作目录。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeleteProjectQuery {
    working_dir: String,
}

/// 会话事件查询参数。
///
/// - `after_event_id`：从指定事件 ID 之后开始订阅（用于断点续传）
/// - `token`：SSE 查询参数认证（EventSource API 不支持自定义请求头时的备选方案）
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionEventsQuery {
    after_event_id: Option<String>,
    token: Option<String>,
}

/// 创建新会话。
///
/// 根据请求中的工作目录创建会话，返回会话元数据列表项。
/// 如果工作目录已有会话，会创建新的子会话。
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

/// 列出所有会话。
///
/// 返回所有会话的元数据列表，按更新时间排序。
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

/// 订阅会话目录事件流（SSE）。
///
/// 广播会话创建/删除、项目删除、会话分支等目录级变更。
/// 这是一个纯实时流，不包含历史事件回放。
/// 客户端断开重连后会从当前时刻开始接收新事件。
pub(crate) async fn session_catalog_events(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    require_auth(&state, &headers, None)?;
    let mut receiver = state.service.subscribe_session_catalog_events();

    let event_stream = stream! {
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    yield Ok::<Event, Infallible>(to_session_catalog_sse_event(event));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    continue;
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

/// 获取会话消息快照。
///
/// 返回会话中所有已持久化的消息列表（User/Assistant/ToolCall），
/// 用于前端初始化会话视图。
/// 响应头 `x-session-cursor` 包含当前快照的游标，可用于增量更新。
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

/// 提交用户提示到会话。
///
/// 将用户的文本输入提交到指定会话，触发智能体执行轮次。
/// 返回 202 Accepted 和轮次 ID，表示请求已接受但尚未完成。
/// 如果会话正在执行中，会自动创建分支会话。
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
            session_id: accepted.session_id,
            branched_from_session_id: accepted.branched_from_session_id,
        }),
    ))
}

/// 中断会话执行。
///
/// 向指定会话发送中断信号，停止当前的智能体轮次。
/// 成功时返回 204 No Content。
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

/// 压缩会话上下文。
///
/// 触发会话上下文压缩，减少 token 占用。
/// 成功时返回 204 No Content。
pub(crate) async fn compact_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    require_auth(&state, &headers, None)?;
    state
        .service
        .compact_session(&session_id)
        .await
        .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

/// 删除单个会话。
///
/// 从会话目录中移除指定会话，同时广播 `SessionDeleted` 事件。
/// 成功时返回 204 No Content。
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

/// 删除整个项目。
///
/// 级联删除指定工作目录下的所有会话。
/// 返回成功删除的会话数和失败的会话 ID 列表。
/// 同时广播 `ProjectDeleted` 事件。
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

/// 订阅会话事件流（SSE）。
///
/// 这是前端获取会话实时事件的核心端点，采用两阶段模式：
///
/// 1. **回放阶段**：从磁盘回放历史事件（支持 `after_event_id` 断点续传）
/// 2. **实时订阅**：切换到 broadcast channel 接收新事件
///
/// ## 断点续传
///
/// 客户端通过以下方式指定起始位置：
/// - `Last-Event-ID` 请求头（标准 SSE 机制）
/// - `afterEventId` 查询参数（备选方案）
///
/// ## Lag 恢复
///
/// 如果 broadcast channel 发生 lag（客户端消费太慢导致消息被丢弃），
/// 会自动从磁盘重新回放历史事件，以 `last_sent` 为游标起点补齐
/// 丢失的事件后继续订阅。如果回放也失败则断开连接。
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
