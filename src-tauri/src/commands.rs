use tauri::{AppHandle, State};

use crate::handle::AgentHandle;

#[tauri::command]
pub async fn submit_prompt(
    _text: String,
    state: State<'_, AgentHandle>,
    _app: AppHandle,
) -> Result<(), String> {
    let _ = &state.runtime;
    Err("submit_prompt not implemented yet".into())
}

#[tauri::command]
pub async fn interrupt(state: State<'_, AgentHandle>) -> Result<(), String> {
    let _ = &state.runtime;
    Err("interrupt not implemented yet".into())
}

#[tauri::command]
pub fn exit_app(app: AppHandle) {
    app.exit(0);
}
