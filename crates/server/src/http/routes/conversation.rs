use std::{convert::Infallible, pin::Pin, time::Duration};

use astrcode_application::{
    ApplicationError, ConversationFocus, TerminalControlFacts, TerminalStreamFacts,
};
use astrcode_protocol::http::conversation::v1::{
    ConversationDeltaDto, ConversationSlashCandidatesResponseDto, ConversationSnapshotResponseDto,
    ConversationStreamEnvelopeDto,
};
use async_stream::stream;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    AppState,
    auth::is_authorized,
    routes::sessions::validate_session_path_id,
    terminal_projection::{
        project_terminal_child_summary_deltas, project_terminal_control_delta,
        project_terminal_rehydrate_envelope, project_terminal_slash_candidates,
        project_terminal_snapshot, project_terminal_stream_replay,
        seeded_terminal_stream_projector,
    },
};

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConversationStreamQuery {
    cursor: Option<String>,
    token: Option<String>,
    focus: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConversationSnapshotQuery {
    focus: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConversationSlashCandidatesQuery {
    q: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConversationRouteErrorPayload {
    code: &'static str,
    message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    details: Option<Value>,
}

#[derive(Debug)]
pub(crate) struct ConversationRouteError {
    status: StatusCode,
    code: &'static str,
    message: String,
    details: Option<Value>,
}

impl ConversationRouteError {
    fn auth_expired() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: "auth_expired",
            message: "unauthorized".to_string(),
            details: None,
        }
    }

    fn invalid_request(message: impl Into<String>, details: Option<Value>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "invalid_request",
            message: message.into(),
            details,
        }
    }
}

impl IntoResponse for ConversationRouteError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ConversationRouteErrorPayload {
                code: self.code,
                message: self.message,
                details: self.details,
            }),
        )
            .into_response()
    }
}

impl From<ApplicationError> for ConversationRouteError {
    fn from(value: ApplicationError) -> Self {
        match value {
            ApplicationError::NotFound(message) => Self {
                status: StatusCode::NOT_FOUND,
                code: "not_found",
                message,
                details: None,
            },
            ApplicationError::Conflict(message) => Self {
                status: StatusCode::CONFLICT,
                code: "conflict",
                message,
                details: None,
            },
            ApplicationError::InvalidArgument(message) => Self::invalid_request(message, None),
            ApplicationError::PermissionDenied(message) => Self {
                status: StatusCode::FORBIDDEN,
                code: "forbidden",
                message,
                details: None,
            },
            ApplicationError::Internal(message) => Self {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                code: "internal_error",
                message,
                details: None,
            },
        }
    }
}

pub(crate) async fn conversation_snapshot(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Query(query): Query<ConversationSnapshotQuery>,
) -> Result<Json<ConversationSnapshotResponseDto>, ConversationRouteError> {
    require_conversation_auth(&state, &headers, None)?;
    let session_id = validate_session_path_id(&session_id)
        .map_err(|error| ConversationRouteError::invalid_request(error.message, None))?;
    let focus = parse_focus_query(query.focus.as_deref())?;
    let facts = state
        .app
        .conversation_snapshot_facts(&session_id, focus)
        .await
        .map_err(ConversationRouteError::from)?;

    Ok(Json(project_terminal_snapshot(&facts)))
}

pub(crate) async fn conversation_stream(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Query(query): Query<ConversationStreamQuery>,
) -> Result<ConversationSse, ConversationRouteError> {
    require_conversation_auth(&state, &headers, query.token.as_deref())?;
    let session_id = validate_session_path_id(&session_id)
        .map_err(|error| ConversationRouteError::invalid_request(error.message, None))?;
    let focus = parse_focus_query(query.focus.as_deref())?;
    let cursor = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string())
        .or(query.cursor);

    let stream_facts = state
        .app
        .conversation_stream_facts(&session_id, cursor.as_deref(), focus.clone())
        .await
        .map_err(ConversationRouteError::from)?;

    match stream_facts {
        TerminalStreamFacts::Replay(facts) => Ok(build_conversation_stream(
            state, session_id, cursor, focus, *facts,
        )),
        TerminalStreamFacts::RehydrateRequired(rehydrate) => Ok(single_envelope_stream(
            project_terminal_rehydrate_envelope(&rehydrate),
        )),
    }
}

pub(crate) async fn conversation_slash_candidates(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Query(query): Query<ConversationSlashCandidatesQuery>,
) -> Result<Json<ConversationSlashCandidatesResponseDto>, ConversationRouteError> {
    require_conversation_auth(&state, &headers, None)?;
    let session_id = validate_session_path_id(&session_id)
        .map_err(|error| ConversationRouteError::invalid_request(error.message, None))?;
    let candidates = state
        .app
        .terminal_slash_candidates(&session_id, query.q.as_deref())
        .await
        .map_err(ConversationRouteError::from)?;

    Ok(Json(project_terminal_slash_candidates(&candidates)))
}

