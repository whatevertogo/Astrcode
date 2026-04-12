use std::{convert::Infallible, pin::Pin, time::Duration};

use astrcode_protocol::http::PROTOCOL_VERSION;
use astrcode_runtime::SessionEventFilter;
use async_stream::stream;
use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    response::sse::{Event, KeepAlive, Sse},
};
use serde::Deserialize;
use serde_json::json;

use crate::{
    ApiError, AppState,
    auth::require_auth,
    mapper::{
        format_event_id, parse_event_id, to_live_sse_event, to_session_catalog_sse_event,
        to_sse_event,
    },
    routes::sessions::{
        filter::{SessionEventFilterQuery, record_is_after_cursor},
        validate_session_path_id,
    },
};

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionEventsQuery {
    after_event_id: Option<String>,
    token: Option<String>,
    #[serde(flatten)]
    filter: SessionEventFilterQuery,
}

pub(crate) async fn session_catalog_events(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    require_auth(&state, &headers, None)?;
    let mut receiver = state.service.sessions().subscribe_catalog();

    let event_stream = stream! {
        loop {
            match receiver.recv().await {
                Ok(event) => yield Ok::<Event, Infallible>(to_session_catalog_sse_event(event)),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    yield Ok::<Event, Infallible>(stream_error_event(
                        "session_catalog_stream_closed",
                        "session catalog stream closed",
                    ));
                    break;
                },
            }
        }
    };

    Ok(Sse::new(event_stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keepalive"),
    ))
}

pub(crate) async fn session_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Query(query): Query<SessionEventsQuery>,
) -> Result<SessionEventSse, ApiError> {
    require_auth(&state, &headers, query.token.as_deref())?;
    let session_id = validate_session_path_id(&session_id)?;
    let filter_spec = query.filter.into_runtime_filter_spec()?;
    let last_event_id = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string())
        .or(query.after_event_id);

    if let Some(filter_spec) = filter_spec {
        return filtered_session_events(state, session_id, filter_spec, last_event_id).await;
    }

    unfiltered_session_events(state, session_id, last_event_id).await
}

async fn unfiltered_session_events(
    state: AppState,
    session_id: String,
    last_event_id: Option<String>,
) -> Result<SessionEventSse, ApiError> {
    let mut replay = state
        .service
        .sessions()
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
            tokio::select! {
                durable = replay.receiver.recv() => {
                    match durable {
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
                            let cursor = last_sent.map(format_event_id);
                            match service
                                .sessions()
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
                                Err(error) => {
                                    log::warn!(
                                        "SSE replay recovery failed for session '{}': cursor='{}', error={}",
                                        session_id_for_stream,
                                        cursor.as_deref().unwrap_or("<start>"),
                                        error
                                    );
                                    yield Ok::<Event, Infallible>(stream_error_event(
                                        "session_event_replay_failed",
                                        format!(
                                            "failed to recover lagged session events for '{}': {error}",
                                            session_id_for_stream
                                        ),
                                    ));
                                    break;
                                },
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            yield Ok::<Event, Infallible>(stream_error_event(
                                "session_event_stream_closed",
                                format!("session event stream closed for '{}'", session_id_for_stream),
                            ));
                            break;
                        },
                    }
                }
                live = replay.live_receiver.recv() => {
                    match live {
                        Ok(event) => yield Ok::<Event, Infallible>(to_live_sse_event(event)),
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                            log::debug!(
                                "session '{}' live delta stream lagged by {} events; skipping lost live-only deltas",
                                session_id_for_stream,
                                skipped
                            );
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {}
                    }
                }
            }
        }
    };

    Ok(
        Sse::new(Box::pin(event_stream) as SessionEventStream).keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("keepalive"),
        ),
    )
}

