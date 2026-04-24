use std::{
    convert::Infallible,
    path::{Path as FsPath, PathBuf},
    pin::Pin,
    time::Duration,
};

use astrcode_core::{AgentEvent, Phase, SessionId};
use astrcode_host_session::ComposerOptionKind;
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
    application_error_bridge::ServerRouteError,
    auth::is_authorized,
    composer_catalog::list_session_composer_options,
    conversation_read_model::{
        ConversationReplayStream, ConversationStreamProjector, ConversationStreamReplayFacts,
        ROOT_AGENT_ID,
    },
    routes::sessions::validate_session_path_id,
    terminal_projection::{
        ConversationAuthoritativeSummary, ConversationChildSummarySummary,
        ConversationControlSummary, ConversationFocus, ConversationSlashCandidateSummary,
        TaskItemFacts, TerminalChildSummaryFacts, TerminalControlFacts, TerminalFacts,
        TerminalRehydrateFacts, TerminalRehydrateReason, TerminalSlashAction,
        TerminalSlashCandidateFacts, TerminalStreamFacts, TerminalStreamReplayFacts,
        build_conversation_replay_frames, build_conversation_snapshot,
        child_summary_summary_lookup, latest_terminal_summary, map_control_facts,
        project_conversation_child_summary_summary_deltas,
        project_conversation_control_summary_delta, project_conversation_frame,
        project_conversation_rehydrate_envelope, project_conversation_slash_candidate_summaries,
        project_conversation_slash_candidates, project_conversation_snapshot,
        project_conversation_step_progress, summarize_conversation_authoritative,
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

impl From<ServerRouteError> for ConversationRouteError {
    fn from(value: ServerRouteError) -> Self {
        match value {
            ServerRouteError::NotFound(message) => Self {
                status: StatusCode::NOT_FOUND,
                code: "not_found",
                message,
                details: None,
            },
            ServerRouteError::Conflict(message) => Self {
                status: StatusCode::CONFLICT,
                code: "conflict",
                message,
                details: None,
            },
            ServerRouteError::InvalidArgument(message) => Self::invalid_request(message, None),
            ServerRouteError::PermissionDenied(message) => Self {
                status: StatusCode::FORBIDDEN,
                code: "forbidden",
                message,
                details: None,
            },
            ServerRouteError::Internal(message) => Self {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                code: "internal_error",
                message,
                details: None,
            },
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
    let facts = build_terminal_snapshot_facts(&state, &session_id, &focus).await?;

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

    let stream_facts =
        build_terminal_stream_facts(&state, &session_id, cursor.as_deref(), &focus).await?;

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
    let candidates = terminal_slash_candidates(&state, &session_id, query.q.as_deref()).await?;

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
    let mut durable_receiver = facts.stream.receiver;
    let mut live_receiver = facts.stream.live_receiver;
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
                            &state,
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
                        match build_terminal_stream_facts(
                            &state,
                            &session_id_for_stream,
                            stream_state.last_sent_cursor(),
                            &focus,
                        ).await {
                            Ok(TerminalStreamFacts::Replay(recovered)) => {
                                for envelope in stream_state.recover_from(&recovered) {
                                    yield Ok::<Event, Infallible>(to_conversation_sse_event(envelope));
                                }
                                durable_receiver = recovered.stream.receiver;
                                live_receiver = recovered.stream.live_receiver;
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
                                    "conversation stream recovery failed for session '{}': {:?}",
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

#[derive(Debug, Clone)]
struct ConversationAuthoritativeFacts {
    control: ConversationControlSummary,
    child_summaries: Vec<ConversationChildSummarySummary>,
    slash_candidates: Vec<ConversationSlashCandidateSummary>,
}

impl ConversationAuthoritativeFacts {
    fn from_replay(facts: &TerminalStreamReplayFacts) -> Self {
        Self::from_summary(summarize_conversation_authoritative(
            &facts.control,
            &facts.child_summaries,
            &facts.slash_candidates,
        ))
    }

    fn from_summary(summary: ConversationAuthoritativeSummary) -> Self {
        Self {
            control: summary.control,
            child_summaries: summary.child_summaries,
            slash_candidates: summary.slash_candidates,
        }
    }
}

struct ConversationStreamProjectorState {
    session_id: String,
    projector: ConversationStreamProjector,
    authoritative: ConversationAuthoritativeFacts,
}

impl ConversationStreamProjectorState {
    fn new(
        session_id: String,
        last_sent_cursor: Option<String>,
        facts: &TerminalStreamReplayFacts,
    ) -> Self {
        Self {
            session_id,
            projector: ConversationStreamProjector::new(last_sent_cursor, &facts.replay),
            authoritative: ConversationAuthoritativeFacts::from_replay(facts),
        }
    }

    fn last_sent_cursor(&self) -> Option<&str> {
        self.projector.last_sent_cursor()
    }

    fn seed_initial_replay(
        &mut self,
        facts: &TerminalStreamReplayFacts,
    ) -> Vec<ConversationStreamEnvelopeDto> {
        let child_lookup = child_summary_summary_lookup(&self.authoritative.child_summaries);
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
        let child_lookup = child_summary_summary_lookup(&self.authoritative.child_summaries);
        self.projector
            .project_durable_record(record)
            .into_iter()
            .map(|frame| project_conversation_frame(self.session_id.as_str(), frame, &child_lookup))
            .collect()
    }

    fn project_live_event(&mut self, event: &AgentEvent) -> Vec<ConversationStreamEnvelopeDto> {
        let mut envelopes = self
            .projector
            .project_live_event(event)
            .into_iter()
            .map(|frame| {
                project_conversation_frame(
                    self.session_id.as_str(),
                    frame,
                    &child_summary_summary_lookup(&self.authoritative.child_summaries),
                )
            })
            .collect::<Vec<_>>();
        if let Some(control) = self.live_control_overlay(event) {
            let cursor = self
                .projector
                .last_sent_cursor()
                .unwrap_or("0.0")
                .to_string();
            envelopes.extend(self.wrap_durable_deltas(
                cursor.as_str(),
                vec![project_conversation_control_summary_delta(&control)],
            ));
        }
        envelopes
    }

    fn live_control_overlay(&self, event: &AgentEvent) -> Option<ConversationControlSummary> {
        let (phase, active_turn_id) = match event {
            AgentEvent::ThinkingDelta { turn_id, .. }
            | AgentEvent::ModelDelta { turn_id, .. }
            | AgentEvent::StreamRetryStarted { turn_id, .. } => {
                (Phase::Streaming, Some(turn_id.clone()))
            },
            AgentEvent::ToolCallStart { turn_id, .. }
            | AgentEvent::ToolCallDelta { turn_id, .. }
            | AgentEvent::ToolCallResult { turn_id, .. } => {
                (Phase::CallingTool, Some(turn_id.clone()))
            },
            AgentEvent::TurnDone { .. } | AgentEvent::Error { .. } => (Phase::Idle, None),
            _ => return None,
        };
        let mut control = self.authoritative.control.clone();
        control.phase = phase;
        control.active_turn_id = active_turn_id;
        control.can_submit_prompt = control.active_turn_id.is_none()
            && matches!(phase, Phase::Idle | Phase::Done | Phase::Interrupted);
        Some(control)
    }

    fn apply_authoritative_refresh(
        &mut self,
        cursor: &str,
        refreshed: ConversationAuthoritativeFacts,
    ) -> Vec<ConversationStreamEnvelopeDto> {
        let mut deltas = if self.authoritative.control == refreshed.control {
            Vec::new()
        } else {
            vec![project_conversation_control_summary_delta(
                &refreshed.control,
            )]
        };
        deltas.extend(project_conversation_child_summary_summary_deltas(
            &self.authoritative.child_summaries,
            &refreshed.child_summaries,
        ));
        if self.authoritative.slash_candidates != refreshed.slash_candidates {
            deltas.push(ConversationDeltaDto::ReplaceSlashCandidates {
                candidates: project_conversation_slash_candidate_summaries(
                    &refreshed.slash_candidates,
                )
                .items,
            });
        }

        self.authoritative = refreshed;
        self.wrap_durable_deltas(cursor, deltas)
    }

    fn recover_from(
        &mut self,
        recovered: &TerminalStreamReplayFacts,
    ) -> Vec<ConversationStreamEnvelopeDto> {
        let refreshed = ConversationAuthoritativeFacts::from_replay(recovered);
        let child_lookup = child_summary_summary_lookup(&refreshed.child_summaries);
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
        envelopes.extend(self.apply_authoritative_refresh(recovery_cursor.as_str(), refreshed));
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
        let step_progress =
            project_conversation_step_progress(self.projector.step_progress().clone());
        deltas
            .into_iter()
            .map(|delta| {
                make_conversation_envelope(
                    self.session_id.as_str(),
                    cursor_owned.as_str(),
                    step_progress.clone(),
                    delta,
                )
            })
            .collect()
    }
}

async fn refresh_conversation_authoritative_facts(
    state: &AppState,
    session_id: &str,
    focus: &ConversationFocus,
) -> Result<ConversationAuthoritativeFacts, ConversationRouteError> {
    let control = terminal_control_facts(state, session_id).await?;
    let child_summaries = conversation_child_summaries(state, session_id, focus).await?;
    let slash_candidates = terminal_slash_candidates(state, session_id, None).await?;
    Ok(ConversationAuthoritativeFacts::from_summary(
        summarize_conversation_authoritative(&control, &child_summaries, &slash_candidates),
    ))
}

async fn build_terminal_snapshot_facts(
    state: &AppState,
    session_id: &str,
    focus: &ConversationFocus,
) -> Result<TerminalFacts, ConversationRouteError> {
    let focus_session_id = resolve_conversation_focus_session_id(state, session_id, focus).await?;
    let focus_session = state
        .session_catalog
        .ensure_loaded_session(&SessionId::from(focus_session_id.clone()))
        .await
        .map_err(|error| {
            ConversationRouteError::from(ServerRouteError::internal(error.to_string()))
        })?;
    let transcript = build_conversation_snapshot(
        &state
            .session_catalog
            .conversation_stream_replay(&SessionId::from(focus_session_id.clone()), None)
            .await
            .map_err(|error| {
                ConversationRouteError::from(ServerRouteError::internal(error.to_string()))
            })?
            .history,
        focus_session.state.current_phase().map_err(|error| {
            ConversationRouteError::from(ServerRouteError::internal(error.to_string()))
        })?,
    );
    let session_title = session_title(state, session_id).await?;
    let control = terminal_control_facts(state, session_id).await?;
    let child_summaries = conversation_child_summaries(state, session_id, focus).await?;
    let slash_candidates = terminal_slash_candidates(state, session_id, None).await?;

    Ok(TerminalFacts {
        active_session_id: session_id.to_string(),
        session_title,
        transcript,
        control,
        child_summaries,
        slash_candidates,
    })
}

async fn build_terminal_stream_facts(
    state: &AppState,
    session_id: &str,
    last_event_id: Option<&str>,
    focus: &ConversationFocus,
) -> Result<TerminalStreamFacts, ConversationRouteError> {
    let focus_session_id = resolve_conversation_focus_session_id(state, session_id, focus).await?;
    if let Some(requested_cursor) = last_event_id {
        validate_cursor_format(requested_cursor)?;
        let records = state
            .session_catalog
            .conversation_stream_replay(&SessionId::from(focus_session_id.clone()), None)
            .await
            .map_err(|error| {
                ConversationRouteError::from(ServerRouteError::internal(error.to_string()))
            })?
            .history;
        let latest_cursor = records.last().map(|record| record.event_id.clone());
        let cursor_missing_from_transcript = !records
            .iter()
            .any(|record| record.event_id == requested_cursor);
        if cursor_is_after_head(requested_cursor, latest_cursor.as_deref())?
            || cursor_missing_from_transcript
        {
            return Ok(TerminalStreamFacts::RehydrateRequired(
                TerminalRehydrateFacts {
                    session_id: session_id.to_string(),
                    requested_cursor: requested_cursor.to_string(),
                    latest_cursor,
                    reason: TerminalRehydrateReason::CursorExpired,
                },
            ));
        }
    }

    let replay = state
        .session_catalog
        .conversation_stream_replay(&SessionId::from(focus_session_id.clone()), last_event_id)
        .await
        .map_err(|error| {
            ConversationRouteError::from(ServerRouteError::internal(error.to_string()))
        })?;
    let loaded = state
        .session_catalog
        .ensure_loaded_session(&SessionId::from(focus_session_id))
        .await
        .map_err(|error| {
            ConversationRouteError::from(ServerRouteError::internal(error.to_string()))
        })?;
    let phase = loaded.state.current_phase().map_err(|error| {
        ConversationRouteError::from(ServerRouteError::internal(error.to_string()))
    })?;
    let replay_history = replay.history.clone();
    let seed_records = replay.seed_records.clone();
    let replay_facts = ConversationStreamReplayFacts {
        cursor: replay.cursor.clone(),
        phase,
        seed_records: seed_records.clone(),
        replay_frames: build_conversation_replay_frames(&seed_records, &replay_history),
        replay_history: replay_history.clone(),
    };
    let control = terminal_control_facts(state, session_id).await?;
    let child_summaries = conversation_child_summaries(state, session_id, focus).await?;
    let slash_candidates = terminal_slash_candidates(state, session_id, None).await?;

    Ok(TerminalStreamFacts::Replay(Box::new(
        TerminalStreamReplayFacts {
            replay: replay_facts,
            stream: ConversationReplayStream {
                receiver: loaded.state.broadcaster.subscribe(),
                live_receiver: loaded.state.subscribe_live(),
            },
            control,
            child_summaries,
            slash_candidates,
        },
    )))
}

async fn session_title(
    state: &AppState,
    session_id: &str,
) -> Result<String, ConversationRouteError> {
    state
        .session_catalog
        .list_session_metas()
        .await
        .map_err(|error| {
            ConversationRouteError::from(ServerRouteError::internal(error.to_string()))
        })?
        .into_iter()
        .find(|meta| meta.session_id == session_id)
        .map(|meta| meta.title)
        .ok_or_else(|| {
            ConversationRouteError::from(ServerRouteError::not_found(format!(
                "session '{}' not found",
                session_id
            )))
        })
}

async fn terminal_control_facts(
    state: &AppState,
    session_id: &str,
) -> Result<TerminalControlFacts, ConversationRouteError> {
    let session_id = SessionId::from(session_id.to_string());
    let mut facts = map_control_facts(
        state
            .session_catalog
            .session_control_state(&session_id)
            .await
            .map_err(|error| {
                ConversationRouteError::from(ServerRouteError::internal(error.to_string()))
            })?,
    );
    let loaded = state
        .session_catalog
        .ensure_loaded_session(&session_id)
        .await
        .map_err(|error| {
            ConversationRouteError::from(ServerRouteError::internal(error.to_string()))
        })?;
    facts.active_tasks = state
        .session_catalog
        .active_task_snapshot(&session_id, ROOT_AGENT_ID)
        .await
        .map_err(|error| {
            ConversationRouteError::from(ServerRouteError::internal(error.to_string()))
        })?
        .map(|snapshot| {
            snapshot
                .items
                .into_iter()
                .map(|item| TaskItemFacts {
                    content: item.content,
                    status: item.status,
                    active_form: item.active_form,
                })
                .collect()
        });
    facts.active_plan = active_plan_reference(session_id.as_str(), &loaded.working_dir)
        .map_err(ConversationRouteError::from)?;
    Ok(facts)
}

async fn conversation_child_summaries(
    state: &AppState,
    root_session_id: &str,
    focus: &ConversationFocus,
) -> Result<Vec<TerminalChildSummaryFacts>, ConversationRouteError> {
    let focus_session_id =
        resolve_conversation_focus_session_id(state, root_session_id, focus).await?;
    let children = state
        .session_catalog
        .session_child_nodes(&SessionId::from(focus_session_id.clone()))
        .await
        .map_err(|error| {
            ConversationRouteError::from(ServerRouteError::internal(error.to_string()))
        })?;
    let session_metas = state
        .session_catalog
        .list_session_metas()
        .await
        .map_err(|error| {
            ConversationRouteError::from(ServerRouteError::internal(error.to_string()))
        })?;

    let mut resolved = Vec::new();
    for node in children
        .into_iter()
        .filter(|node| node.parent_sub_run_id().is_none())
    {
        if node.parent_session_id.as_str() != focus_session_id {
            return Err(ConversationRouteError::from(
                ServerRouteError::permission_denied(format!(
                    "child '{}' is not visible from session '{}'",
                    node.sub_run_id(),
                    focus_session_id
                )),
            ));
        }
        let child_meta = session_metas
            .iter()
            .find(|meta| meta.session_id == node.child_session_id.as_str());
        let child_session_id = SessionId::from(node.child_session_id.to_string());
        let child_loaded = state
            .session_catalog
            .ensure_loaded_session(&child_session_id)
            .await
            .map_err(|error| {
                ConversationRouteError::from(ServerRouteError::internal(error.to_string()))
            })?;
        let child_transcript = build_conversation_snapshot(
            &state
                .session_catalog
                .conversation_stream_replay(&child_session_id, None)
                .await
                .map_err(|error| {
                    ConversationRouteError::from(ServerRouteError::internal(error.to_string()))
                })?
                .history,
            child_loaded.state.current_phase().map_err(|error| {
                ConversationRouteError::from(ServerRouteError::internal(error.to_string()))
            })?,
        );
        resolved.push(TerminalChildSummaryFacts {
            node,
            phase: child_transcript.phase,
            title: child_meta.map(|meta| meta.title.clone()),
            display_name: child_meta.map(|meta| meta.display_name.clone()),
            recent_output: latest_terminal_summary(&child_transcript),
        });
    }
    resolved.sort_by(|left, right| left.node.sub_run_id().cmp(right.node.sub_run_id()));
    Ok(resolved)
}

async fn terminal_slash_candidates(
    state: &AppState,
    session_id: &str,
    query: Option<&str>,
) -> Result<Vec<TerminalSlashCandidateFacts>, ConversationRouteError> {
    let query = normalize_query(query);
    let control = terminal_control_facts(state, session_id).await?;
    let mut candidates = terminal_builtin_candidates(&control);
    candidates.extend(
        list_session_composer_options(
            state,
            session_id,
            query.as_deref(),
            &[ComposerOptionKind::Skill],
            50,
        )
        .await
        .map_err(ConversationRouteError::from)?
        .into_iter()
        .map(|option| TerminalSlashCandidateFacts {
            id: option.id.clone(),
            title: option.title,
            description: option.description,
            keywords: option.keywords,
            badges: option.badges,
            action: TerminalSlashAction::InsertText {
                text: format!("/{}", option.id),
            },
        }),
    );

    if let Some(query) = query.as_deref() {
        candidates.retain(|candidate| slash_candidate_matches(candidate, query));
    }

    Ok(candidates)
}

async fn resolve_conversation_focus_session_id(
    state: &AppState,
    root_session_id: &str,
    focus: &ConversationFocus,
) -> Result<String, ConversationRouteError> {
    match focus {
        ConversationFocus::Root => Ok(root_session_id.to_string()),
        ConversationFocus::SubRun { sub_run_id } => {
            let mut pending = vec![root_session_id.to_string()];
            let mut visited = std::collections::HashSet::new();

            while let Some(session_id) = pending.pop() {
                if !visited.insert(session_id.clone()) {
                    continue;
                }
                for node in state
                    .session_catalog
                    .session_child_nodes(&SessionId::from(session_id.clone()))
                    .await
                    .map_err(|error| {
                        ConversationRouteError::from(ServerRouteError::internal(error.to_string()))
                    })?
                {
                    if node.sub_run_id().as_str() == sub_run_id {
                        return Ok(node.child_session_id.to_string());
                    }
                    pending.push(node.child_session_id.to_string());
                }
            }

            Err(ConversationRouteError::from(ServerRouteError::not_found(
                format!(
                    "sub-run '{}' not found under session '{}'",
                    sub_run_id, root_session_id
                ),
            )))
        },
    }
}

fn normalize_query(query: Option<&str>) -> Option<String> {
    query
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .map(|query| query.to_lowercase())
}

fn terminal_builtin_candidates(control: &TerminalControlFacts) -> Vec<TerminalSlashCandidateFacts> {
    let mut candidates = vec![
        TerminalSlashCandidateFacts {
            id: "new".to_string(),
            title: "新建会话".to_string(),
            description: "创建新 session 并切换焦点".to_string(),
            keywords: vec!["new".to_string(), "session".to_string()],
            badges: vec!["built-in".to_string()],
            action: TerminalSlashAction::CreateSession,
        },
        TerminalSlashCandidateFacts {
            id: "resume".to_string(),
            title: "恢复会话".to_string(),
            description: "搜索并切换到已有 session".to_string(),
            keywords: vec!["resume".to_string(), "switch".to_string()],
            badges: vec!["built-in".to_string()],
            action: TerminalSlashAction::OpenResume,
        },
    ];

    if !control.manual_compact_pending && !control.compacting {
        candidates.push(TerminalSlashCandidateFacts {
            id: "compact".to_string(),
            title: "压缩上下文".to_string(),
            description: "向服务端提交显式 compact 控制请求".to_string(),
            keywords: vec!["compact".to_string(), "compress".to_string()],
            badges: vec!["built-in".to_string()],
            action: TerminalSlashAction::RequestCompact,
        });
    }
    candidates
}

fn slash_candidate_matches(candidate: &TerminalSlashCandidateFacts, query: &str) -> bool {
    candidate.id.to_lowercase().contains(query)
        || candidate.title.to_lowercase().contains(query)
        || candidate.description.to_lowercase().contains(query)
        || candidate
            .keywords
            .iter()
            .any(|keyword| keyword.to_lowercase().contains(query))
}

fn validate_cursor_format(cursor: &str) -> Result<(), ConversationRouteError> {
    let Some((storage_seq, subindex)) = cursor.split_once('.') else {
        return Err(ConversationRouteError::invalid_request(
            format!("invalid cursor '{cursor}'"),
            None,
        ));
    };
    if storage_seq.parse::<u64>().is_err() || subindex.parse::<u32>().is_err() {
        return Err(ConversationRouteError::invalid_request(
            format!("invalid cursor '{cursor}'"),
            None,
        ));
    }
    Ok(())
}

fn cursor_is_after_head(
    requested_cursor: &str,
    latest_cursor: Option<&str>,
) -> Result<bool, ConversationRouteError> {
    let Some(latest_cursor) = latest_cursor else {
        return Ok(false);
    };
    Ok(parse_cursor(requested_cursor)? > parse_cursor(latest_cursor)?)
}

fn parse_cursor(cursor: &str) -> Result<(u64, u32), ConversationRouteError> {
    let (storage_seq, subindex) = cursor.split_once('.').ok_or_else(|| {
        ConversationRouteError::invalid_request(format!("invalid cursor '{cursor}'"), None)
    })?;
    let storage_seq = storage_seq.parse::<u64>().map_err(|_| {
        ConversationRouteError::invalid_request(format!("invalid cursor '{cursor}'"), None)
    })?;
    let subindex = subindex.parse::<u32>().map_err(|_| {
        ConversationRouteError::invalid_request(format!("invalid cursor '{cursor}'"), None)
    })?;
    Ok((storage_seq, subindex))
}

fn active_plan_reference(
    session_id: &str,
    working_dir: &FsPath,
) -> Result<Option<crate::terminal_projection::PlanReferenceFacts>, ServerRouteError> {
    let Some(state) = load_session_plan_state(session_id, working_dir)? else {
        return Ok(None);
    };
    Ok(Some(crate::terminal_projection::PlanReferenceFacts {
        slug: state.active_plan_slug.clone(),
        path: session_plan_markdown_path(session_id, working_dir, &state.active_plan_slug)?
            .display()
            .to_string(),
        status: state.status.to_string(),
        title: state.title,
    }))
}

fn load_session_plan_state(
    session_id: &str,
    working_dir: &FsPath,
) -> Result<Option<astrcode_host_session::SessionPlanState>, ServerRouteError> {
    let path = session_plan_state_path(session_id, working_dir)?;
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path).map_err(|error| {
        ServerRouteError::internal(format!("reading '{}' failed: {error}", path.display()))
    })?;
    serde_json::from_str::<astrcode_host_session::SessionPlanState>(&content)
        .map(Some)
        .map_err(|error| {
            ServerRouteError::internal(format!(
                "failed to parse session plan state '{}': {error}",
                path.display()
            ))
        })
}

fn session_plan_state_path(
    session_id: &str,
    working_dir: &FsPath,
) -> Result<PathBuf, ServerRouteError> {
    Ok(session_plan_dir(session_id, working_dir)?.join("state.json"))
}

fn session_plan_markdown_path(
    session_id: &str,
    working_dir: &FsPath,
    slug: &str,
) -> Result<PathBuf, ServerRouteError> {
    Ok(session_plan_dir(session_id, working_dir)?.join(format!("{slug}.md")))
}

fn session_plan_dir(session_id: &str, working_dir: &FsPath) -> Result<PathBuf, ServerRouteError> {
    Ok(astrcode_support::hostpaths::project_dir(working_dir)
        .map_err(|error| {
            ServerRouteError::internal(format!(
                "failed to resolve project directory for '{}': {error}",
                working_dir.display()
            ))
        })?
        .join("sessions")
        .join(session_id)
        .join("plan"))
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
    step_progress: astrcode_protocol::http::conversation::v1::ConversationStepProgressDto,
    delta: ConversationDeltaDto,
) -> ConversationStreamEnvelopeDto {
    ConversationStreamEnvelopeDto {
        session_id: session_id.to_string(),
        cursor: astrcode_protocol::http::conversation::v1::ConversationCursorDto(
            cursor.to_string(),
        ),
        step_progress,
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
    use astrcode_core::{
        AgentEventContext, AgentLifecycleStatus, ChildExecutionIdentity, ChildSessionLineageKind,
        ChildSessionNode, ChildSessionStatusSource, ExecutionTaskStatus, ParentExecutionRef, Phase,
        SessionEventRecord, ToolExecutionResult, ToolOutputStream,
    };
    use astrcode_protocol::http::conversation::v1::ConversationStreamEnvelopeDto;
    use serde_json::{Value, json};
    use tokio::sync::broadcast;

    use super::{AgentEvent, ConversationAuthoritativeFacts, ConversationStreamProjectorState};
    use crate::{
        conversation_read_model::{
            ConversationBlockFacts, ConversationBlockPatchFacts, ConversationDeltaFacts,
            ConversationDeltaFrameFacts, ConversationReplayStream, ConversationStreamReplayFacts,
            ConversationUserBlockFacts,
        },
        terminal_projection::{
            TaskItemFacts, TerminalChildSummaryFacts, TerminalControlFacts,
            TerminalStreamReplayFacts, summarize_conversation_authoritative,
        },
    };

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
                "stepProgress": {},
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
                continuation: None,
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

        let refreshed =
            ConversationAuthoritativeFacts::from_summary(summarize_conversation_authoritative(
                &facts.control,
                &[sample_child_summary()],
                &facts.slash_candidates,
            ));

        let envelopes = state.apply_authoritative_refresh("1.4", refreshed);
        assert_eq!(envelopes.len(), 1);
        assert_eq!(
            serde_json::to_value(&envelopes[0]).expect("child summary envelope should encode"),
            json!({
                "sessionId": "session-root",
                "cursor": "1.4",
                "stepProgress": {},
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

    #[test]
    fn authoritative_refresh_emits_control_delta_for_active_tasks() {
        let facts = sample_stream_facts(Vec::new(), Vec::new());
        let mut state = ConversationStreamProjectorState::new(
            "session-root".to_string(),
            Some("1.4".to_string()),
            &facts,
        );

        let mut refreshed_control = facts.control.clone();
        refreshed_control.active_tasks = Some(vec![
            TaskItemFacts {
                content: "实现 authoritative task panel".to_string(),
                status: ExecutionTaskStatus::InProgress,
                active_form: Some("正在实现 authoritative task panel".to_string()),
            },
            TaskItemFacts {
                content: "补充前端 hydration 测试".to_string(),
                status: ExecutionTaskStatus::Pending,
                active_form: None,
            },
        ]);

        let refreshed =
            ConversationAuthoritativeFacts::from_summary(summarize_conversation_authoritative(
                &refreshed_control,
                &facts.child_summaries,
                &facts.slash_candidates,
            ));

        let envelopes = state.apply_authoritative_refresh("1.4", refreshed);
        assert_eq!(envelopes.len(), 1);
        assert_eq!(
            serde_json::to_value(&envelopes[0]).expect("control envelope should encode"),
            json!({
                "sessionId": "session-root",
                "cursor": "1.4",
                "stepProgress": {},
                "kind": "update_control_state",
                "control": {
                    "phase": "callingTool",
                    "canSubmitPrompt": false,
                    "canRequestCompact": true,
                    "compactPending": false,
                    "compacting": false,
                    "currentModeId": "code",
                    "activeTurnId": "turn-1",
                    "activeTasks": [
                        {
                            "content": "实现 authoritative task panel",
                            "status": "in_progress",
                            "activeForm": "正在实现 authoritative task panel"
                        },
                        {
                            "content": "补充前端 hydration 测试",
                            "status": "pending"
                        }
                    ]
                }
            })
        );
    }

    #[test]
    fn live_events_emit_control_overlay_for_runtime_phase_changes() {
        let facts = sample_stream_facts(Vec::new(), Vec::new());
        let mut state = ConversationStreamProjectorState::new(
            "session-root".to_string(),
            Some("1.9".to_string()),
            &facts,
        );

        let streaming_envelopes = state.project_live_event(&AgentEvent::ModelDelta {
            turn_id: "turn-1".to_string(),
            agent: sample_agent_context(),
            delta: "hello".to_string(),
        });
        let streaming_control = live_control_json(&streaming_envelopes);
        assert_eq!(streaming_control["cursor"], json!("1.9"));
        assert_eq!(streaming_control["control"]["phase"], json!("streaming"));
        assert_eq!(
            streaming_control["control"]["activeTurnId"],
            json!("turn-1")
        );
        assert_eq!(
            streaming_control["control"]["canSubmitPrompt"],
            json!(false)
        );

        let done_envelopes = state.project_live_event(&AgentEvent::TurnDone {
            turn_id: "turn-1".to_string(),
            agent: sample_agent_context(),
        });
        let done_control = live_control_json(&done_envelopes);
        assert_eq!(done_control["cursor"], json!("1.9"));
        assert_eq!(done_control["control"]["phase"], json!("idle"));
        assert!(done_control["control"].get("activeTurnId").is_none());
        assert_eq!(done_control["control"]["canSubmitPrompt"], json!(true));
    }

    fn sample_stream_facts(
        seed_records: Vec<SessionEventRecord>,
        history: Vec<SessionEventRecord>,
    ) -> TerminalStreamReplayFacts {
        let (_, receiver) = broadcast::channel(8);
        let (_, live_receiver) = broadcast::channel(8);

        TerminalStreamReplayFacts {
            replay: ConversationStreamReplayFacts {
                cursor: history.last().map(|record| record.event_id.clone()),
                phase: Phase::CallingTool,
                seed_records: seed_records.clone(),
                replay_frames: history
                    .iter()
                    .map(|record| ConversationDeltaFrameFacts {
                        cursor: record.event_id.clone(),
                        step_progress: Default::default(),
                        delta: match &record.event {
                            AgentEvent::ToolCallDelta {
                                tool_call_id,
                                stream,
                                delta,
                                ..
                            } => ConversationDeltaFacts::Patch {
                                block_id: format!("tool:{tool_call_id}:call"),
                                patch: ConversationBlockPatchFacts::AppendToolStream {
                                    stream: *stream,
                                    chunk: delta.clone(),
                                },
                            },
                            _ => ConversationDeltaFacts::Append {
                                block: Box::new(ConversationBlockFacts::User(
                                    ConversationUserBlockFacts {
                                        id: "noop".to_string(),
                                        turn_id: None,
                                        markdown: String::new(),
                                    },
                                )),
                            },
                        },
                    })
                    .collect(),
                replay_history: history.clone(),
            },
            stream: ConversationReplayStream {
                receiver,
                live_receiver,
            },
            control: TerminalControlFacts {
                phase: Phase::CallingTool,
                active_turn_id: Some("turn-1".to_string()),
                manual_compact_pending: false,
                compacting: false,
                last_compact_meta: None,
                current_mode_id: "code".to_string(),
                active_plan: None,
                active_tasks: None,
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

    fn live_control_json(
        envelopes: &[ConversationStreamEnvelopeDto],
    ) -> serde_json::Map<String, Value> {
        envelopes
            .iter()
            .map(|envelope| {
                serde_json::to_value(envelope).expect("conversation envelope should encode")
            })
            .find_map(|value| {
                if value["kind"] == json!("update_control_state") {
                    value.as_object().cloned()
                } else {
                    None
                }
            })
            .expect("live event should include control overlay")
    }
}
