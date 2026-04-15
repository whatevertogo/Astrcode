use std::{convert::Infallible, pin::Pin, time::Duration};

use astrcode_application::{
    ApplicationError, ConversationFocus, TerminalChildSummaryFacts, TerminalControlFacts,
    TerminalSlashCandidateFacts, TerminalStreamFacts, TerminalStreamReplayFacts,
};
use astrcode_core::{AgentEvent, SessionEventRecord};
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
        TerminalDeltaProjector, project_terminal_child_summary_deltas,
        project_terminal_control_delta, project_terminal_rehydrate_envelope,
        project_terminal_slash_candidates, project_terminal_snapshot,
        project_terminal_stream_replay, seeded_terminal_stream_projector,
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
    facts: TerminalStreamReplayFacts,
) -> ConversationSse {
    let mut stream_state =
        ConversationStreamProjectorState::new(session_id.clone(), cursor, &facts);
    let initial_envelopes = stream_state.seed_initial_replay(&facts);
    let mut durable_receiver = facts.replay.receiver;
    let mut live_receiver = facts.replay.live_receiver;
    let app = state.app.clone();
    let session_id_for_stream = session_id.clone();
    let mut live_receiver_open = true;

    let event_stream = stream! {
        for envelope in initial_envelopes {
            yield Ok::<Event, Infallible>(to_conversation_sse_event(envelope));
        }

        loop {
            // Why: durable replay 负责恢复/补放的权威事实，live receiver 只负责 token 级即时体验。
            // 两者共用同一个 projector，这样前端只需要维护一套 terminal/conversation block 语义。
            tokio::select! {
                durable = durable_receiver.recv() => match durable {
                    Ok(record) => {
                        for envelope in stream_state.project_durable_record(&record) {
                            yield Ok::<Event, Infallible>(to_conversation_sse_event(envelope));
                        }

                        let Ok(refreshed_facts) = refresh_conversation_authoritative_facts(
                            &app,
                            &session_id_for_stream,
                            &focus,
                        ).await else {
                            log::warn!(
                                "conversation stream authoritative refresh failed for session '{}'",
                                session_id_for_stream
                            );
                            break;
                        };
                        for envelope in stream_state.apply_authoritative_refresh(
                            record.event_id.as_str(),
                            refreshed_facts,
                        ) {
                            yield Ok::<Event, Infallible>(to_conversation_sse_event(envelope));
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        log::debug!(
                            "conversation stream lagged by {} durable events for session '{}'",
                            skipped,
                            session_id_for_stream
                        );
                        match app
                            .conversation_stream_facts(
                                &session_id_for_stream,
                                stream_state.last_sent_cursor(),
                                focus.clone(),
                            )
                            .await
                        {
                            Ok(TerminalStreamFacts::Replay(recovered)) => {
                                for envelope in stream_state.recover_from(&recovered) {
                                    yield Ok::<Event, Infallible>(to_conversation_sse_event(envelope));
                                }
                                durable_receiver = recovered.replay.receiver;
                                live_receiver = recovered.replay.live_receiver;
                                live_receiver_open = true;
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
                },
                live = live_receiver.recv(), if live_receiver_open => match live {
                    Ok(event) => {
                        for envelope in stream_state.project_live_event(&event) {
                            yield Ok::<Event, Infallible>(to_conversation_sse_event(envelope));
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        log::debug!(
                            "conversation stream lagged by {} live events for session '{}'",
                            skipped,
                            session_id_for_stream
                        );
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        live_receiver_open = false;
                    }
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

#[derive(Debug, Clone)]
struct ConversationAuthoritativeFacts {
    control: TerminalControlFacts,
    child_summaries: Vec<TerminalChildSummaryFacts>,
    slash_candidates: Vec<TerminalSlashCandidateFacts>,
}

impl ConversationAuthoritativeFacts {
    fn from_replay(facts: &TerminalStreamReplayFacts) -> Self {
        Self {
            control: facts.control.clone(),
            child_summaries: facts.child_summaries.clone(),
            slash_candidates: facts.slash_candidates.clone(),
        }
    }
}

struct ConversationStreamProjectorState {
    session_id: String,
    projector: TerminalDeltaProjector,
    last_sent_cursor: Option<String>,
    fallback_live_cursor: Option<String>,
    control: TerminalControlFacts,
    child_summaries: Vec<TerminalChildSummaryFacts>,
    slash_candidates: Vec<TerminalSlashCandidateFacts>,
}

impl ConversationStreamProjectorState {
    fn new(
        session_id: String,
        last_sent_cursor: Option<String>,
        facts: &TerminalStreamReplayFacts,
    ) -> Self {
        Self {
            session_id,
            projector: seeded_terminal_stream_projector(facts),
            last_sent_cursor,
            fallback_live_cursor: fallback_live_cursor(facts),
            control: facts.control.clone(),
            child_summaries: facts.child_summaries.clone(),
            slash_candidates: facts.slash_candidates.clone(),
        }
    }

    fn last_sent_cursor(&self) -> Option<&str> {
        self.last_sent_cursor.as_deref()
    }

    fn seed_initial_replay(
        &mut self,
        facts: &TerminalStreamReplayFacts,
    ) -> Vec<ConversationStreamEnvelopeDto> {
        let envelopes = project_terminal_stream_replay(facts, self.last_sent_cursor.as_deref());
        self.observe_durable_envelopes(&envelopes);
        envelopes
    }

    fn project_durable_record(
        &mut self,
        record: &SessionEventRecord,
    ) -> Vec<ConversationStreamEnvelopeDto> {
        let deltas = self.projector.project_record(record);
        self.wrap_durable_deltas(record.event_id.as_str(), deltas)
    }

    fn project_live_event(&mut self, event: &AgentEvent) -> Vec<ConversationStreamEnvelopeDto> {
        let cursor = self.live_cursor();
        self.projector
            .project_live_event(event)
            .into_iter()
            .map(|delta| {
                make_conversation_envelope(self.session_id.as_str(), cursor.as_str(), delta)
            })
            .collect()
    }

    fn apply_authoritative_refresh(
        &mut self,
        cursor: &str,
        refreshed: ConversationAuthoritativeFacts,
    ) -> Vec<ConversationStreamEnvelopeDto> {
        let mut deltas = project_terminal_control_deltas(&self.control, &refreshed.control);
        deltas.extend(project_terminal_child_summary_deltas(
            &self.child_summaries,
            &refreshed.child_summaries,
        ));
        if self.slash_candidates != refreshed.slash_candidates {
            deltas.push(ConversationDeltaDto::ReplaceSlashCandidates {
                candidates: project_terminal_slash_candidates(&refreshed.slash_candidates).items,
            });
        }

        self.control = refreshed.control;
        self.child_summaries = refreshed.child_summaries;
        self.slash_candidates = refreshed.slash_candidates;
        self.wrap_durable_deltas(cursor, deltas)
    }

    fn recover_from(
        &mut self,
        recovered: &TerminalStreamReplayFacts,
    ) -> Vec<ConversationStreamEnvelopeDto> {
        let mut envelopes =
            project_terminal_stream_replay(recovered, self.last_sent_cursor.as_deref());
        self.observe_durable_envelopes(&envelopes);
        self.projector = seeded_terminal_stream_projector(recovered);
        self.fallback_live_cursor = fallback_live_cursor(recovered);

        let recovery_cursor = self.live_cursor();
        envelopes.extend(self.apply_authoritative_refresh(
            recovery_cursor.as_str(),
            ConversationAuthoritativeFacts::from_replay(recovered),
        ));
        envelopes
    }

    fn wrap_durable_deltas(
        &mut self,
        cursor: &str,
        deltas: Vec<ConversationDeltaDto>,
    ) -> Vec<ConversationStreamEnvelopeDto> {
        if deltas.is_empty() {
            return Vec::new();
        }
        let cursor_owned = cursor.to_string();
        self.last_sent_cursor = Some(cursor_owned.clone());
        deltas
            .into_iter()
            .map(|delta| {
                make_conversation_envelope(self.session_id.as_str(), cursor_owned.as_str(), delta)
            })
            .collect()
    }

    fn observe_durable_envelopes(&mut self, envelopes: &[ConversationStreamEnvelopeDto]) {
        if let Some(cursor) = envelopes.last().map(|envelope| envelope.cursor.0.clone()) {
            self.last_sent_cursor = Some(cursor);
        }
    }

    fn live_cursor(&self) -> String {
        self.last_sent_cursor
            .clone()
            .or_else(|| self.fallback_live_cursor.clone())
            .unwrap_or_else(|| "0.0".to_string())
    }
}

fn fallback_live_cursor(facts: &TerminalStreamReplayFacts) -> Option<String> {
    facts
        .seed_records
        .last()
        .map(|record| record.event_id.clone())
        .or_else(|| {
            facts
                .replay
                .history
                .last()
                .map(|record| record.event_id.clone())
        })
}

async fn refresh_conversation_authoritative_facts(
    app: &astrcode_application::App,
    session_id: &str,
    focus: &ConversationFocus,
) -> Result<ConversationAuthoritativeFacts, ApplicationError> {
    Ok(ConversationAuthoritativeFacts {
        control: app.terminal_control_facts(session_id).await?,
        child_summaries: app.conversation_child_summaries(session_id, focus).await?,
        slash_candidates: app.terminal_slash_candidates(session_id, None).await?,
    })
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
