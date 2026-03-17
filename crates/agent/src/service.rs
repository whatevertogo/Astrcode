use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use dashmap::DashMap;
use tokio::sync::{broadcast, Mutex};
use uuid::Uuid;

use astrcode_core::{AgentEvent, AstrError, CancelToken, Phase, ToolCallEventResult};

use crate::agent_loop::AgentLoop;
use crate::config::{
    config_path, load_config, open_config_in_editor, save_config, test_connection,
};
use crate::event_log::{generate_session_id, DeleteProjectResult, EventLog, SessionMeta};
use crate::events::{StorageEvent, StoredEvent};
use crate::projection::project;
use crate::provider_factory::ConfigFileProviderFactory;
use crate::tool_registry::ToolRegistry;

#[derive(Debug, Clone)]
pub struct PromptAccepted {
    pub turn_id: String,
}

#[derive(Clone, Debug)]
pub enum SessionMessage {
    User {
        content: String,
        timestamp: String,
    },
    Assistant {
        content: String,
        timestamp: String,
    },
    ToolCall {
        tool_call_id: String,
        tool_name: String,
        args: serde_json::Value,
        output: Option<String>,
        ok: Option<bool>,
        duration_ms: Option<u64>,
    },
}

#[derive(Clone, Debug)]
pub struct SessionEventRecord {
    pub event_id: String,
    pub event: AgentEvent,
}

pub struct SessionReplay {
    pub history: Vec<SessionEventRecord>,
    pub receiver: broadcast::Receiver<SessionEventRecord>,
}

pub trait SessionReplaySource {
    fn replay(&self, session_id: &str, last_event_id: Option<&str>)
        -> ServiceResult<SessionReplay>;
}

#[derive(Debug)]
pub enum ServiceError {
    NotFound(String),
    Conflict(String),
    InvalidInput(String),
    Internal(AstrError),
}

impl Display for ServiceError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(message) | Self::Conflict(message) | Self::InvalidInput(message) => {
                f.write_str(message)
            }
            Self::Internal(error) => Display::fmt(error, f),
        }
    }
}

impl std::error::Error for ServiceError {}

impl From<AstrError> for ServiceError {
    fn from(value: AstrError) -> Self {
        match &value {
            AstrError::SessionNotFound(id) => Self::NotFound(format!("session not found: {}", id)),
            AstrError::ProjectNotFound(id) => Self::NotFound(format!("project not found: {}", id)),
            AstrError::TurnInProgress(id) => Self::Conflict(format!("turn already in progress: {}", id)),
            AstrError::Validation(msg) => Self::InvalidInput(msg.clone()),
            AstrError::InvalidSessionId(id) => Self::InvalidInput(format!("invalid session id: {}", id)),
            AstrError::MissingApiKey(profile) => Self::InvalidInput(format!("missing api key for profile: {}", profile)),
            AstrError::MissingBaseUrl(profile) => Self::InvalidInput(format!("missing base url for profile: {}", profile)),
            _ => Self::Internal(value),
        }
    }
}

impl From<anyhow::Error> for ServiceError {
    fn from(value: anyhow::Error) -> Self {
        Self::Internal(AstrError::Internal(value.to_string()))
    }
}

pub type ServiceResult<T> = std::result::Result<T, ServiceError>;

struct SessionWriter {
    inner: StdMutex<EventLog>,
}

impl SessionWriter {
    fn new(log: EventLog) -> Self {
        Self {
            inner: StdMutex::new(log),
        }
    }

    fn append(&self, event: &StorageEvent) -> Result<StoredEvent> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| anyhow!("session writer lock poisoned"))?;
        guard.append(event)
    }
}

struct SessionState {
    working_dir: PathBuf,
    phase: StdMutex<Phase>,
    running: AtomicBool,
    cancel: StdMutex<CancelToken>,
    broadcaster: broadcast::Sender<SessionEventRecord>,
    writer: Arc<SessionWriter>,
}

