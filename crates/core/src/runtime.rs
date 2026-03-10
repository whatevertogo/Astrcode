use anyhow::Result;
use chrono::Utc;
use tokio_util::sync::CancellationToken;

use crate::agent_loop::AgentLoop;
use crate::event_log::{generate_session_id, DeleteProjectResult, EventLog, SessionMeta};
use crate::events::StorageEvent;
use crate::projection::{project, AgentState};
use crate::provider_factory::ConfigFileProviderFactory;
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
                    if !matches!(event, StorageEvent::AssistantDelta { .. }) {
                        log.append(&event).ok();
                    }
                    emit(&event);
                },
                cancel,
            )
            .await
    }

    pub fn list_sessions() -> Result<Vec<String>> {
        EventLog::list_sessions()
    }

    pub fn list_sessions_with_meta() -> Result<Vec<SessionMeta>> {
        EventLog::list_sessions_with_meta()
    }

    pub fn delete_session(session_id: &str) -> Result<()> {
        EventLog::delete_session(session_id)
    }

    pub fn delete_project(working_dir: &str) -> Result<DeleteProjectResult> {
        EventLog::delete_sessions_by_working_dir(working_dir)
    }
}

fn build_agent_loop() -> Result<AgentLoop> {
    let tools = ToolRegistry::with_v1_defaults();
    Ok(AgentLoop::new(
        std::sync::Arc::new(ConfigFileProviderFactory),
        tools,
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::fs::File;
    use std::io::{BufRead, BufReader, BufWriter, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    use anyhow::anyhow;
    use async_trait::async_trait;
    use chrono::Utc;
    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::task::JoinHandle;
    use tokio_util::sync::CancellationToken;

    use crate::action::LlmMessage;
    use crate::config::{save_config, Config};
    use crate::llm::{EventSink, LlmEvent, LlmOutput, LlmProvider, LlmRequest};
    use crate::provider_factory::ProviderFactory;
    use crate::test_support::TestEnvGuard;

    use super::*;

    struct MockProvider {
        responses: Mutex<VecDeque<LlmOutput>>,
    }

    struct MockProviderFactory {
        provider: Arc<MockProvider>,
    }

    #[async_trait]
    impl LlmProvider for MockProvider {
        async fn generate(&self, request: LlmRequest, sink: Option<EventSink>) -> anyhow::Result<LlmOutput> {
            let response = self.responses
                .lock()
                .expect("lock")
                .pop_front()
                .ok_or_else(|| anyhow!("no responses"))?;

            if request.cancel.is_cancelled() {
                return Err(anyhow!("cancelled"));
            }

            if let Some(sink) = sink {
                for delta in response.content.chars() {
                    sink(LlmEvent::TextDelta(delta.to_string()));
                }
            }

            Ok(response)
        }
    }

    impl ProviderFactory for MockProviderFactory {
        fn build(&self) -> anyhow::Result<Arc<dyn LlmProvider>> {
            Ok(self.provider.clone())
        }
    }

    fn make_test_runtime_with_mock_provider(
        dir: &std::path::Path,
        responses: Vec<LlmOutput>,
    ) -> AgentRuntime {
        let session_id = generate_session_id();
        let path = dir.join(format!("session-{session_id}.jsonl"));
        let log = EventLog::create_at_path(&session_id, path).expect("create log");

        let provider = Arc::new(MockProvider {
            responses: Mutex::new(VecDeque::from(responses)),
        });
        let factory = Arc::new(MockProviderFactory { provider });
        let tools = ToolRegistry::new();
        let loop_ = AgentLoop::new(factory, tools);

        AgentRuntime {
            session_id,
            log,
            loop_,
        }
    }

    async fn spawn_model_echo_server(
        recorded_models: Arc<Mutex<Vec<String>>>,
    ) -> (String, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let addr = listener.local_addr().expect("listener should have addr");
        listener
            .set_nonblocking(true)
            .expect("listener should support nonblocking");
        let listener = tokio::net::TcpListener::from_std(listener).expect("tokio listener");

        let handle = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.expect("accept should work");
                let recorded_models = recorded_models.clone();

                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut buf = [0u8; 1024];
                    let mut content_length = None;
                    let mut header_len = None;

                    loop {
                        let n = socket.read(&mut buf).await.expect("read should work");
                        if n == 0 {
                            break;
                        }
                        request.extend_from_slice(&buf[..n]);

                        if header_len.is_none() {
                            if let Some(idx) =
                                request.windows(4).position(|window| window == b"\r\n\r\n")
                            {
                                let end = idx + 4;
                                header_len = Some(end);
                                let headers = String::from_utf8_lossy(&request[..end]);
                                content_length = headers.lines().find_map(|line| {
                                    let mut parts = line.splitn(2, ':');
                                    let name = parts.next()?.trim();
                                    let value = parts.next()?.trim();
                                    if name.eq_ignore_ascii_case("content-length") {
                                        value.parse::<usize>().ok()
                                    } else {
                                        None
                                    }
                                });
                            }
                        }

                        if let (Some(end), Some(length)) = (header_len, content_length) {
                            if request.len() >= end + length {
                                break;
                            }
                        }
                    }

                    if let (Some(end), Some(length)) = (header_len, content_length) {
                        let body = &request[end..end + length];
                        let payload: serde_json::Value = serde_json::from_slice(body)
                            .expect("request body should be valid json");
                        let model = payload
                            .get("model")
                            .and_then(|value| value.as_str())
                            .unwrap_or_default()
                            .to_string();
                        recorded_models.lock().expect("lock").push(model);
                    }

                    let response_body = format!(
                        "data: {}\n\ndata: [DONE]\n\n",
                        json!({
                            "choices": [
                                {
                                    "delta": {
                                        "content": "ok"
                                    },
                                    "finish_reason": "stop"
                                }
                            ]
                        })
                    );
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        response_body.len(),
                        response_body
                    );
                    socket
                        .write_all(response.as_bytes())
                        .await
                        .expect("response should be written");
                });
            }
        });

        (format!("http://{}", addr), handle)
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
        let responses = vec![LlmOutput {
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
            .submit("hi".into(), CancellationToken::new(), move |e| {
                collected_clone.lock().expect("lock").push(e.clone());
            })
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

    #[tokio::test]
    async fn submit_does_not_persist_assistant_deltas() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let responses = vec![LlmOutput {
            content: "streamed text".into(),
            tool_calls: vec![],
        }];
        let mut runtime = make_test_runtime_with_mock_provider(tmp.path(), responses);

        runtime
            .log
            .append(&StorageEvent::SessionStart {
                session_id: runtime.session_id.clone(),
                timestamp: Utc::now(),
                working_dir: "/tmp".into(),
            })
            .expect("append session start");

        runtime
            .submit("hi".into(), CancellationToken::new(), |_event| {})
            .await
            .expect("submit");

        let path = tmp
            .path()
            .join(format!("session-{}.jsonl", runtime.session_id));
        let events = load_events_from_path(&path);

        assert!(
            !events
                .iter()
                .any(|event| matches!(event, StorageEvent::AssistantDelta { .. })),
            "delta events are transient and should not be persisted"
        );
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
        assert!(matches!(&state.messages[0], LlmMessage::User { content } if content == "hello"));
        assert!(matches!(&state.messages[1], LlmMessage::Assistant { .. }));
    }

    #[tokio::test]
    async fn submit_uses_updated_model_after_config_changes() {
        let guard = TestEnvGuard::new();
        let temp = tempfile::tempdir().expect("tempdir should be created");
        guard.set_current_dir(temp.path());

        let recorded_models = Arc::new(Mutex::new(Vec::new()));
        let (base_url, server_handle) = spawn_model_echo_server(recorded_models.clone()).await;

        let config = Config {
            active_profile: "default".to_string(),
            active_model: "model-a".to_string(),
            profiles: vec![crate::config::Profile {
                base_url: base_url.clone(),
                api_key: Some("sk-test".to_string()),
                models: vec!["model-a".to_string(), "model-b".to_string()],
                ..crate::config::Profile::default()
            }],
            ..Config::default()
        };
        save_config(&config).expect("config should save");

        let mut runtime = AgentRuntime::new_session().expect("runtime should build");
        runtime
            .submit("first".into(), CancellationToken::new(), |_event| {})
            .await
            .expect("first submit should succeed");

        let updated = Config {
            active_model: "model-b".to_string(),
            ..config
        };
        save_config(&updated).expect("updated config should save");

        runtime
            .submit("second".into(), CancellationToken::new(), |_event| {})
            .await
            .expect("second submit should succeed");

        let models = recorded_models.lock().expect("lock").clone();
        server_handle.abort();

        assert_eq!(models, vec!["model-a".to_string(), "model-b".to_string()]);
    }
}
