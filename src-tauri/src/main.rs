#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod commands;
mod paths;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use crate::paths::resolve_home_dir;
use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use tauri::async_runtime;
use tauri::Manager;
use tauri_plugin_shell::{
    process::{CommandChild, CommandEvent},
    ShellExt,
};

struct ServerState {
    child: Mutex<Option<CommandChild>>,
}

#[derive(Clone)]
struct BootstrapScript(String);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunInfo {
    port: u16,
    token: String,
    pid: u32,
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let (server_state, bootstrap_script) = initialize_server(app.handle())?;
            app.manage(server_state);
            app.manage(bootstrap_script.clone());
            if let Some(window) = app.get_webview_window("main") {
                window
                    .eval(&bootstrap_script.0)
                    .map_err(|error| anyhow!(error.to_string()))?;
            }
            Ok(())
        })
        .on_page_load(|window, _payload| {
            if let Some(bootstrap) = window.app_handle().try_state::<BootstrapScript>() {
                let _ = window.eval(&bootstrap.0);
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::minimize_window,
            commands::maximize_window,
            commands::close_window,
            commands::select_directory,
            commands::open_config_in_editor,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            if matches!(event, tauri::RunEvent::ExitRequested { .. }) {
                if let Some(state) = app_handle.try_state::<ServerState>() {
                    if let Ok(mut child) = state.child.lock() {
                        if let Some(child) = child.take() {
                            let _ = child.kill();
                        }
                    }
                }
            }
        });
}

fn initialize_server(app_handle: &tauri::AppHandle) -> Result<(ServerState, BootstrapScript)> {
    let (pid, child) = spawn_server_process(app_handle)?;
    let run_info = wait_for_run_info(pid)?;
    let bootstrap = serde_json::json!({
        "token": run_info.token,
        "isDesktopHost": true,
        "serverOrigin": format!("http://127.0.0.1:{}", run_info.port),
    });

    Ok((
        ServerState {
            child: Mutex::new(Some(child)),
        },
        BootstrapScript(format!(
            "window.__ASTRCODE_BOOTSTRAP__ = {};",
            serde_json::to_string(&bootstrap)?
        )),
    ))
}

fn spawn_server_process(app_handle: &tauri::AppHandle) -> Result<(u32, CommandChild)> {
    let sidecar = app_handle
        .shell()
        .sidecar("astrcode-server")
        .context("failed to prepare astrcode-server sidecar")?;
    let (mut events, child) = sidecar
        .spawn()
        .context("failed to spawn astrcode-server sidecar")?;
    async_runtime::spawn(async move {
        while let Some(event) = events.recv().await {
            match event {
                CommandEvent::Stdout(line) => {
                    #[cfg(debug_assertions)]
                    eprintln!(
                        "[astrcode-server stdout] {}",
                        String::from_utf8_lossy(&line)
                    );
                }
                CommandEvent::Stderr(line) => {
                    eprintln!(
                        "[astrcode-server stderr] {}",
                        String::from_utf8_lossy(&line)
                    );
                }
                CommandEvent::Error(error) => {
                    eprintln!("[astrcode-server error] {error}");
                }
                CommandEvent::Terminated(payload) => {
                    if payload.code.unwrap_or_default() != 0 {
                        eprintln!(
                            "[astrcode-server exited] code={:?} signal={:?}",
                            payload.code, payload.signal
                        );
                    }
                }
                _ => {}
            }
        }
    });
    let pid = child.pid();
    Ok((pid, child))
}

fn wait_for_run_info(pid: u32) -> Result<RunInfo> {
    let path = run_info_path();
    for _ in 0..100 {
        if path.exists() {
            let raw = std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read run info '{}'", path.display()))?;
            let run_info: RunInfo = serde_json::from_str(&raw)
                .with_context(|| format!("failed to parse run info '{}'", path.display()))?;
            if run_info.pid == pid {
                return Ok(run_info);
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    Err(anyhow!(
        "timed out waiting for run info matching server pid {}",
        pid
    ))
}

fn run_info_path() -> PathBuf {
    resolve_home_dir().join(".astrcode").join("run.json")
}