impl SessionState {
    fn new(working_dir: PathBuf, phase: Phase, writer: Arc<SessionWriter>) -> Self {
        let (broadcaster, _) = broadcast::channel(512);
        Self {
            working_dir,
            phase: StdMutex::new(phase),
            running: AtomicBool::new(false),
            cancel: StdMutex::new(CancelToken::new()),
            broadcaster,
            writer,
        }
    }
}

pub struct AgentService {
    sessions: DashMap<String, Arc<SessionState>>,
    loop_: Arc<AgentLoop>,
    config: Mutex<crate::config::Config>,
}

impl AgentService {
    pub fn new(registry: ToolRegistry) -> ServiceResult<Self> {
        let config = load_config().map_err(ServiceError::from)?;
        let loop_ = AgentLoop::new(Arc::new(ConfigFileProviderFactory), registry);
        Ok(Self {
            sessions: DashMap::new(),
            loop_: Arc::new(loop_),
            config: Mutex::new(config),
        })
    }

    pub async fn get_config(&self) -> crate::config::Config {
        self.config.lock().await.clone()
    }

    pub async fn save_active_selection(
        &self,
        active_profile: String,
        active_model: String,
    ) -> ServiceResult<()> {
        let mut config = self.config.lock().await;
        let profile = config
            .profiles
            .iter()
            .find(|profile| profile.name == active_profile)
            .ok_or_else(|| {
                ServiceError::InvalidInput(format!("profile '{}' does not exist", active_profile))
            })?;

        if !profile.models.iter().any(|model| model == &active_model) {
            return Err(ServiceError::InvalidInput(format!(
                "model '{}' does not exist in profile '{}'",
                active_model, active_profile
            )));
        }

        config.active_profile = active_profile;
        config.active_model = active_model;
        save_config(&config).map_err(ServiceError::from)
    }

    pub async fn current_config_path(&self) -> ServiceResult<PathBuf> {
        config_path().map_err(ServiceError::from)
    }

    pub async fn list_sessions_with_meta(&self) -> ServiceResult<Vec<SessionMeta>> {
        EventLog::list_sessions_with_meta().map_err(ServiceError::from)
    }

    pub async fn list_sessions(&self) -> ServiceResult<Vec<String>> {
        EventLog::list_sessions().map_err(ServiceError::from)
    }

    pub async fn create_session(
        &self,
        working_dir: impl Into<PathBuf>,
    ) -> ServiceResult<SessionMeta> {
        let working_dir = normalize_working_dir(working_dir.into())?;
        let session_id = generate_session_id();
        let mut log = EventLog::create(&session_id).map_err(ServiceError::from)?;
        let created_at = Utc::now();
        let session_start = StorageEvent::SessionStart {
            session_id: session_id.clone(),
            timestamp: created_at,
            working_dir: working_dir.to_string_lossy().to_string(),
        };
        let _ = log.append(&session_start).map_err(ServiceError::from)?;

        let state = Arc::new(SessionState::new(
            working_dir.clone(),
            Phase::Idle,
            Arc::new(SessionWriter::new(log)),
        ));
        self.sessions.insert(session_id.clone(), state);

        Ok(SessionMeta {
            session_id,
            working_dir: working_dir.to_string_lossy().to_string(),
            display_name: display_name_from_working_dir(&working_dir),
            title: "新会话".to_string(),
            created_at,
            updated_at: created_at,
            phase: Phase::Idle,
        })
    }

    pub async fn load_session_messages(
        &self,
        session_id: &str,
    ) -> ServiceResult<Vec<SessionMessage>> {
        Ok(self.load_session_snapshot(session_id)?.0)
    }

