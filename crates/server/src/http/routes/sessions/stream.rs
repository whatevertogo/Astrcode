use std::{convert::Infallible, time::Duration};

use async_stream::stream;
use axum::{
    extract::State,
    http::HeaderMap,
    response::sse::{Event, KeepAlive, Sse},
};
use serde_json::json;

use crate::{ApiError, AppState, auth::require_auth, mapper::to_session_catalog_sse_event};

pub(crate) async fn session_catalog_events(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    require_auth(&state, &headers, None)?;
    let mut receiver = state.session_catalog.subscribe_catalog_events();

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

fn stream_error_event(code: &str, message: impl Into<String>) -> Event {
    let payload = json!({
        "event": "error",
        "data": {
            "code": code,
            "message": message.into()
        }
    });
    Event::default().event("error").data(payload.to_string())
}
