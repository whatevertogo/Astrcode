mod commands;
mod handle;

use handle::AgentHandle;

fn main() {
    let agent = match AgentHandle::new() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Core 初始化失败: {e}");
            std::process::exit(1);
        }
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(agent)
        .invoke_handler(tauri::generate_handler![
            commands::submit_prompt,
            commands::interrupt,
            commands::get_working_dir,
            commands::exit_app,
            commands::list_sessions,
            commands::list_sessions_with_meta,
            commands::load_session,
            commands::switch_session,
            commands::new_session,
            commands::get_session_id,
            commands::delete_session,
            commands::delete_project,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