    pub fn load_session_snapshot(
        &self,
        session_id: &str,
    ) -> ServiceResult<(Vec<SessionMessage>, Option<String>)> {
        let events = EventLog::load(session_id).map_err(ServiceError::from)?;
        let cursor = replay_records(&events, None)
            .last()
            .map(|record| record.event_id.clone());
        Ok((convert_events_to_messages(&events), cursor))
    }

    pub async fn delete_session(&self, session_id: &str) -> ServiceResult<()> {
        let normalized = normalize_session_id(session_id);
        self.interrupt(&normalized).await?;
        self.sessions.remove(&normalized);
        EventLog::delete_session(&normalized).map_err(ServiceError::from)
    }

    pub async fn delete_project(&self, working_dir: &str) -> ServiceResult<DeleteProjectResult> {
        let metas = EventLog::list_sessions_with_meta().map_err(ServiceError::from)?;
        let targets = metas
            .into_iter()
            .filter(|meta| meta.working_dir == working_dir)
            .map(|meta| meta.session_id)
            .collect::<Vec<_>>();

        for session_id in &targets {
            let _ = self.interrupt(session_id).await;
            self.sessions.remove(session_id);
        }

        EventLog::delete_sessions_by_working_dir(working_dir).map_err(ServiceError::from)
    }

    pub async fn submit_prompt(
        &self,
        session_id: &str,
        text: String,
    ) -> ServiceResult<PromptAccepted> {
        let session_id = normalize_session_id(session_id);
        let session = self.ensure_session_loaded(&session_id).await?;
        if session.running.swap(true, Ordering::SeqCst) {
            return Err(ServiceError::Conflict(format!(
                "session '{}' is already running",
                session_id
            )));
        }

        let turn_id = Uuid::new_v4().to_string();
        let cancel = CancelToken::new();
        {
            let mut guard = session
                .cancel
                .lock()
                .map_err(|_| ServiceError::Internal(AstrError::Internal("session cancel lock poisoned".to_string())))?;
            *guard = cancel.clone();
        }

        let state = session.clone();
        let session_id_for_task = session_id.clone();
        let loop_ = self.loop_.clone();
        let text_for_task = text;

        let accepted_turn_id = turn_id.clone();
        tokio::spawn(async move {
            let initial_phase = state
                .phase
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or(Phase::Idle);
            let mut translator = EventTranslator::new(initial_phase);

            let user_event = StorageEvent::UserMessage {
                turn_id: Some(turn_id.clone()),
                content: text_for_task,
                timestamp: Utc::now(),
            };

            let task_result = append_and_broadcast(&state, &user_event, &mut translator)
                .and_then(|_| EventLog::load(&session_id_for_task))
                .map(|events| {
                    events
                        .into_iter()
                        .map(|stored| stored.event)
                        .collect::<Vec<_>>()
                })
                .and_then(|events| {
                    let projected = project(&events);
                    Ok(projected)
                });

            let result = match task_result {
                Ok(projected) => loop_
                    .run_turn(
                        &projected,
                        &turn_id,
                        &mut |event| {
                            let _ = append_and_broadcast(&state, &event, &mut translator);
                        },
                        cancel.clone(),
                    )
                    .await
                    .map_err(|error| anyhow!(error)),
                Err(error) => Err(error),
            };

            if let Err(error) = result {
                let error_event = StorageEvent::Error {
                    turn_id: Some(turn_id.clone()),
                    message: error.to_string(),
                };
                let _ = append_and_broadcast(&state, &error_event, &mut translator);
                let turn_done = StorageEvent::TurnDone {
                    turn_id: Some(turn_id.clone()),
                    timestamp: Utc::now(),
                };
                let _ = append_and_broadcast(&state, &turn_done, &mut translator);
            }

            if let Ok(mut phase) = state.phase.lock() {
                *phase = translator.phase;
            }
            if let Ok(mut guard) = state.cancel.lock() {
                *guard = CancelToken::new();
            }
            state.running.store(false, Ordering::SeqCst);
        });

        Ok(PromptAccepted {
            turn_id: accepted_turn_id,
        })
    }