async fn filtered_session_events(
    state: AppState,
    session_id: String,
    filter_spec: astrcode_runtime::SessionEventFilterSpec,
    last_event_id: Option<String>,
) -> Result<SessionEventSse, ApiError> {
    let snapshot = state
        .service
        .sessions()
        .history(&session_id)
        .await
        .map_err(ApiError::from)?;
    let latest_cursor = snapshot.cursor.clone();
    let mut filter =
        SessionEventFilter::new(filter_spec, &snapshot.history).map_err(ApiError::from)?;
    let mut initial_history = Vec::new();
    let mut last_sent = last_event_id.as_deref().and_then(parse_event_id);

    for record in snapshot.history {
        let matched = filter.matches(&record);
        if !matched || !record_is_after_cursor(&record, last_event_id.as_deref()) {
            continue;
        }
        if let Some(id) = parse_event_id(&record.event_id) {
            last_sent = Some(id);
        }
        initial_history.push(record);
    }

    let mut replay = state
        .service
        .sessions()
        .replay(&session_id, latest_cursor.as_deref())
        .await
        .map_err(ApiError::from)?;
    let service = state.service.clone();
    let session_id_for_stream = session_id.clone();

    let event_stream = stream! {
        for record in initial_history {
            yield Ok::<Event, Infallible>(to_sse_event(record));
        }

        loop {
            tokio::select! {
                durable = replay.receiver.recv() => {
                    match durable {
                        Ok(record) => {
                            let Some(current_id) = parse_event_id(&record.event_id) else {
                                continue;
                            };
                            if let Some(last_id) = last_sent {
                                if current_id <= last_id {
                                    continue;
                                }
                            }
                            if !filter.matches(&record) {
                                continue;
                            }
                            last_sent = Some(current_id);
                            yield Ok::<Event, Infallible>(to_sse_event(record));
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            let cursor = last_sent.map(format_event_id);
                            match service
                                .sessions()
                                .replay(&session_id_for_stream, cursor.as_deref())
                                .await
                            {
                                Ok(recovered) => {
                                    let mut recovered_filter =
                                        match SessionEventFilter::new(filter.spec().clone(), &recovered.history) {
                                            Ok(filter) => filter,
                                            Err(error) => {
                                                yield Ok::<Event, Infallible>(stream_error_event(
                                                    "lineage_metadata_unavailable",
                                                    error.to_string(),
                                                ));
                                                break;
                                            },
                                        };
                                    for record in &recovered.history {
                                        let Some(current_id) = parse_event_id(&record.event_id) else {
                                            continue;
                                        };
                                        if let Some(last_id) = last_sent {
                                            if current_id <= last_id {
                                                continue;
                                            }
                                        }
                                        if !recovered_filter.matches(record) {
                                            continue;
                                        }
                                        last_sent = Some(current_id);
                                        yield Ok::<Event, Infallible>(to_sse_event(record.clone()));
                                    }
                                    filter = recovered_filter;
                                    replay = recovered;
                                }
                                Err(error) => {
                                    log::warn!(
                                        "SSE filtered replay recovery failed for session '{}': cursor='{}', filter={:?}, error={}",
                                        session_id_for_stream,
                                        cursor.as_deref().unwrap_or("<start>"),
                                        filter.spec(),
                                        error
                                    );
                                    yield Ok::<Event, Infallible>(stream_error_event(
                                        "session_event_replay_failed",
                                        format!(
                                            "failed to recover lagged filtered session events for '{}': {error}",
                                            session_id_for_stream
                                        ),
                                    ));
                                    break;
                                },
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            yield Ok::<Event, Infallible>(stream_error_event(
                                "session_event_stream_closed",
                                format!("filtered session event stream closed for '{}'", session_id_for_stream),
                            ));
                            break;
                        },
                    }
                }
                live = replay.live_receiver.recv() => {
                    match live {
                        Ok(event) => {
                            if filter.matches_event(&event) {
                                yield Ok::<Event, Infallible>(to_live_sse_event(event));
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                            log::debug!(
                                "session '{}' filtered live delta stream lagged by {} events; skipping lost live-only deltas",
                                session_id_for_stream,
                                skipped
                            );
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {}
                    }
                }
            }
        }
    };

    Ok(
        Sse::new(Box::pin(event_stream) as SessionEventStream).keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("keepalive"),
        ),
    )
}

/// 在 SSE 终止前发送结构化错误事件，帮助前端给出可解释的断流提示。
fn stream_error_event(code: &str, message: impl Into<String>) -> Event {
    let payload = json!({
        "protocolVersion": PROTOCOL_VERSION,
        "event": "error",
        "data": {
            "code": code,
            "message": message.into()
        }
    });
    Event::default().event("error").data(payload.to_string())
}

type SessionEventStream =
    Pin<Box<dyn futures_util::Stream<Item = Result<Event, Infallible>> + Send>>;
type SessionEventSse = Sse<axum::response::sse::KeepAliveStream<SessionEventStream>>;
