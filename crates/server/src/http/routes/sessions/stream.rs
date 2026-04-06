use std::{convert::Infallible, time::Duration};

use astrcode_protocol::http::PROTOCOL_VERSION;
use astrcode_runtime::SessionReplaySource;
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
    mapper::{format_event_id, parse_event_id, to_session_catalog_sse_event, to_sse_event},
    routes::sessions::validate_session_path_id,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionEventsQuery {
    after_event_id: Option<String>,
    token: Option<String>,
}

pub(crate) async fn session_catalog_events(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    require_auth(&state, &headers, None)?;
    let mut receiver = state.service.subscribe_session_catalog_events();

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
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    require_auth(&state, &headers, query.token.as_deref())?;
    let session_id = validate_session_path_id(&session_id)?;
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
                    let cursor = last_sent.map(format_event_id);
                    match service.replay(&session_id_for_stream, cursor.as_deref()).await {
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
                            yield Ok::<Event, Infallible>(stream_error_event(
                                "session_event_replay_failed",
                                format!("failed to recover lagged session events: {error}"),
                            ));
                            break;
                        },
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    yield Ok::<Event, Infallible>(stream_error_event(
                        "session_event_stream_closed",
                        "session event stream closed",
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