fn require_conversation_auth(
    state: &AppState,
    headers: &HeaderMap,
    query_token: Option<&str>,
) -> Result<(), ConversationRouteError> {
    if is_authorized(state, headers, query_token) {
        Ok(())
    } else {
        Err(ConversationRouteError::auth_expired())
    }
}

fn build_conversation_stream(
    state: AppState,
    session_id: String,
    cursor: Option<String>,
    focus: ConversationFocus,
    facts: astrcode_application::TerminalStreamReplayFacts,
) -> ConversationSse {
    let initial_envelopes = project_terminal_stream_replay(&facts, cursor.as_deref());
    let mut projector = seeded_terminal_stream_projector(&facts);
    let mut replay = facts.replay;
    let app = state.app.clone();
    let session_id_for_stream = session_id.clone();
    let mut last_sent_cursor = cursor;
    let mut cached_control = facts.control.clone();
    let mut cached_children = facts.child_summaries.clone();
    let mut cached_slash_candidates = facts.slash_candidates.clone();

    let event_stream = stream! {
        for envelope in initial_envelopes {
            last_sent_cursor = Some(envelope.cursor.0.clone());
            yield Ok::<Event, Infallible>(to_conversation_sse_event(envelope));
        }

        loop {
            match replay.receiver.recv().await {
                Ok(record) => {
                    let cursor = record.event_id.clone();
                    for delta in projector.project_record(&record) {
                        last_sent_cursor = Some(cursor.clone());
                        yield Ok::<Event, Infallible>(to_conversation_sse_event(make_conversation_envelope(
                            session_id_for_stream.as_str(),
                            cursor.as_str(),
                            delta,
                        )));
                    }

                    let Ok(current_control) = app.terminal_control_facts(&session_id_for_stream).await else {
                        log::warn!("conversation stream control refresh failed for session '{}'", session_id_for_stream);
                        break;
                    };
                    for delta in project_terminal_control_deltas(&cached_control, &current_control) {
                        last_sent_cursor = Some(cursor.clone());
                        yield Ok::<Event, Infallible>(to_conversation_sse_event(make_conversation_envelope(
                            session_id_for_stream.as_str(),
                            cursor.as_str(),
                            delta,
                        )));
                    }
                    cached_control = current_control;

                    let Ok(current_children) = app.conversation_child_summaries(&session_id_for_stream, &focus).await else {
                        log::warn!("conversation stream child summary refresh failed for session '{}'", session_id_for_stream);
                        break;
                    };
                    for delta in project_terminal_child_summary_deltas(&cached_children, &current_children) {
                        last_sent_cursor = Some(cursor.clone());
                        yield Ok::<Event, Infallible>(to_conversation_sse_event(make_conversation_envelope(
                            session_id_for_stream.as_str(),
                            cursor.as_str(),
                            delta,
                        )));
                    }
                    cached_children = current_children;

                    let Ok(current_slash_candidates) = app.terminal_slash_candidates(&session_id_for_stream, None).await else {
                        log::warn!(
                            "terminal stream slash candidate refresh failed for session '{}'",
                            session_id_for_stream
                        );
                        break;
                    };
                    if cached_slash_candidates != current_slash_candidates {
                        last_sent_cursor = Some(cursor.clone());
                        yield Ok::<Event, Infallible>(to_conversation_sse_event(make_conversation_envelope(
                            session_id_for_stream.as_str(),
                            cursor.as_str(),
                            ConversationDeltaDto::ReplaceSlashCandidates {
                                candidates: project_terminal_slash_candidates(&current_slash_candidates).items,
                            },
                        )));
                    }
                    cached_slash_candidates = current_slash_candidates;
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    log::debug!(
                        "conversation stream lagged by {} events for session '{}'",
                        skipped,
                        session_id_for_stream
                    );
                    match app
                        .conversation_stream_facts(
                            &session_id_for_stream,
                            last_sent_cursor.as_deref(),
                            focus.clone(),
                        )
                        .await
                    {
                        Ok(TerminalStreamFacts::Replay(recovered)) => {
                            for envelope in project_terminal_stream_replay(&recovered, last_sent_cursor.as_deref()) {
                                last_sent_cursor = Some(envelope.cursor.0.clone());
                                yield Ok::<Event, Infallible>(to_conversation_sse_event(envelope));
                            }

                            let recovery_cursor = last_sent_cursor
                                .clone()
                                .unwrap_or_else(|| "0.0".to_string());
                            for delta in project_terminal_control_deltas(&cached_control, &recovered.control) {
                                yield Ok::<Event, Infallible>(to_conversation_sse_event(make_conversation_envelope(
                                    session_id_for_stream.as_str(),
                                    recovery_cursor.as_str(),
                                    delta,
                                )));
                            }
                            cached_control = recovered.control.clone();

                            for delta in project_terminal_child_summary_deltas(&cached_children, &recovered.child_summaries) {
                                yield Ok::<Event, Infallible>(to_conversation_sse_event(make_conversation_envelope(
                                    session_id_for_stream.as_str(),
                                    recovery_cursor.as_str(),
                                    delta,
                                )));
                            }
                            cached_children = recovered.child_summaries.clone();

                            if cached_slash_candidates != recovered.slash_candidates {
                                yield Ok::<Event, Infallible>(to_conversation_sse_event(make_conversation_envelope(
                                    session_id_for_stream.as_str(),
                                    recovery_cursor.as_str(),
                                    ConversationDeltaDto::ReplaceSlashCandidates {
                                        candidates: project_terminal_slash_candidates(&recovered.slash_candidates).items,
                                    },
                                )));
                            }
                            cached_slash_candidates = recovered.slash_candidates.clone();
                            projector = seeded_terminal_stream_projector(&recovered);
                            replay = recovered.replay;
                        }
                        Ok(TerminalStreamFacts::RehydrateRequired(rehydrate)) => {
                            yield Ok::<Event, Infallible>(to_conversation_sse_event(
                                project_terminal_rehydrate_envelope(&rehydrate),
                            ));
                            break;
                        }
                        Err(error) => {
                            log::warn!(
                                "conversation stream recovery failed for session '{}': {}",
                                session_id_for_stream,
                                error
                            );
                            break;
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    break;
                }
            }
        }
    };

    Sse::new(Box::pin(event_stream) as ConversationEventStream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keepalive"),
    )
}

fn project_terminal_control_deltas(
    previous: &TerminalControlFacts,
    current: &TerminalControlFacts,
) -> Vec<ConversationDeltaDto> {
    let previous = control_state_delta(previous);
    let current = control_state_delta(current);
    if previous == current {
        Vec::new()
    } else {
        vec![current]
    }
}

fn control_state_delta(control: &TerminalControlFacts) -> ConversationDeltaDto {
    project_terminal_control_delta(control)
}

fn single_envelope_stream(envelope: ConversationStreamEnvelopeDto) -> ConversationSse {
    let event_stream = stream! {
        yield Ok::<Event, Infallible>(to_conversation_sse_event(envelope));
    };

    Sse::new(Box::pin(event_stream) as ConversationEventStream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keepalive"),
    )
}

fn make_conversation_envelope(
    session_id: &str,
    cursor: &str,
    delta: ConversationDeltaDto,
) -> ConversationStreamEnvelopeDto {
    ConversationStreamEnvelopeDto {
        session_id: session_id.to_string(),
        cursor: astrcode_protocol::http::conversation::v1::ConversationCursorDto(
            cursor.to_string(),
        ),
        delta,
    }
}

fn to_conversation_sse_event(envelope: ConversationStreamEnvelopeDto) -> Event {
    let payload = serde_json::to_string(&envelope).unwrap_or_else(|error| {
        serde_json::json!({
            "sessionId": envelope.session_id,
            "cursor": envelope.cursor.0,
            "kind": "set_banner",
            "banner": {
                "error": {
                    "code": "stream_disconnected",
                    "message": format!("failed to serialize conversation delta: {error}"),
                    "rehydrateRequired": false
                }
            }
        })
        .to_string()
    });

    Event::default()
        .id(envelope.cursor.0)
        .event("message")
        .data(payload)
}

fn parse_focus_query(raw: Option<&str>) -> Result<ConversationFocus, ConversationRouteError> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(ConversationFocus::Root);
    };
    if raw.eq_ignore_ascii_case("root") {
        return Ok(ConversationFocus::Root);
    }
    let Some(sub_run_id) = raw.strip_prefix("subrun:") else {
        return Err(ConversationRouteError::invalid_request(
            format!("invalid focus '{raw}'"),
            None,
        ));
    };
    let sub_run_id = validate_session_path_id(sub_run_id)
        .map_err(|error| ConversationRouteError::invalid_request(error.message, None))?;
    Ok(ConversationFocus::SubRun { sub_run_id })
}

type ConversationEventStream =
    Pin<Box<dyn futures_util::Stream<Item = Result<Event, Infallible>> + Send>>;
type ConversationSse = Sse<axum::response::sse::KeepAliveStream<ConversationEventStream>>;
