use std::{convert::Infallible, time::Duration};

use astrcode_runtime::SessionReplaySource;
use async_stream::stream;
use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    response::sse::{Event, KeepAlive, Sse},
};
use serde::Deserialize;

use crate::{
    ApiError, AppState,
    auth::require_auth,
    mapper::{format_event_id, parse_event_id, to_session_catalog_sse_event, to_sse_event},
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