    pub async fn interrupt(&self, session_id: &str) -> ServiceResult<()> {
        let session_id = normalize_session_id(session_id);
        if let Some(session) = self.sessions.get(&session_id) {
            if let Ok(cancel) = session.cancel.lock() {
                cancel.cancel();
            }
        }
        Ok(())
    }

    pub async fn open_config_in_editor(&self) -> ServiceResult<()> {
        open_config_in_editor().map_err(ServiceError::from)
    }

    pub async fn test_connection(
        &self,
        profile_name: &str,
        model: &str,
    ) -> ServiceResult<crate::config::TestResult> {
        let config = self.config.lock().await.clone();
        let profile = config
            .profiles
            .iter()
            .find(|profile| profile.name == profile_name)
            .ok_or_else(|| {
                ServiceError::InvalidInput(format!("profile '{}' does not exist", profile_name))
            })?;
        test_connection(profile, model)
            .await
            .map_err(ServiceError::from)
    }

    async fn ensure_session_loaded(&self, session_id: &str) -> ServiceResult<Arc<SessionState>> {
        self.load_or_get_session_state(session_id)
    }

    fn load_or_get_session_state(&self, session_id: &str) -> ServiceResult<Arc<SessionState>> {
        if let Some(existing) = self.sessions.get(session_id) {
            return Ok(existing.clone());
        }

        let stored = EventLog::load(session_id).map_err(|error| match error.to_string() {
            message if message.contains("session file not found") => {
                ServiceError::NotFound(message)
            }
            _ => ServiceError::from(error),
        })?;
        let Some(first) = stored.first() else {
            return Err(ServiceError::NotFound(format!(
                "session '{}' is empty",
                session_id
            )));
        };

        let working_dir = match &first.event {
            StorageEvent::SessionStart { working_dir, .. } => PathBuf::from(working_dir),
            _ => {
                return Err(ServiceError::Internal(AstrError::Internal(format!(
                    "session '{}' is missing sessionStart",
                    session_id
                ))))
            }
        };
        let phase = stored
            .last()
            .map(|event| phase_of_storage_event(&event.event))
            .unwrap_or(Phase::Idle);
        let log = EventLog::open(session_id).map_err(ServiceError::from)?;
        let state = Arc::new(SessionState::new(
            working_dir,
            phase,
            Arc::new(SessionWriter::new(log)),
        ));
        self.sessions.insert(session_id.to_string(), state.clone());
        Ok(state)
    }
}

impl SessionReplaySource for AgentService {
    fn replay(
        &self,
        session_id: &str,
        last_event_id: Option<&str>,
    ) -> ServiceResult<SessionReplay> {
        let session_id = normalize_session_id(session_id);
        let state = self.load_or_get_session_state(&session_id)?;

        let receiver = state.broadcaster.subscribe();
        let history = EventLog::load(&session_id)
            .map_err(ServiceError::from)
            .map(|events| replay_records(&events, last_event_id))?;
        Ok(SessionReplay { history, receiver })
    }
}

fn normalize_session_id(session_id: &str) -> String {
    session_id
        .strip_prefix("session-")
        .unwrap_or(session_id)
        .trim()
        .to_string()
}

fn normalize_working_dir(working_dir: PathBuf) -> ServiceResult<PathBuf> {
    let path = if working_dir.is_absolute() {
        working_dir
    } else {
        std::env::current_dir()
            .map_err(|error| ServiceError::Internal(AstrError::io("failed to get current directory", error)))?
            .join(working_dir)
    };

    let metadata = std::fs::metadata(&path).map_err(|error| {
        ServiceError::InvalidInput(format!(
            "workingDir '{}' is invalid: {}",
            path.display(),
            error
        ))
    })?;
    if !metadata.is_dir() {
        return Err(ServiceError::InvalidInput(format!(
            "workingDir '{}' is not a directory",
            path.display()
        )));
    }

    std::fs::canonicalize(&path)
        .with_context(|| format!("failed to canonicalize workingDir '{}'", path.display()))
        .map_err(ServiceError::from)
}

