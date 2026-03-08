use std::sync::Arc;

use anyhow::{anyhow, Result};
use chrono::Utc;
use tokio_util::sync::CancellationToken;

use crate::agent_loop::AgentLoop;
use crate::config::{load_config, Profile};
use crate::event_log::{generate_session_id, EventLog};
use crate::events::StorageEvent;
use crate::llm::openai::OpenAiProvider;
use crate::llm::LlmProvider;
use crate::projection::{project, AgentState};
use crate::tools::registry::ToolRegistry;

pub struct AgentRuntime {
    pub session_id: String,
    log: EventLog,
    loop_: AgentLoop,
}

impl AgentRuntime {
    /// Create a brand new session, writing a SessionStart event.
    pub fn new_session() -> Result<Self> {
        let session_id = generate_session_id();
        let mut log = EventLog::create(&session_id)?;

        let working_dir = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        log.append(&StorageEvent::SessionStart {
            session_id: session_id.clone(),
            timestamp: Utc::now(),
            working_dir,
        })?;

        let loop_ = build_agent_loop()?;

        Ok(Self {
            session_id,
            log,
            loop_,
        })
    }

    /// Resume an existing session.
    pub fn resume(session_id: &str) -> Result<Self> {
        let log = EventLog::open(session_id)?;
        let loop_ = build_agent_loop()?;
        Ok(Self {
            session_id: session_id.to_string(),
            log,
            loop_,
        })
    }

    /// Resume the last session (by alphabetical order), or None if no sessions exist.
    pub fn resume_last() -> Result<Option<Self>> {
        let sessions = EventLog::list_sessions()?;
        match sessions.last() {
            Some(id) => Ok(Some(Self::resume(id)?)),
            None => Ok(None),
        }
    }

    /// Get the current AgentState by replaying all events.
    pub fn state(&self) -> Result<AgentState> {
        let events = EventLog::load_from_path(self.log.path())?;
        Ok(project(&events))
    }

    /// Submit a user message: append it, rebuild state, run a turn, and emit+log
    /// every event produced by the agent loop.
    ///
    /// `cancel` is managed by the caller (Tauri handle) for interruption.
    /// `emit` is called for every StorageEvent so the caller can bridge to IPC.
    pub async fn submit(
        &mut self,
        text: String,
        cancel: CancellationToken,
        emit: impl Fn(&StorageEvent),
    ) -> Result<()> {
        // 1. Write & emit UserMessage
        let user_event = StorageEvent::UserMessage {
            content: text,
            timestamp: Utc::now(),
        };
        self.log.append(&user_event)?;
        emit(&user_event);

        // 2. Rebuild state from full log
        let events = EventLog::load_from_path(self.log.path())?;
        let state = project(&events);

        // 3. Run the agent loop; each event is logged + emitted
        let log = &mut self.log;
        let loop_ = &self.loop_;

        loop_
            .run_turn(
                &state,
                &mut |event: StorageEvent| {
                    log.append(&event).ok();
                    emit(&event);
                },
                cancel,
            )
            .await
    }

    pub fn list_sessions() -> Result<Vec<String>> {
        EventLog::list_sessions()
    }
}

fn build_agent_loop() -> Result<AgentLoop> {
    let config = load_config()?;
    let profile = select_profile(&config.profiles, &config.active_profile)?;
    let api_key = profile.resolve_api_key()?;
    let model = profile
        .models
        .first()
        .cloned()
        .unwrap_or_else(|| config.active_model.clone());
    let provider: Arc<dyn LlmProvider> = Arc::new(OpenAiProvider::new(
        profile.base_url.clone(),
        api_key,
        model,
    ));
    let tools = ToolRegistry::with_v1_defaults();
    Ok(AgentLoop::new(provider, tools))
}

