use tauri::{AppHandle, State};

use crate::handle::AgentHandle;

#[tauri::command]
pub async fn submit_prompt(
    text: String,
    state: State<'_, AgentHandle>,
    app: AppHandle,
) -> Result<(), String> {
    state.submit_prompt(text, app).await
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
