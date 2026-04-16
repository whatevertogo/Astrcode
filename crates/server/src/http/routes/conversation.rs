use std::{collections::HashMap, convert::Infallible, pin::Pin, time::Duration};

use astrcode_application::{
    ApplicationError, ConversationFocus, TerminalChildSummaryFacts, TerminalControlFacts,
    TerminalSlashCandidateFacts, TerminalStreamFacts, TerminalStreamReplayFacts,
};
use astrcode_core::AgentEvent;
use astrcode_protocol::http::conversation::v1::{
    ConversationChildSummaryDto, ConversationDeltaDto, ConversationSlashCandidatesResponseDto,
    ConversationSnapshotResponseDto, ConversationStreamEnvelopeDto,
};
use astrcode_session_runtime::ConversationStreamProjector as RuntimeConversationStreamProjector;
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
        project_child_summary, project_conversation_child_summary_deltas,
        project_conversation_control_delta, project_conversation_frame,
        project_conversation_rehydrate_envelope, project_conversation_slash_candidates,
        project_conversation_snapshot,
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

    Ok(Json(project_conversation_snapshot(&facts)))
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
            project_conversation_rehydrate_envelope(&rehydrate),
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

    Ok(Json(project_conversation_slash_candidates(&candidates)))
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
    let mut durable_receiver = facts.replay.replay.receiver;
    let mut live_receiver = facts.replay.replay.live_receiver;
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
                                durable_receiver = recovered.replay.replay.receiver;
                                live_receiver = recovered.replay.replay.live_receiver;
                                live_receiver_open = true;
                            }
                            Ok(TerminalStreamFacts::RehydrateRequired(rehydrate)) => {
                                yield Ok::<Event, Infallible>(to_conversation_sse_event(
                                    project_conversation_rehydrate_envelope(&rehydrate),
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

fn project_conversation_control_deltas(
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
    projector: RuntimeConversationStreamProjector,
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
            projector: RuntimeConversationStreamProjector::new(last_sent_cursor, &facts.replay),
            control: facts.control.clone(),
            child_summaries: facts.child_summaries.clone(),
            slash_candidates: facts.slash_candidates.clone(),
        }
    }

    fn last_sent_cursor(&self) -> Option<&str> {
        self.projector.last_sent_cursor()
    }

    fn seed_initial_replay(
        &mut self,
        facts: &TerminalStreamReplayFacts,
    ) -> Vec<ConversationStreamEnvelopeDto> {
        let child_lookup = child_summary_lookup(&facts.child_summaries);
        let envelopes = self
            .projector
            .seed_initial_replay(&facts.replay)
            .into_iter()
            .map(|frame| project_conversation_frame(self.session_id.as_str(), frame, &child_lookup))
            .collect::<Vec<_>>();
        let _ = self.projector.recover_from(&facts.replay);
        envelopes
    }

    fn project_durable_record(
        &mut self,
        record: &astrcode_core::SessionEventRecord,
    ) -> Vec<ConversationStreamEnvelopeDto> {
        let child_lookup = child_summary_lookup(&self.child_summaries);
        self.projector
            .project_durable_record(record)
            .into_iter()
            .map(|frame| project_conversation_frame(self.session_id.as_str(), frame, &child_lookup))
            .collect()
    }

    fn project_live_event(&mut self, event: &AgentEvent) -> Vec<ConversationStreamEnvelopeDto> {
        self.projector
            .project_live_event(event)
            .into_iter()
            .map(|frame| {
                project_conversation_frame(
                    self.session_id.as_str(),
                    frame,
                    &child_summary_lookup(&self.child_summaries),
                )
            })
            .collect()
    }

    fn apply_authoritative_refresh(
        &mut self,
        cursor: &str,
        refreshed: ConversationAuthoritativeFacts,
    ) -> Vec<ConversationStreamEnvelopeDto> {
        let mut deltas = project_conversation_control_deltas(&self.control, &refreshed.control);
        deltas.extend(project_conversation_child_summary_deltas(
            &self.child_summaries,
            &refreshed.child_summaries,
        ));
        if self.slash_candidates != refreshed.slash_candidates {
            deltas.push(ConversationDeltaDto::ReplaceSlashCandidates {
                candidates: project_conversation_slash_candidates(&refreshed.slash_candidates)
                    .items,
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
        let child_lookup = child_summary_lookup(&recovered.child_summaries);
        let mut envelopes = self
            .projector
            .recover_from(&recovered.replay)
            .into_iter()
            .map(|frame| project_conversation_frame(self.session_id.as_str(), frame, &child_lookup))
            .collect::<Vec<_>>();

        let recovery_cursor = self
            .projector
            .last_sent_cursor()
            .unwrap_or("0.0")
            .to_string();
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
        deltas
            .into_iter()
            .map(|delta| {
                make_conversation_envelope(self.session_id.as_str(), cursor_owned.as_str(), delta)
            })
            .collect()
    }
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

fn child_summary_lookup(
    summaries: &[TerminalChildSummaryFacts],
) -> HashMap<String, ConversationChildSummaryDto> {
    let mut lookup = HashMap::new();
    for summary in summaries {
        let dto = project_child_summary(summary);
        lookup.insert(summary.node.child_session_id.to_string(), dto.clone());
        if let Some(child_ref) = &dto.child_ref {
            lookup.insert(child_ref.open_session_id.clone(), dto.clone());
            lookup.insert(child_ref.session_id.clone(), dto.clone());
        }
    }
    lookup
}

fn control_state_delta(control: &TerminalControlFacts) -> ConversationDeltaDto {
    project_conversation_control_delta(control)
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

#[cfg(test)]
mod tests {
    use astrcode_application::{
        TerminalChildSummaryFacts, TerminalControlFacts, TerminalStreamReplayFacts,
    };
    use astrcode_core::{
        AgentEventContext, AgentLifecycleStatus, ChildExecutionIdentity, ChildSessionLineageKind,
        ChildSessionNode, ChildSessionStatusSource, ParentExecutionRef, Phase, SessionEventRecord,
        ToolExecutionResult, ToolOutputStream,
    };
    use astrcode_session_runtime::{
        ConversationBlockPatchFacts, ConversationDeltaFacts, ConversationDeltaFrameFacts,
        ConversationStreamReplayFacts as RuntimeConversationStreamReplayFacts, SessionReplay,
    };
    use serde_json::json;
    use tokio::sync::broadcast;

    use super::{AgentEvent, ConversationAuthoritativeFacts, ConversationStreamProjectorState};

    #[test]
    fn recover_from_replays_only_missing_records_and_advances_cursor() {
        let initial = sample_stream_facts(
            vec![record(
                "1.1",
                AgentEvent::UserMessage {
                    turn_id: "turn-1".to_string(),
                    agent: sample_agent_context(),
                    content: "check pipeline".to_string(),
                },
            )],
            vec![record(
                "1.2",
                AgentEvent::ToolCallStart {
                    turn_id: "turn-1".to_string(),
                    agent: sample_agent_context(),
                    tool_call_id: "call-1".to_string(),
                    tool_name: "shell_command".to_string(),
                    input: json!({ "command": "pwd" }),
                },
            )],
        );
        let mut state = ConversationStreamProjectorState::new(
            "session-root".to_string(),
            Some("1.1".to_string()),
            &initial,
        );

        let initial_envelopes = state.seed_initial_replay(&initial);
        assert_eq!(initial_envelopes.len(), 1);
        assert_eq!(initial_envelopes[0].cursor.0, "1.2");

        let recovered = sample_stream_facts(
            vec![
                record(
                    "1.1",
                    AgentEvent::UserMessage {
                        turn_id: "turn-1".to_string(),
                        agent: sample_agent_context(),
                        content: "check pipeline".to_string(),
                    },
                ),
                record(
                    "1.2",
                    AgentEvent::ToolCallStart {
                        turn_id: "turn-1".to_string(),
                        agent: sample_agent_context(),
                        tool_call_id: "call-1".to_string(),
                        tool_name: "shell_command".to_string(),
                        input: json!({ "command": "pwd" }),
                    },
                ),
            ],
            vec![record(
                "1.3",
                AgentEvent::ToolCallDelta {
                    turn_id: "turn-1".to_string(),
                    agent: sample_agent_context(),
                    tool_call_id: "call-1".to_string(),
                    tool_name: "shell_command".to_string(),
                    stream: ToolOutputStream::Stdout,
                    delta: "D:/GitObjectsOwn/Astrcode\n".to_string(),
                },
            )],
        );

        let recovered_envelopes = state.recover_from(&recovered);
        assert_eq!(recovered_envelopes.len(), 1);
        assert_eq!(recovered_envelopes[0].cursor.0, "1.3");
        assert_eq!(
            serde_json::to_value(&recovered_envelopes[0])
                .expect("recovered envelope should encode"),
            json!({
                "sessionId": "session-root",
                "cursor": "1.3",
                "kind": "patch_block",
                "blockId": "tool:call-1:call",
                "patch": {
                    "kind": "append_tool_stream",
                    "stream": "stdout",
                    "chunk": "D:/GitObjectsOwn/Astrcode\n"
                }
            })
        );

        let live_envelopes = state.project_live_event(&AgentEvent::ToolCallResult {
            turn_id: "turn-1".to_string(),
            agent: sample_agent_context(),
            result: ToolExecutionResult {
                tool_call_id: "call-1".to_string(),
                tool_name: "shell_command".to_string(),
                ok: true,
                output: "D:/GitObjectsOwn/Astrcode\n".to_string(),
                child_ref: None,
                error: None,
                metadata: None,
                duration_ms: 8,
                truncated: false,
            },
        });
        assert!(
            live_envelopes
                .iter()
                .all(|envelope| envelope.cursor.0 == "1.3"),
            "live cursor should stay anchored to last durable cursor after recovery"
        );
    }

    #[test]
    fn authoritative_refresh_emits_child_summary_delta_on_current_cursor() {
        let facts = sample_stream_facts(Vec::new(), Vec::new());
        let mut state = ConversationStreamProjectorState::new(
            "session-root".to_string(),
            Some("1.4".to_string()),
            &facts,
        );

        let refreshed = ConversationAuthoritativeFacts {
            control: facts.control.clone(),
            child_summaries: vec![sample_child_summary()],
            slash_candidates: facts.slash_candidates.clone(),
        };

        let envelopes = state.apply_authoritative_refresh("1.4", refreshed);
        assert_eq!(envelopes.len(), 1);
        assert_eq!(
            serde_json::to_value(&envelopes[0]).expect("child summary envelope should encode"),
            json!({
                "sessionId": "session-root",
                "cursor": "1.4",
                "kind": "upsert_child_summary",
                "child": {
                    "childSessionId": "session-child-1",
                    "childAgentId": "agent-child-1",
                    "title": "Repo inspector",
                    "lifecycle": "running",
                    "latestOutputSummary": "正在检查 conversation projector",
                    "childRef": {
                        "agentId": "agent-child-1",
                        "sessionId": "session-root",
                        "subRunId": "subrun-child-1",
                        "parentAgentId": "agent-root",
                        "parentSubRunId": "subrun-root",
                        "lineageKind": "spawn",
                        "status": "running",
                        "openSessionId": "session-child-1"
                    }
                }
            })
        );
    }

    fn sample_stream_facts(
        seed_records: Vec<SessionEventRecord>,
        history: Vec<SessionEventRecord>,
    ) -> TerminalStreamReplayFacts {
        let (_, receiver) = broadcast::channel(8);
        let (_, live_receiver) = broadcast::channel(8);

        TerminalStreamReplayFacts {
            active_session_id: "session-root".to_string(),
            replay: RuntimeConversationStreamReplayFacts {
                cursor: history.last().map(|record| record.event_id.clone()),
                phase: Phase::CallingTool,
                seed_records: seed_records.clone(),
                replay_frames: history
                    .iter()
                    .map(|record| ConversationDeltaFrameFacts {
                        cursor: record.event_id.clone(),
                        delta: match &record.event {
                            AgentEvent::ToolCallDelta {
                                tool_call_id,
                                stream,
                                delta,
                                ..
                            } => ConversationDeltaFacts::PatchBlock {
                                block_id: format!("tool:{tool_call_id}:call"),
                                patch: ConversationBlockPatchFacts::AppendToolStream {
                                    stream: *stream,
                                    chunk: delta.clone(),
                                },
                            },
                            _ => ConversationDeltaFacts::AppendBlock {
                                block: astrcode_session_runtime::ConversationBlockFacts::User(
                                    astrcode_session_runtime::ConversationUserBlockFacts {
                                        id: "noop".to_string(),
                                        turn_id: None,
                                        markdown: String::new(),
                                    },
                                ),
                            },
                        },
                    })
                    .collect(),
                replay: SessionReplay {
                    history,
                    receiver,
                    live_receiver,
                },
            },
            control: TerminalControlFacts {
                phase: Phase::CallingTool,
                active_turn_id: Some("turn-1".to_string()),
                manual_compact_pending: false,
            },
            child_summaries: Vec::new(),
            slash_candidates: Vec::new(),
        }
    }

    fn sample_child_summary() -> TerminalChildSummaryFacts {
        TerminalChildSummaryFacts {
            node: ChildSessionNode {
                identity: ChildExecutionIdentity {
                    agent_id: "agent-child-1".into(),
                    session_id: "session-root".into(),
                    sub_run_id: "subrun-child-1".into(),
                },
                child_session_id: "session-child-1".into(),
                parent_session_id: "session-root".into(),
                parent: ParentExecutionRef {
                    parent_agent_id: Some("agent-root".into()),
                    parent_sub_run_id: Some("subrun-root".into()),
                },
                parent_turn_id: "turn-1".into(),
                lineage_kind: ChildSessionLineageKind::Spawn,
                status: AgentLifecycleStatus::Running,
                status_source: ChildSessionStatusSource::Durable,
                created_by_tool_call_id: Some("call-2".into()),
                lineage_snapshot: None,
            },
            phase: Phase::CallingTool,
            title: Some("Repo inspector".to_string()),
            display_name: Some("repo-inspector".to_string()),
            recent_output: Some("正在检查 conversation projector".to_string()),
        }
    }

    fn sample_agent_context() -> AgentEventContext {
        AgentEventContext::root_execution("agent-root", "default")
    }

    fn record(event_id: &str, event: AgentEvent) -> SessionEventRecord {
        SessionEventRecord {
            event_id: event_id.to_string(),
            event,
        }
    }
}