fn select_profile<'a>(profiles: &'a [Profile], active_profile: &str) -> Result<&'a Profile> {
    profiles
        .iter()
        .find(|profile| profile.name == active_profile)
        .or_else(|| profiles.first())
        .ok_or_else(|| anyhow!("no profiles configured"))
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::fs::File;
    use std::io::{BufRead, BufReader, BufWriter, Write};
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use chrono::Utc;
    use tokio_util::sync::CancellationToken;

    use crate::action::{LlmMessage, LlmResponse, ToolDefinition};
    use crate::llm::LlmProvider;

    use super::*;

    struct MockProvider {
        responses: Mutex<VecDeque<LlmResponse>>,
    }

    #[async_trait]
    impl LlmProvider for MockProvider {
        async fn complete(
            &self,
            _messages: &[LlmMessage],
            _tools: &[ToolDefinition],
            _cancel: CancellationToken,
        ) -> anyhow::Result<LlmResponse> {
            self.responses
                .lock()
                .expect("lock")
                .pop_front()
                .ok_or_else(|| anyhow!("no responses"))
        }
    }

    fn make_test_runtime_with_mock_provider(
        dir: &std::path::Path,
        responses: Vec<LlmResponse>,
    ) -> AgentRuntime {
        let session_id = generate_session_id();
        let path = dir.join(format!("session-{session_id}.jsonl"));
        let log = EventLog::create_at_path(&session_id, path).expect("create log");

        let provider = Arc::new(MockProvider {
            responses: Mutex::new(VecDeque::from(responses)),
        });
        let tools = ToolRegistry::new();
        let loop_ = AgentLoop::new(provider, tools);

        AgentRuntime {
            session_id,
            log,
            loop_,
        }
    }

    fn load_events_from_path(path: &std::path::Path) -> Vec<StorageEvent> {
        let file = File::open(path).expect("open");
        let reader = BufReader::new(file);
        reader
            .lines()
            .filter_map(|line| serde_json::from_str(&line.expect("read line")).ok())
            .collect()
    }

    #[test]
    fn new_session_creates_file_with_session_start() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let session_id = generate_session_id();
        let path = tmp.path().join(format!("session-{session_id}.jsonl"));

        // Create log at the temp path and append SessionStart
        let mut log = EventLog::create_at_path(&session_id, path.clone()).expect("create log");

        log.append(&StorageEvent::SessionStart {
            session_id: session_id.clone(),
            timestamp: Utc::now(),
            working_dir: "/tmp".into(),
        })
        .expect("append");

        assert!(path.exists());
        let events = load_events_from_path(&path);
        assert!(!events.is_empty());
        assert!(matches!(&events[0], StorageEvent::SessionStart { .. }));
    }

    #[tokio::test]
    async fn submit_appends_events_and_load_can_read_them() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let responses = vec![LlmResponse {
            content: "Hello!".into(),
            tool_calls: vec![],
        }];
        let mut runtime = make_test_runtime_with_mock_provider(tmp.path(), responses);

        // First append SessionStart (normally done in new_session)
        runtime
            .log
            .append(&StorageEvent::SessionStart {
                session_id: runtime.session_id.clone(),
                timestamp: Utc::now(),
                working_dir: "/tmp".into(),
            })
            .expect("append session start");

        let collected: Arc<Mutex<Vec<StorageEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let collected_clone = collected.clone();
        runtime
            .submit(
                "hi".into(),
                CancellationToken::new(),
                move |e| {
                    collected_clone.lock().expect("lock").push(e.clone());
                },
            )
            .await
            .expect("submit");

        // Verify emitted events
        let emitted = collected.lock().expect("lock").clone();
        assert!(emitted
            .iter()
            .any(|e| matches!(e, StorageEvent::UserMessage { .. })));
        assert!(emitted
            .iter()
            .any(|e| matches!(e, StorageEvent::AssistantFinal { .. })));
        assert!(emitted
            .iter()
            .any(|e| matches!(e, StorageEvent::TurnDone { .. })));

        // Verify persistence
        let path = tmp
            .path()
            .join(format!("session-{}.jsonl", runtime.session_id));
        let events = load_events_from_path(&path);

        assert!(events
            .iter()
            .any(|e| matches!(e, StorageEvent::SessionStart { .. })));
        assert!(events
            .iter()
            .any(|e| matches!(e, StorageEvent::UserMessage { .. })));
        assert!(events
            .iter()
            .any(|e| matches!(e, StorageEvent::AssistantFinal { .. })));
        assert!(events
            .iter()
            .any(|e| matches!(e, StorageEvent::TurnDone { .. })));
    }

    #[test]
    fn resume_rebuilds_historical_messages() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let session_id = generate_session_id();
        let path = tmp.path().join(format!("session-{session_id}.jsonl"));

        // Write history manually
        {
            let file = File::create(&path).expect("create");
            let mut writer = BufWriter::new(file);
            writeln!(
                writer,
                r#"{{"type":"sessionStart","sessionId":"{}","timestamp":"2026-01-01T00:00:00Z","workingDir":"/tmp"}}"#,
                session_id
            )
            .unwrap();
            writeln!(
                writer,
                r#"{{"type":"userMessage","content":"hello","timestamp":"2026-01-01T00:01:00Z"}}"#
            )
            .unwrap();
            writeln!(
                writer,
                r#"{{"type":"assistantFinal","content":"Hi there!"}}"#
            )
            .unwrap();
            writeln!(
                writer,
                r#"{{"type":"turnDone","timestamp":"2026-01-01T00:02:00Z"}}"#
            )
            .unwrap();
        }

        // Load events and project to state
        let events = load_events_from_path(&path);
        let state = project(&events);

        assert_eq!(state.messages.len(), 2); // User + Assistant
        assert!(
            matches!(&state.messages[0], LlmMessage::User { content } if content == "hello")
        );
        assert!(matches!(&state.messages[1], LlmMessage::Assistant { .. }));
    }
}