fn display_name_from_working_dir(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("默认项目")
        .to_string()
}

fn convert_events_to_messages(events: &[StoredEvent]) -> Vec<SessionMessage> {
    let mut messages = Vec::new();
    let mut pending_tool_calls: Vec<(String, String, serde_json::Value)> = Vec::new();

    for stored in events {
        match &stored.event {
            StorageEvent::UserMessage {
                content, timestamp, ..
            } => messages.push(SessionMessage::User {
                content: content.clone(),
                timestamp: timestamp.to_rfc3339(),
            }),
            StorageEvent::AssistantFinal { content, .. } if !content.is_empty() => {
                messages.push(SessionMessage::Assistant {
                    content: content.clone(),
                    timestamp: Utc::now().to_rfc3339(),
                });
            }
            StorageEvent::ToolCall {
                tool_call_id,
                tool_name,
                args,
                ..
            } => pending_tool_calls.push((tool_call_id.clone(), tool_name.clone(), args.clone())),
            StorageEvent::ToolResult {
                tool_call_id,
                output,
                success,
                duration_ms,
                ..
            } => {
                if let Some(index) = pending_tool_calls
                    .iter()
                    .position(|(pending_id, _, _)| pending_id == tool_call_id)
                {
                    let (_, tool_name, args) = pending_tool_calls.remove(index);
                    messages.push(SessionMessage::ToolCall {
                        tool_call_id: tool_call_id.clone(),
                        tool_name,
                        args,
                        output: Some(output.clone()),
                        ok: Some(*success),
                        duration_ms: Some(*duration_ms),
                    });
                }
            }
            _ => {}
        }
    }

    for (tool_call_id, tool_name, args) in pending_tool_calls {
        messages.push(SessionMessage::ToolCall {
            tool_call_id,
            tool_name,
            args,
            output: None,
            ok: None,
            duration_ms: None,
        });
    }

    messages
}

fn append_and_broadcast(
    session: &SessionState,
    event: &StorageEvent,
    translator: &mut EventTranslator,
) -> Result<()> {
    let stored = session.writer.append(event)?;
    let records = translator.translate(&stored);
    for record in records {
        let _ = session.broadcaster.send(record);
    }
    Ok(())
}

fn replay_records(events: &[StoredEvent], last_event_id: Option<&str>) -> Vec<SessionEventRecord> {
    let mut translator = EventTranslator::new(Phase::Idle);
    let after_id = last_event_id.and_then(parse_event_id);
    let mut history = Vec::new();

    for stored in events {
        for record in translator.translate(stored) {
            if let Some(after_id) = after_id {
                let Some(current_id) = parse_event_id(&record.event_id) else {
                    continue;
                };
                if current_id <= after_id {
                    continue;
                }
            }
            history.push(record);
        }
    }

    history
}

fn parse_event_id(raw: &str) -> Option<(u64, u32)> {
    let (storage_seq, subindex) = raw.split_once('.')?;
    let storage_seq = storage_seq.parse().ok()?;
    let subindex = subindex.parse().ok()?;
    Some((storage_seq, subindex))
}

fn phase_of_storage_event(event: &StorageEvent) -> Phase {
    match event {
        StorageEvent::SessionStart { .. } => Phase::Idle,
        StorageEvent::UserMessage { .. } => Phase::Thinking,
        StorageEvent::AssistantDelta { .. } | StorageEvent::AssistantFinal { .. } => {
            Phase::Streaming
        }
        StorageEvent::ToolCall { .. } | StorageEvent::ToolResult { .. } => Phase::CallingTool,
        StorageEvent::TurnDone { .. } | StorageEvent::Error { .. } => Phase::Idle,
    }
}

