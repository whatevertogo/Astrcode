use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::ipc::Channel;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use astrcode_core::{
    load_config, open_config_in_editor as open_config_file_in_editor, save_config, test_connection,
    AgentRuntime, DeleteProjectResult, EventLog, SessionMeta, StorageEvent, TestResult,
};
use ipc::{AgentEvent, AgentEventKind, Phase, ToolCallResultEnvelope};

fn canonical_session_id(session_id: &str) -> &str {
    session_id.strip_prefix("session-").unwrap_or(session_id)
}

fn normalize_working_dir(working_dir: &str) -> String {
    let trimmed = working_dir.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        working_dir.to_string()
    } else {
        trimmed.to_string()
    }
}

fn same_working_dir(a: &str, b: &str) -> bool {
    let left = normalize_working_dir(a);
    let right = normalize_working_dir(b);
    #[cfg(windows)]
    {
        left.eq_ignore_ascii_case(&right)
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn user_home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(std::path::PathBuf::from))
        .or_else(dirs::home_dir)
}

/// Message type for frontend display (converted from StorageEvent)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
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
        args: Value,
        output: Option<String>,
        success: Option<bool>,
        duration_ms: Option<u64>,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileView {
    pub name: String,
    pub base_url: String,
    pub api_key_preview: String,
    pub models: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigView {
    pub config_path: String,
    pub active_profile: String,
    pub active_model: String,
    pub profiles: Vec<ProfileView>,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CurrentModelInfo {
    pub profile_name: String,
    pub model: String,
    pub provider_kind: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelOption {
    pub profile_name: String,
    pub model: String,
    pub provider_kind: String,
}

/// Convert StorageEvent slice to SessionMessage list.
/// ToolCall + ToolResult are merged by tool_call_id.
pub fn convert_events_to_messages(events: &[StorageEvent]) -> Vec<SessionMessage> {
    let mut messages = Vec::new();
    let mut pending_tool_calls: HashMap<String, (String, Value)> = HashMap::new();

    for event in events {
        match event {
            StorageEvent::UserMessage { content, timestamp } => {
                messages.push(SessionMessage::User {
                    content: content.clone(),
                    timestamp: timestamp.to_rfc3339(),
                });
            }
            StorageEvent::AssistantFinal { content } => {
                if !content.is_empty() {
                    messages.push(SessionMessage::Assistant {
                        content: content.clone(),
                        timestamp: chrono::Utc::now().to_rfc3339(),
                    });
                }
            }
            StorageEvent::ToolCall {
                tool_call_id,
                tool_name,
                args,
            } => {
                pending_tool_calls.insert(tool_call_id.clone(), (tool_name.clone(), args.clone()));
            }
            StorageEvent::ToolResult {
                tool_call_id,
                output,
                success,
                duration_ms,
            } => {
                if let Some((tool_name, args)) = pending_tool_calls.remove(tool_call_id) {
                    messages.push(SessionMessage::ToolCall {
                        tool_call_id: tool_call_id.clone(),
                        tool_name,
                        args,
                        output: Some(output.clone()),
                        success: Some(*success),
                        duration_ms: Some(*duration_ms),
                    });
                }
            }
            _ => {}
        }
    }

    messages
}

pub struct AgentHandle {
    runtime: Mutex<AgentRuntime>,
    cancel: Mutex<Option<CancellationToken>>,
    session_id: Mutex<String>,
}

impl AgentHandle {
    pub fn new() -> anyhow::Result<Self> {
        // Try to resume the last session, fallback to new session
        let runtime = match AgentRuntime::resume_last()? {
            Some(r) => {
                // Sync working directory
                if let Ok(state) = r.state() {
                    let _ = std::env::set_current_dir(&state.working_dir);
                }
                r
            }
            None => AgentRuntime::new_session()?,
        };

        let session_id = runtime.session_id.clone();
        Ok(Self {
            runtime: Mutex::new(runtime),
            cancel: Mutex::new(None),
            session_id: Mutex::new(session_id),
        })
    }

    /// Get the current session ID.
    pub async fn get_session_id(&self) -> String {
        canonical_session_id(&self.session_id.lock().await).to_string()
    }

    /// List all session IDs.
    pub fn list_sessions() -> Result<Vec<String>, String> {
        AgentRuntime::list_sessions().map_err(|e| e.to_string())
    }

    /// List all sessions with metadata.
    pub fn list_sessions_with_meta() -> Result<Vec<SessionMeta>, String> {
        AgentRuntime::list_sessions_with_meta().map_err(|e| e.to_string())
    }

    /// Load messages from a session.
    pub fn load_session(session_id: &str) -> Result<Vec<SessionMessage>, String> {
        let session_id = canonical_session_id(session_id);
        let events = EventLog::load(session_id).map_err(|e| e.to_string())?;
        Ok(convert_events_to_messages(&events))
    }

    /// Create a new session, interrupting any current operation.
    pub async fn new_session(&self) -> Result<String, String> {
        // Interrupt current operation
        self.interrupt().await?;

        // Create new runtime
        let runtime = AgentRuntime::new_session().map_err(|e| e.to_string())?;
        let session_id = runtime.session_id.clone();

        // Update handle state
        *self.runtime.lock().await = runtime;
        *self.session_id.lock().await = session_id.clone();

        Ok(session_id)
    }

    /// Switch to an existing session, interrupting any current operation.
    pub async fn switch_session(&self, session_id: &str) -> Result<(), String> {
        let session_id = canonical_session_id(session_id);

        // Interrupt current operation
        self.interrupt().await?;

        // Load target session
        let runtime = AgentRuntime::resume(session_id).map_err(|e| e.to_string())?;

        // Sync working directory
        if let Ok(state) = runtime.state() {
            let _ = std::env::set_current_dir(&state.working_dir);
        }

        // Update handle state
        *self.runtime.lock().await = runtime;
        *self.session_id.lock().await = session_id.to_string();

        Ok(())
    }

    pub async fn delete_session(&self, session_id: String) -> Result<(), String> {
        let target_id = canonical_session_id(&session_id).to_string();
        let current_id = canonical_session_id(&self.session_id.lock().await).to_string();

        if current_id == target_id {
            self.interrupt().await?;
            let runtime = AgentRuntime::new_session().map_err(|e| e.to_string())?;
            let next_session_id = runtime.session_id.clone();

            if let Ok(state) = runtime.state() {
                let _ = std::env::set_current_dir(&state.working_dir);
            }

            *self.runtime.lock().await = runtime;
            *self.session_id.lock().await = next_session_id;
        }

        AgentRuntime::delete_session(&target_id).map_err(|e| e.to_string())
    }

    pub async fn delete_project(&self, working_dir: String) -> Result<DeleteProjectResult, String> {
        let metas = AgentRuntime::list_sessions_with_meta().map_err(|e| e.to_string())?;
        let targets: HashSet<String> = metas
            .iter()
            .filter(|meta| same_working_dir(&meta.working_dir, &working_dir))
            .map(|meta| meta.session_id.clone())
            .collect();

        if targets.is_empty() {
            return Ok(DeleteProjectResult {
                success_count: 0,
                failed_session_ids: Vec::new(),
            });
        }

        let current_id = canonical_session_id(&self.session_id.lock().await).to_string();
        if targets.contains(&current_id) {
            self.interrupt().await?;

            if let Some(replacement) = metas
                .iter()
                .find(|meta| !targets.contains(&meta.session_id))
            {
                let runtime =
                    AgentRuntime::resume(&replacement.session_id).map_err(|e| e.to_string())?;
                if let Ok(state) = runtime.state() {
                    let _ = std::env::set_current_dir(&state.working_dir);
                }
                *self.runtime.lock().await = runtime;
                *self.session_id.lock().await = replacement.session_id.clone();
            } else {
                let home = user_home_dir()
                    .ok_or_else(|| "unable to resolve home directory".to_string())?;
                std::env::set_current_dir(&home).map_err(|e| e.to_string())?;
                let runtime = AgentRuntime::new_session().map_err(|e| e.to_string())?;
                let session_id = runtime.session_id.clone();
                *self.runtime.lock().await = runtime;
                *self.session_id.lock().await = session_id;
            }
        }

        AgentRuntime::delete_project(&working_dir).map_err(|e| e.to_string())
    }

    pub async fn get_config() -> Result<ConfigView, String> {
        let config = load_config().map_err(|e| e.to_string())?;
        let config_path = astrcode_core::config::config_path()
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .to_string();

        if config.profiles.is_empty() {
            return Ok(ConfigView {
                config_path,
                active_profile: String::new(),
                active_model: String::new(),
                profiles: Vec::new(),
                warning: Some("no profiles configured".to_string()),
            });
        }

        let profiles = config
            .profiles
            .iter()
            .map(|profile| ProfileView {
                name: profile.name.clone(),
                base_url: profile.base_url.clone(),
                api_key_preview: api_key_preview(profile.api_key.as_deref()),
                models: profile.models.clone(),
            })
            .collect::<Vec<_>>();

        let (active_profile, active_model, warning) = resolve_active_selection(
            &config.active_profile,
            &config.active_model,
            &config.profiles,
        )?;

        Ok(ConfigView {
            config_path,
            active_profile,
            active_model,
            profiles,
            warning,
        })
    }

    pub async fn save_active_selection(
        &self,
        active_profile: String,
        active_model: String,
    ) -> Result<(), String> {
        self.set_model(active_profile, active_model).await
    }

    pub async fn set_model(&self, profile_name: String, model: String) -> Result<(), String> {
        let mut config = load_config().map_err(|e| e.to_string())?;
        let profile = config
            .profiles
            .iter()
            .find(|profile| profile.name == profile_name)
            .ok_or_else(|| format!("profile '{}' does not exist", profile_name))?;

        if profile.models.is_empty() {
            return Err(format!("profile '{}' has no models", profile_name));
        }

        if !profile
            .models
            .iter()
            .any(|profile_model| profile_model == &model)
        {
            return Err(format!(
                "model '{}' does not exist in profile '{}'",
                model, profile_name
            ));
        }

        config.active_profile = profile_name;
        config.active_model = model;
        save_config(&config).map_err(|e| e.to_string())
    }

    pub async fn get_current_model(&self) -> Result<CurrentModelInfo, String> {
        let config = load_config().map_err(|e| e.to_string())?;

        let profile = config
            .profiles
            .iter()
            .find(|profile| profile.name == config.active_profile)
            .or_else(|| config.profiles.first())
            .ok_or_else(|| "no profiles configured".to_string())?;

        let profile_name = profile.name.clone();
        let provider_kind = profile.provider_kind.clone();

        let model = if profile
            .models
            .iter()
            .any(|item| item == &config.active_model)
        {
            config.active_model.clone()
        } else {
            profile
                .models
                .first()
                .cloned()
                .ok_or_else(|| format!("profile '{}' has no models", profile.name))?
        };

        Ok(CurrentModelInfo {
            profile_name,
            model,
            provider_kind,
        })
    }

    pub async fn list_available_models(&self) -> Result<Vec<ModelOption>, String> {
        let config = load_config().map_err(|e| e.to_string())?;
        Ok(config
            .profiles
            .iter()
            .flat_map(|profile| {
                profile.models.iter().map(|model| ModelOption {
                    profile_name: profile.name.clone(),
                    model: model.clone(),
                    provider_kind: profile.provider_kind.clone(),
                })
            })
            .collect())
    }

    pub async fn test_connection_for_selection(
        &self,
        profile_name: String,
        model: String,
    ) -> Result<TestResult, String> {
        let config = load_config().map_err(|e| e.to_string())?;
        let profile = config
            .profiles
            .iter()
            .find(|profile| profile.name == profile_name)
            .ok_or_else(|| format!("profile '{}' does not exist", profile_name))?;

        test_connection(profile, &model)
            .await
            .map_err(|e| e.to_string())
    }

    pub fn open_config_in_editor() -> Result<(), String> {
        open_config_file_in_editor().map_err(|e| e.to_string())
    }

    pub async fn submit_prompt(
        &self,
        text: String,
        channel: Channel<AgentEvent>,
    ) -> Result<(), String> {
        // Cancel any previous in-flight turn.
        {
            let mut guard = self.cancel.lock().await;
            if let Some(prev) = guard.take() {
                prev.cancel();
            }
        }

        let turn_id = uuid::Uuid::new_v4().to_string();
        let cancel_token = CancellationToken::new();

        {
            let mut guard = self.cancel.lock().await;
            *guard = Some(cancel_token.clone());
        }

        // Emit PhaseChanged(Thinking) before starting the turn.
        send_agent_event(
            &channel,
            AgentEventKind::PhaseChanged {
                turn_id: Some(turn_id.clone()),
                phase: Phase::Thinking,
            },
        );

        let mut runtime = self.runtime.lock().await;
        let cancel = cancel_token;
        let tid = turn_id.clone();
        let streaming_phase_emitted = AtomicBool::new(false);

        let result = runtime
            .submit(text, cancel, |event| {
                // Emit PhaseChanged(Streaming) exactly once per streaming sequence.
                if matches!(event, StorageEvent::AssistantDelta { .. }) {
                    if !streaming_phase_emitted.swap(true, Ordering::Relaxed) {
                        send_agent_event(
                            &channel,
                            AgentEventKind::PhaseChanged {
                                turn_id: Some(tid.clone()),
                                phase: Phase::Streaming,
                            },
                        );
                    }
                } else {
                    streaming_phase_emitted.store(false, Ordering::Relaxed);
                }

                for kind in collect_event_kinds(&tid, event) {
                    send_agent_event(&channel, kind);
                }
            })
            .await;

        if let Err(e) = result {
            eprintln!("agent turn error: {e}");
            return Err(e.to_string());
        }

        Ok(())
    }

    pub async fn interrupt(&self) -> Result<(), String> {
        let mut guard = self.cancel.lock().await;
        if let Some(token) = guard.take() {
            token.cancel();
        }
        Ok(())
    }
}

fn is_env_var_name(value: &str) -> bool {
    value
        .chars()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && value.contains('_')
}

fn api_key_preview(api_key: Option<&str>) -> String {
    match api_key.map(str::trim) {
        None => "未配置".to_string(),
        Some("") => "未配置".to_string(),
        Some(value) if is_env_var_name(value) => format!("环境变量: {}", value),
        Some(value) if value.chars().count() > 4 => {
            let suffix = value
                .chars()
                .rev()
                .take(4)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<String>();
            format!("****{}", suffix)
        }
        Some(_) => "****".to_string(),
    }
}

fn resolve_active_selection(
    active_profile: &str,
    active_model: &str,
    profiles: &[astrcode_core::config::Profile],
) -> Result<(String, String, Option<String>), String> {
    let fallback_profile = profiles
        .first()
        .ok_or_else(|| "no profiles configured".to_string())?;

    let selected_profile = profiles
        .iter()
        .find(|profile| profile.name == active_profile)
        .unwrap_or(fallback_profile);

    if selected_profile.models.is_empty() {
        return Err(format!("profile '{}' has no models", selected_profile.name));
    }

    if selected_profile.name != active_profile {
        return Ok((
            selected_profile.name.clone(),
            selected_profile.models[0].clone(),
            Some(format!(
                "配置中的 Profile 不存在，已自动选择 {}",
                selected_profile.name
            )),
        ));
    }

    if let Some(model) = selected_profile
        .models
        .iter()
        .find(|model| *model == active_model)
    {
        return Ok((selected_profile.name.clone(), model.clone(), None));
    }

    let fallback_model = selected_profile
        .models
        .first()
        .cloned()
        .ok_or_else(|| format!("profile '{}' has no models", selected_profile.name))?;

    Ok((
        selected_profile.name.clone(),
        fallback_model.clone(),
        Some(format!(
            "配置中的 {} 在当前 Profile 下不存在，已自动选择 {}",
            active_model, fallback_model
        )),
    ))
}

/// Convert a StorageEvent into zero or more AgentEventKinds for IPC dispatch.
fn collect_event_kinds(turn_id: &str, event: &StorageEvent) -> Vec<AgentEventKind> {
    match event {
        StorageEvent::UserMessage { .. } => {
            // No direct AgentEvent for the user message itself.
            Vec::new()
        }

        StorageEvent::AssistantDelta { token } => {
            vec![AgentEventKind::ModelDelta {
                turn_id: turn_id.to_string(),
                delta: token.clone(),
            }]
        }

        StorageEvent::AssistantFinal { .. } => Vec::new(),

        StorageEvent::ToolCall {
            tool_call_id,
            tool_name,
            args,
        } => {
            vec![
                AgentEventKind::PhaseChanged {
                    turn_id: Some(turn_id.to_string()),
                    phase: Phase::CallingTool,
                },
                AgentEventKind::ToolCallStart {
                    turn_id: turn_id.to_string(),
                    tool_call_id: tool_call_id.clone(),
                    tool_name: tool_name.clone(),
                    args: args.clone(),
                },
            ]
        }

        StorageEvent::ToolResult {
            tool_call_id,
            output,
            success,
            duration_ms,
        } => {
            vec![AgentEventKind::ToolCallResult {
                turn_id: turn_id.to_string(),
                result: ToolCallResultEnvelope {
                    tool_call_id: tool_call_id.clone(),
                    tool_name: String::new(),
                    ok: *success,
                    output: output.clone(),
                    error: if *success { None } else { Some(output.clone()) },
                    metadata: None,
                    duration_ms: *duration_ms as u128,
                },
            }]
        }

        StorageEvent::TurnDone { .. } => {
            vec![
                AgentEventKind::PhaseChanged {
                    turn_id: Some(turn_id.to_string()),
                    phase: Phase::Done,
                },
                AgentEventKind::TurnDone {
                    turn_id: turn_id.to_string(),
                },
                AgentEventKind::PhaseChanged {
                    turn_id: None,
                    phase: Phase::Idle,
                },
            ]
        }

        StorageEvent::Error { message } => {
            if message == "interrupted" {
                vec![AgentEventKind::PhaseChanged {
                    turn_id: Some(turn_id.to_string()),
                    phase: Phase::Interrupted,
                }]
            } else {
                vec![AgentEventKind::Error {
                    turn_id: Some(turn_id.to_string()),
                    code: "agent_error".to_string(),
                    message: message.clone(),
                }]
            }
        }

        StorageEvent::SessionStart { session_id, .. } => {
            vec![AgentEventKind::SessionStarted {
                session_id: session_id.clone(),
            }]
        }
    }
}

fn send_agent_event(channel: &Channel<AgentEvent>, kind: AgentEventKind) {
    let event = AgentEvent::new(kind);
    if let Err(e) = channel.send(event) {
        eprintln!("failed to send agent-event over channel: {e}");
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::{Mutex as StdMutex, MutexGuard, OnceLock};

    use astrcode_core::config::Config;
    use astrcode_core::config::Profile;
    use uuid::Uuid;

    use super::*;

    fn config_env_lock() -> &'static StdMutex<()> {
        static LOCK: OnceLock<StdMutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| StdMutex::new(()))
    }

    struct AppHomeGuard {
        _lock: MutexGuard<'static, ()>,
        previous: Option<OsString>,
        home: PathBuf,
    }

    impl AppHomeGuard {
        fn new() -> Self {
            let lock = config_env_lock().lock().expect("lock should work");
            let previous = std::env::var_os("ASTRCODE_HOME_DIR");
            let home = std::env::temp_dir().join(format!("astrcode-handle-{}", Uuid::new_v4()));
            fs::create_dir_all(&home).expect("temp home should exist");
            std::env::set_var("ASTRCODE_HOME_DIR", &home);

            Self {
                _lock: lock,
                previous,
                home,
            }
        }
    }

    impl Drop for AppHomeGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var("ASTRCODE_HOME_DIR", value),
                None => std::env::remove_var("ASTRCODE_HOME_DIR"),
            }
            let _ = fs::remove_dir_all(&self.home);
        }
    }

    #[test]
    fn canonical_session_id_strips_prefix_once() {
        assert_eq!(
            canonical_session_id("session-2026-03-08T10-00-00-aaaaaaaa"),
            "2026-03-08T10-00-00-aaaaaaaa"
        );
        assert_eq!(
            canonical_session_id("2026-03-08T10-00-00-aaaaaaaa"),
            "2026-03-08T10-00-00-aaaaaaaa"
        );
    }

    #[test]
    fn assistant_final_produces_no_events() {
        let events = collect_event_kinds(
            "turn-1",
            &StorageEvent::AssistantFinal {
                content: "hello world".to_string(),
            },
        );

        assert!(
            events.is_empty(),
            "AssistantFinal should not produce IPC events (content arrives via deltas)"
        );
    }

    #[test]
    fn assistant_delta_produces_only_model_delta() {
        let events = collect_event_kinds(
            "turn-2",
            &StorageEvent::AssistantDelta {
                token: "hello".to_string(),
            },
        );

        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            AgentEventKind::ModelDelta { turn_id, delta }
            if turn_id == "turn-2" && delta == "hello"
        ));
    }

    #[test]
    fn tool_result_preserves_output_and_failure_state() {
        let events = collect_event_kinds(
            "turn-3",
            &StorageEvent::ToolResult {
                tool_call_id: "tool-1".to_string(),
                output: "boom".to_string(),
                success: false,
                duration_ms: 42,
            },
        );

        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            AgentEventKind::ToolCallResult { result, .. }
            if result.tool_call_id == "tool-1"
                && result.output == "boom"
                && result.error.as_deref() == Some("boom")
                && !result.ok
                && result.duration_ms == 42
        ));
    }

    #[test]
    fn convert_events_to_user_and_assistant_messages() {
        use chrono::Utc;

        let events = vec![
            StorageEvent::UserMessage {
                content: "hello".to_string(),
                timestamp: Utc::now(),
            },
            StorageEvent::AssistantFinal {
                content: "hi there".to_string(),
            },
        ];

        let messages = convert_events_to_messages(&events);
        assert_eq!(messages.len(), 2);
        assert!(matches!(&messages[0], SessionMessage::User { content, .. } if content == "hello"));
        assert!(
            matches!(&messages[1], SessionMessage::Assistant { content, .. } if content == "hi there")
        );
    }

    #[test]
    fn convert_events_merges_tool_call_and_result() {
        use serde_json::json;

        let events = vec![
            StorageEvent::ToolCall {
                tool_call_id: "tc-1".to_string(),
                tool_name: "listDir".to_string(),
                args: json!({ "path": "." }),
            },
            StorageEvent::ToolResult {
                tool_call_id: "tc-1".to_string(),
                output: "files listed".to_string(),
                success: true,
                duration_ms: 100,
            },
        ];

        let messages = convert_events_to_messages(&events);
        assert_eq!(messages.len(), 1);
        match &messages[0] {
            SessionMessage::ToolCall {
                tool_name,
                output,
                success,
                duration_ms,
                ..
            } => {
                assert_eq!(tool_name, "listDir");
                assert_eq!(output, &Some("files listed".to_string()));
                assert_eq!(success, &Some(true));
                assert_eq!(duration_ms, &Some(100));
            }
            _ => panic!("expected ToolCall"),
        }
    }

    #[test]
    fn convert_events_ignores_transient_events() {
        use chrono::Utc;

        let events = vec![
            StorageEvent::SessionStart {
                session_id: "s-1".to_string(),
                timestamp: Utc::now(),
                working_dir: "/tmp".to_string(),
            },
            StorageEvent::AssistantDelta {
                token: "partial".to_string(),
            },
            StorageEvent::TurnDone {
                timestamp: Utc::now(),
            },
        ];

        let messages = convert_events_to_messages(&events);
        assert!(messages.is_empty(), "transient events should be ignored");
    }

    #[test]
    fn api_key_preview_masks_values_and_env_names() {
        assert_eq!(api_key_preview(None), "未配置");
        assert_eq!(
            api_key_preview(Some("DEEPSEEK_API_KEY")),
            "环境变量: DEEPSEEK_API_KEY"
        );
        assert_eq!(api_key_preview(Some("abcd")), "****");
        assert_eq!(api_key_preview(Some("secret-1234")), "****1234");
    }

    #[test]
    fn resolve_active_selection_falls_back_and_returns_warning() {
        let profiles = vec![Profile {
            name: "default".to_string(),
            models: vec!["model-a".to_string(), "model-b".to_string()],
            ..Profile::default()
        }];

        let (profile, model, warning) = resolve_active_selection("missing", "model-z", &profiles)
            .expect("fallback should work");

        assert_eq!(profile, "default");
        assert_eq!(model, "model-a");
        assert_eq!(
            warning.as_deref(),
            Some("配置中的 Profile 不存在，已自动选择 default")
        );
    }

    #[tokio::test]
    async fn set_model_writes_config_json() {
        let _guard = AppHomeGuard::new();
        save_config(&Config {
            active_profile: "deepseek".to_string(),
            active_model: "model-a".to_string(),
            profiles: vec![Profile {
                name: "deepseek".to_string(),
                models: vec!["model-a".to_string(), "model-b".to_string()],
                api_key: Some("sk-test".to_string()),
                ..Profile::default()
            }],
            ..Config::default()
        })
        .expect("config should save");

        let handle = AgentHandle::new().expect("handle should build");
        handle
            .set_model("deepseek".to_string(), "model-b".to_string())
            .await
            .expect("set_model should succeed");

        let updated = load_config().expect("config should load");
        assert_eq!(updated.active_profile, "deepseek");
        assert_eq!(updated.active_model, "model-b");
    }

    #[tokio::test]
    async fn set_model_errors_for_missing_profile() {
        let _guard = AppHomeGuard::new();
        save_config(&Config {
            profiles: vec![Profile {
                name: "deepseek".to_string(),
                models: vec!["model-a".to_string()],
                api_key: Some("sk-test".to_string()),
                ..Profile::default()
            }],
            ..Config::default()
        })
        .expect("config should save");

        let handle = AgentHandle::new().expect("handle should build");
        let err = handle
            .set_model("missing".to_string(), "model-a".to_string())
            .await
            .expect_err("missing profile should fail");

        assert!(err.contains("profile 'missing' does not exist"));
    }

    #[tokio::test]
    async fn set_model_errors_for_missing_model() {
        let _guard = AppHomeGuard::new();
        save_config(&Config {
            profiles: vec![Profile {
                name: "deepseek".to_string(),
                models: vec!["model-a".to_string()],
                api_key: Some("sk-test".to_string()),
                ..Profile::default()
            }],
            ..Config::default()
        })
        .expect("config should save");

        let handle = AgentHandle::new().expect("handle should build");
        let err = handle
            .set_model("deepseek".to_string(), "model-b".to_string())
            .await
            .expect_err("missing model should fail");

        assert!(err.contains("model 'model-b' does not exist in profile 'deepseek'"));
    }

    #[tokio::test]
    async fn get_current_model_falls_back_without_writing_config() {
        let _guard = AppHomeGuard::new();
        let config = Config {
            active_profile: "missing".to_string(),
            active_model: "missing-model".to_string(),
            profiles: vec![Profile {
                name: "deepseek".to_string(),
                provider_kind: "openai-compatible".to_string(),
                models: vec!["model-a".to_string(), "model-b".to_string()],
                api_key: Some("sk-test".to_string()),
                ..Profile::default()
            }],
            ..Config::default()
        };
        save_config(&config).expect("config should save");

        let handle = AgentHandle::new().expect("handle should build");
        let current = handle
            .get_current_model()
            .await
            .expect("get_current_model should succeed");

        assert_eq!(current.profile_name, "deepseek");
        assert_eq!(current.model, "model-a");
        assert_eq!(current.provider_kind, "openai-compatible");

        let persisted = load_config().expect("config should still load");
        assert_eq!(persisted.active_profile, "missing");
        assert_eq!(persisted.active_model, "missing-model");
    }
}
