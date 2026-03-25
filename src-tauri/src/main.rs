#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod commands;
mod paths;
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
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
    shutting_down: Arc<AtomicBool>,
}

#[derive(Clone)]
struct BootstrapScript(String);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunInfo {
    port: u16,
    token: String,
    pid: u32,
    started_at: Option<String>,
    expires_at_ms: Option<i64>,
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
                    state.shutting_down.store(true, Ordering::SeqCst);
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
    let shutting_down = Arc::new(AtomicBool::new(false));
    let (pid, child) = spawn_server_process(app_handle, shutting_down.clone())?;
    let run_info = wait_for_run_info(pid)?;
    let started_at = run_info
        .started_at
        .as_deref()
        .unwrap_or("unknown-start-time");
    wait_for_server_http_ready(run_info.port).with_context(|| {
        format!(
            "server pid {} (startedAt={started_at}) did not become ready on port {}",
            run_info.pid, run_info.port
        )
    })?;
    let bootstrap = serde_json::json!({
        "token": run_info.token,
        "isDesktopHost": true,
        "serverOrigin": format!("http://127.0.0.1:{}", run_info.port),
    });

    Ok((
        ServerState {
            child: Mutex::new(Some(child)),
            shutting_down,
        },
        BootstrapScript(format!(
            "window.__ASTRCODE_BOOTSTRAP__ = {};",
            serde_json::to_string(&bootstrap)?
        )),
    ))
}

fn spawn_server_process(
    app_handle: &tauri::AppHandle,
    shutting_down: Arc<AtomicBool>,
) -> Result<(u32, CommandChild)> {
    let sidecar = app_handle
        .shell()
        .sidecar("astrcode-server")
        .context("failed to prepare astrcode-server sidecar")?;
    let (mut events, child) = sidecar
        .spawn()
        .context("failed to spawn astrcode-server sidecar")?;
    let app_handle = app_handle.clone();
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
                    if !shutting_down.load(Ordering::SeqCst) {
                        eprintln!(
                            "[astrcode-server] sidecar reported an error, closing desktop host"
                        );
                        app_handle.exit(1);
                    }
                }
                CommandEvent::Terminated(payload) => {
                    if shutting_down.load(Ordering::SeqCst) {
                        if payload.code.unwrap_or_default() != 0 {
                            eprintln!(
                                "[astrcode-server exited] code={:?} signal={:?}",
                                payload.code, payload.signal
                            );
                        }
                        continue;
                    }

                    eprintln!(
                        "[astrcode-server exited] code={:?} signal={:?}; closing desktop host",
                        payload.code, payload.signal
                    );
                    let exit_code = payload.code.filter(|code| *code != 0).unwrap_or(1);
                    app_handle.exit(exit_code);
                }
                _ => {}
            }
        }
    });
    let pid = child.pid();
    Ok((pid, child))
}

fn wait_for_run_info(pid: u32) -> Result<RunInfo> {
    let path = run_info_path()?;
    for _ in 0..100 {
        if path.exists() {
            let raw = std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read run info '{}'", path.display()))?;
            let run_info: RunInfo = serde_json::from_str(&raw)
                .with_context(|| format!("failed to parse run info '{}'", path.display()))?;
            if run_info.pid == pid && run_info_is_fresh(&run_info)? {
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

fn wait_for_server_http_ready(port: u16) -> Result<()> {
    for _ in 0..100 {
        match probe_server_http_ready(port) {
            Ok(true) => return Ok(()),
            Ok(false) => std::thread::sleep(Duration::from_millis(100)),
            Err(error) => return Err(error),
        }
    }

    Err(anyhow!(
        "timed out waiting for server HTTP readiness on port {}",
        port
    ))
}

fn probe_server_http_ready(port: u16) -> Result<bool> {
    let mut stream = match TcpStream::connect(("127.0.0.1", port)) {
        Ok(stream) => stream,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::ConnectionAborted
                    | ErrorKind::ConnectionRefused
                    | ErrorKind::ConnectionReset
                    | ErrorKind::NotConnected
                    | ErrorKind::TimedOut
                    | ErrorKind::WouldBlock
            ) =>
        {
            return Ok(false);
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to connect to astrcode-server on port {}", port));
        }
    };

    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .context("failed to configure server readiness read timeout")?;
    stream
        .set_write_timeout(Some(Duration::from_millis(100)))
        .context("failed to configure server readiness write timeout")?;
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .context("failed to write server readiness probe")?;

    let mut buffer = [0_u8; 64];
    match stream.read(&mut buffer) {
        Ok(0) => Ok(false),
        Ok(read) => {
            let response_head = String::from_utf8_lossy(&buffer[..read]);
            Ok(response_head.starts_with("HTTP/1.1 200")
                || response_head.starts_with("HTTP/1.0 200"))
        }
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::ConnectionReset | ErrorKind::TimedOut | ErrorKind::WouldBlock
            ) =>
        {
            Ok(false)
        }
        Err(error) => Err(error).context("failed to read server readiness probe"),
    }
}

fn run_info_is_fresh(run_info: &RunInfo) -> Result<bool> {
    let Some(expires_at_ms) = run_info.expires_at_ms else {
        return Ok(true);
    };
    Ok(current_time_ms()? <= expires_at_ms)
}

fn current_time_ms() -> Result<i64> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("system clock is before unix epoch")?;
    Ok(now.as_millis() as i64)
}

fn run_info_path() -> Result<PathBuf> {
    Ok(resolve_home_dir()?.join(".astrcode").join("run.json"))
}