struct EventTranslator {
    phase: Phase,
    current_turn_id: Option<String>,
    legacy_turn_index: u64,
}

impl EventTranslator {
    fn new(phase: Phase) -> Self {
        Self {
            phase,
            current_turn_id: None,
            legacy_turn_index: 0,
        }
    }

    fn translate(&mut self, stored: &StoredEvent) -> Vec<SessionEventRecord> {
        let mut subindex = 0u32;
        let mut records = Vec::new();
        let turn_id = self.turn_id_for(&stored.event);

        let mut push = |event: AgentEvent, records: &mut Vec<SessionEventRecord>| {
            records.push(SessionEventRecord {
                event_id: format!("{}.{}", stored.storage_seq, subindex),
                event,
            });
            subindex = subindex.saturating_add(1);
        };

        match &stored.event {
            StorageEvent::SessionStart { session_id, .. } => {
                push(
                    AgentEvent::SessionStarted {
                        session_id: session_id.clone(),
                    },
                    &mut records,
                );
                self.phase = Phase::Idle;
            }
            StorageEvent::UserMessage { .. } => {
                if self.phase != Phase::Thinking {
                    push(
                        AgentEvent::PhaseChanged {
                            turn_id: turn_id.clone(),
                            phase: Phase::Thinking,
                        },
                        &mut records,
                    );
                }
                self.phase = Phase::Thinking;
            }
            StorageEvent::AssistantDelta { token, .. } => {
                if self.phase != Phase::Streaming {
                    push(
                        AgentEvent::PhaseChanged {
                            turn_id: turn_id.clone(),
                            phase: Phase::Streaming,
                        },
                        &mut records,
                    );
                }
                if let Some(turn_id) = turn_id.clone() {
                    push(
                        AgentEvent::ModelDelta {
                            turn_id,
                            delta: token.clone(),
                        },
                        &mut records,
                    );
                }
                self.phase = Phase::Streaming;
            }
            StorageEvent::AssistantFinal { content, .. } => {
                if self.phase != Phase::Streaming {
                    push(
                        AgentEvent::PhaseChanged {
                            turn_id: turn_id.clone(),
                            phase: Phase::Streaming,
                        },
                        &mut records,
                    );
                }
                if !content.is_empty() {
                    if let Some(turn_id) = turn_id.clone() {
                        push(
                            AgentEvent::AssistantMessage {
                                turn_id,
                                content: content.clone(),
                            },
                            &mut records,
                        );
                    }
                }
                self.phase = Phase::Streaming;
            }
            StorageEvent::ToolCall {
                tool_call_id,
                tool_name,
                args,
                ..
            } => {
                if self.phase != Phase::CallingTool {
                    push(
                        AgentEvent::PhaseChanged {
                            turn_id: turn_id.clone(),
                            phase: Phase::CallingTool,
                        },
                        &mut records,
                    );
                }
                if let Some(turn_id) = turn_id.clone() {
                    push(
                        AgentEvent::ToolCallStart {
                            turn_id,
                            tool_call_id: tool_call_id.clone(),
                            tool_name: tool_name.clone(),
                            input: args.clone(),
                        },
                        &mut records,
                    );
                }
                self.phase = Phase::CallingTool;
            }
            StorageEvent::ToolResult {
                tool_call_id,
                output,
                success,
                duration_ms,
                ..
            } => {
                if let Some(turn_id) = turn_id.clone() {
                    push(
                        AgentEvent::ToolCallResult {
                            turn_id,
                            result: ToolCallEventResult {
                                tool_call_id: tool_call_id.clone(),
                                tool_name: String::new(),
                                ok: *success,
                                output: output.clone(),
                                error: None,
                                metadata: None,
                                duration_ms: *duration_ms as u128,
                            },
                        },
                        &mut records,
                    );
                }
                self.phase = Phase::CallingTool;
            }
            StorageEvent::TurnDone { .. } => {
                push(
                    AgentEvent::PhaseChanged {
                        turn_id: turn_id.clone(),
                        phase: Phase::Idle,
                    },
                    &mut records,
                );
                if let Some(turn_id) = turn_id.clone() {
                    push(AgentEvent::TurnDone { turn_id }, &mut records);
                }
                self.phase = Phase::Idle;
                self.current_turn_id = None;
            }
            StorageEvent::Error { message, .. } => {
                if message == "interrupted" {
                    push(
                        AgentEvent::PhaseChanged {
                            turn_id: turn_id.clone(),
                            phase: Phase::Interrupted,
                        },
                        &mut records,
                    );
                    self.phase = Phase::Interrupted;
                }
                push(
                    AgentEvent::Error {
                        turn_id,
                        code: if message == "interrupted" {
                            "interrupted".to_string()
                        } else {
                            "agent_error".to_string()
                        },
                        message: message.clone(),
                    },
                    &mut records,
                );
            }
        }

        records
    }

