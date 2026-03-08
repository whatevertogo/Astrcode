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
        .manage(agent)
        .invoke_handler(tauri::generate_handler![
            commands::submit_prompt,
            commands::interrupt,
            commands::exit_app,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
