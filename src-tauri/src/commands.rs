use tauri::ipc::Channel;
use tauri::{AppHandle, State};

use crate::handle::{AgentHandle, ConfigView, CurrentModelInfo, ModelOption, SessionMessage};
use astrcode_core::{DeleteProjectResult, SessionMeta, TestResult};
use ipc::AgentEvent;

#[tauri::command]
pub async fn submit_prompt(
    text: String,
    state: State<'_, AgentHandle>,
    channel: Channel<AgentEvent>,
) -> Result<(), String> {
    state.submit_prompt(text, channel).await
}

#[tauri::command]
pub async fn interrupt(state: State<'_, AgentHandle>) -> Result<(), String> {
    state.interrupt().await
}

#[tauri::command]
pub fn get_working_dir() -> Result<String, String> {
    std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn exit_app(app: AppHandle) {
    app.exit(0);
}

// ────────────────────────────────────────────────────────────
// Session management commands
// ────────────────────────────────────────────────────────────

/// List all session IDs.
#[tauri::command]
pub fn list_sessions() -> Result<Vec<String>, String> {
    AgentHandle::list_sessions()
}

/// List all sessions with metadata.
#[tauri::command]
pub fn list_sessions_with_meta() -> Result<Vec<SessionMeta>, String> {
    AgentHandle::list_sessions_with_meta()
}

/// Load messages from a session.
#[tauri::command]
pub fn load_session(session_id: String) -> Result<Vec<SessionMessage>, String> {
    AgentHandle::load_session(&session_id)
}

/// Switch to an existing session.
#[tauri::command]
pub async fn switch_session(
    session_id: String,
    state: State<'_, AgentHandle>,
) -> Result<String, String> {
    state.switch_session(&session_id).await?;
    Ok(state.get_session_id().await)
}

/// Create a new session.
#[tauri::command]
pub async fn new_session(state: State<'_, AgentHandle>) -> Result<String, String> {
    state.new_session().await
}

/// Get the current session ID.
#[tauri::command]
pub async fn get_session_id(state: State<'_, AgentHandle>) -> Result<String, String> {
    Ok(state.get_session_id().await)
}

#[tauri::command]
pub async fn delete_session(
    session_id: String,
    state: State<'_, AgentHandle>,
) -> Result<(), String> {
    state.delete_session(session_id).await
}

#[tauri::command]
pub async fn delete_project(
    working_dir: String,
    state: State<'_, AgentHandle>,
) -> Result<DeleteProjectResult, String> {
    state.delete_project(working_dir).await
}

#[tauri::command]
pub async fn get_config() -> Result<ConfigView, String> {
    AgentHandle::get_config().await
}

#[tauri::command]
pub async fn save_active_selection(
    active_profile: String,
    active_model: String,
    state: State<'_, AgentHandle>,
) -> Result<(), String> {
    state
        .save_active_selection(active_profile, active_model)
        .await
}

#[tauri::command]
pub async fn set_model(
    profile_name: String,
    model: String,
    state: State<'_, AgentHandle>,
) -> Result<(), String> {
    state.set_model(profile_name, model).await
}

#[tauri::command]
pub async fn get_current_model(state: State<'_, AgentHandle>) -> Result<CurrentModelInfo, String> {
    state.get_current_model().await
}

#[tauri::command]
pub async fn list_available_models(
    state: State<'_, AgentHandle>,
) -> Result<Vec<ModelOption>, String> {
    state.list_available_models().await
}

#[tauri::command]
pub async fn test_connection(
    profile_name: String,
    model: String,
    state: State<'_, AgentHandle>,
) -> Result<TestResult, String> {
    state
        .test_connection_for_selection(profile_name, model)
        .await
}

#[tauri::command]
pub fn open_config_in_editor() -> Result<(), String> {
    AgentHandle::open_config_in_editor()
}