    fn turn_id_for(&mut self, event: &StorageEvent) -> Option<String> {
        if let Some(turn_id) = event.turn_id() {
            let turn_id = turn_id.to_string();
            self.current_turn_id = Some(turn_id.clone());
            return Some(turn_id);
        }

        match event {
            StorageEvent::UserMessage { .. } => {
                self.legacy_turn_index = self.legacy_turn_index.saturating_add(1);
                let turn_id = format!("legacy-turn-{}", self.legacy_turn_index);
                self.current_turn_id = Some(turn_id.clone());
                Some(turn_id)
            }
            StorageEvent::SessionStart { .. } => None,
            _ => self.current_turn_id.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn empty_assistant_final_only_updates_phase() {
        let mut translator = EventTranslator::new(Phase::Thinking);
        let stored = StoredEvent {
            storage_seq: 7,
            event: StorageEvent::AssistantFinal {
                turn_id: Some("turn-1".to_string()),
                content: String::new(),
            },
        };

        let records = translator.translate(&stored);

        assert_eq!(records.len(), 1);
        assert!(matches!(
            records[0].event,
            AgentEvent::PhaseChanged {
                turn_id: Some(ref turn_id),
                phase: Phase::Streaming,
            } if turn_id == "turn-1"
        ));
    }

    #[test]
    fn non_empty_assistant_final_emits_message() {
        let mut translator = EventTranslator::new(Phase::Thinking);
        let stored = StoredEvent {
            storage_seq: 8,
            event: StorageEvent::AssistantFinal {
                turn_id: Some("turn-2".to_string()),
                content: "hello".to_string(),
            },
        };

        let records = translator.translate(&stored);

        assert_eq!(records.len(), 2);
        assert!(matches!(
            records[1].event,
            AgentEvent::AssistantMessage {
                ref turn_id,
                ref content,
            } if turn_id == "turn-2" && content == "hello"
        ));
    }

    #[test]
    fn replay_skips_empty_assistant_messages() {
        let events = vec![
            StoredEvent {
                storage_seq: 1,
                event: StorageEvent::SessionStart {
                    session_id: "session-1".to_string(),
                    timestamp: Utc::now(),
                    working_dir: "/tmp".to_string(),
                },
            },
            StoredEvent {
                storage_seq: 2,
                event: StorageEvent::UserMessage {
                    turn_id: Some("turn-3".to_string()),
                    content: "run tool".to_string(),
                    timestamp: Utc::now(),
                },
            },
            StoredEvent {
                storage_seq: 3,
                event: StorageEvent::AssistantFinal {
                    turn_id: Some("turn-3".to_string()),
                    content: String::new(),
                },
            },
        ];

        let records = replay_records(&events, None);

        assert!(!records
            .iter()
            .any(|record| { matches!(record.event, AgentEvent::AssistantMessage { .. }) }));
    }
}
