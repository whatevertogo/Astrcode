#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod commands;
mod paths;
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::paths::{resolve_home_dir, runtime_sidecar_dir};
use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use tauri::async_runtime;
use tauri::{Manager, Url, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_shell::{
    process::{CommandChild, CommandEvent},
    ShellExt,
};

type SpawnedSidecarPath = Arc<Mutex<Option<PathBuf>>>;

struct ServerState {
    child: Mutex<Option<CommandChild>>,
    shutting_down: Arc<AtomicBool>,
    spawned_sidecar_path: SpawnedSidecarPath,
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
            let window = create_main_window(app.handle())?;
            window
                .eval(&bootstrap_script.0)
                .map_err(|error| anyhow!(error.to_string()))?;
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
                    cleanup_spawned_sidecar(&state.spawned_sidecar_path);
                }
            }
        });
}

fn initialize_server(app_handle: &tauri::AppHandle) -> Result<(ServerState, BootstrapScript)> {
    if let Some(run_info) = try_connect_existing_server()? {
        return Ok((
            ServerState {
                child: Mutex::new(None),
                shutting_down: Arc::new(AtomicBool::new(false)),
                spawned_sidecar_path: Arc::new(Mutex::new(None)),
            },
            build_bootstrap_script(&run_info)?,
        ));
    }

    let shutting_down = Arc::new(AtomicBool::new(false));
    let spawned_sidecar_path = Arc::new(Mutex::new(None));
    let (pid, child) = spawn_server_process(
        app_handle,
        shutting_down.clone(),
        spawned_sidecar_path.clone(),
    )?;
    let run_info = match wait_for_run_info(pid) {
        Ok(run_info) => run_info,
        Err(error) => {
            let _ = child.kill();
            cleanup_spawned_sidecar(&spawned_sidecar_path);
            return Err(error);
        }
    };
    let started_at = run_info
        .started_at
        .as_deref()
        .unwrap_or("unknown-start-time");
    if let Err(error) = wait_for_server_http_ready(run_info.port).with_context(|| {
        format!(
            "server pid {} (startedAt={started_at}) did not become ready on port {}",
            run_info.pid, run_info.port
        )
    }) {
        let _ = child.kill();
        cleanup_spawned_sidecar(&spawned_sidecar_path);
        return Err(error);
    }
    Ok((
        ServerState {
            child: Mutex::new(Some(child)),
            shutting_down,
            spawned_sidecar_path,
        },
        build_bootstrap_script(&run_info)?,
    ))
}

fn create_main_window(app_handle: &tauri::AppHandle) -> Result<tauri::WebviewWindow> {
    if let Some(window) = app_handle.get_webview_window("main") {
        return Ok(window);
    }

    let mut window_config = app_handle
        .config()
        .app
        .windows
        .iter()
        .find(|config| config.label == "main")
        .cloned()
        .ok_or_else(|| anyhow!("main window config is missing"))?;
    window_config.url = resolve_main_window_url(app_handle)?;

    WebviewWindowBuilder::from_config(app_handle, &window_config)
        .context("failed to build main window from config")?
        .build()
        .context("failed to create main window")
}

fn build_bootstrap_script(run_info: &RunInfo) -> Result<BootstrapScript> {
    let bootstrap = serde_json::json!({
        "token": run_info.token,
        "isDesktopHost": true,
        "serverOrigin": format!("http://127.0.0.1:{}", run_info.port),
    });
    Ok(BootstrapScript(format!(
        "window.__ASTRCODE_BOOTSTRAP__ = {};",
        serde_json::to_string(&bootstrap)?
    )))
}

fn try_connect_existing_server() -> Result<Option<RunInfo>> {
    let path = run_info_path()?;
    if !path.is_file() {
        return Ok(None);
    }

    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read run info '{}'", path.display()))?;
    let run_info: RunInfo = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse run info '{}'", path.display()))?;

    if !run_info_is_fresh(&run_info)? {
        return Ok(None);
    }

    if wait_for_server_http_ready(run_info.port).is_err() {
        return Ok(None);
    }

    Ok(Some(run_info))
}

fn resolve_main_window_url(app_handle: &tauri::AppHandle) -> Result<WebviewUrl> {
    let Some(dev_url) = app_handle.config().build.dev_url.as_ref() else {
        return Ok(WebviewUrl::App("index.html".into()));
    };

    if !cfg!(dev) {
        return Ok(WebviewUrl::App("index.html".into()));
    }

    // 开发环境优先直连 Vite，这样 `cargo tauri dev` 仍保留 HMR。
    if dev_server_is_reachable(dev_url) {
        return Ok(WebviewUrl::External(dev_url.clone()));
    }

    // 当开发服务器未启动时，调试 exe 退回到内置前端资源，避免直接双击
    // `target/debug/astrcode.exe` 只看到 “localhost refused to connect”。
    Ok(WebviewUrl::App("index.html".into()))
}

fn spawn_server_process(
    app_handle: &tauri::AppHandle,
    shutting_down: Arc<AtomicBool>,
    spawned_sidecar_path: SpawnedSidecarPath,
) -> Result<(u32, CommandChild)> {
    let detached_sidecar_path = prepare_detached_sidecar_copy()?;
    let sidecar = app_handle
        .shell()
        .command(&detached_sidecar_path)
        .current_dir(detached_sidecar_path.parent().unwrap_or(Path::new(".")));
    let (mut events, child) = match sidecar.spawn().with_context(|| {
        format!(
            "failed to spawn astrcode-server sidecar from '{}'",
            detached_sidecar_path.display()
        )
    }) {
        Ok(result) => result,
        Err(error) => {
            if let Err(remove_error) = std::fs::remove_file(&detached_sidecar_path) {
                if remove_error.kind() != ErrorKind::NotFound {
                    eprintln!(
                        "[astrcode-server cleanup] failed to remove unspawned detached sidecar '{}': {remove_error}",
                        detached_sidecar_path.display()
                    );
                }
            }
            return Err(error);
        }
    };
    {
        let mut slot = spawned_sidecar_path
            .lock()
            .map_err(|_| anyhow!("spawned sidecar path mutex poisoned"))?;
        *slot = Some(detached_sidecar_path.clone());
    }
    let app_handle = app_handle.clone();
    let cleanup_path = spawned_sidecar_path.clone();
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

        cleanup_spawned_sidecar(&cleanup_path);
    });
    let pid = child.pid();
    Ok((pid, child))
}

fn dev_server_is_reachable(dev_url: &Url) -> bool {
    let host = match dev_url.host_str() {
        Some(host) => host,
        None => return false,
    };
    let port = match dev_url.port_or_known_default() {
        Some(port) => port,
        None => return false,
    };

    let mut stream = match TcpStream::connect((host, port)) {
        Ok(stream) => stream,
        Err(_) => return false,
    };

    if stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .is_err()
    {
        return false;
    }
    if stream
        .set_write_timeout(Some(Duration::from_millis(100)))
        .is_err()
    {
        return false;
    }

    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .is_ok()
}

fn prepare_detached_sidecar_copy() -> Result<PathBuf> {
    let source_path = resolve_packaged_sidecar_path()?;
    let sidecar_dir = runtime_sidecar_dir()?;
    std::fs::create_dir_all(&sidecar_dir).with_context(|| {
        format!(
            "failed to create runtime sidecar directory '{}'",
            sidecar_dir.display()
        )
    })?;

    // 先清理历史遗留副本，避免调试环境长期堆积无主进程留下的 sidecar。
    cleanup_stale_sidecar_copies(&sidecar_dir)?;

    let target_path = sidecar_dir.join(runtime_sidecar_file_name(current_time_ms()?));
    std::fs::copy(&source_path, &target_path).with_context(|| {
        format!(
            "failed to copy sidecar from '{}' to '{}'",
            source_path.display(),
            target_path.display()
        )
    })?;

    Ok(target_path)
}

fn resolve_packaged_sidecar_path() -> Result<PathBuf> {
    let current_exe =
        std::env::current_exe().context("failed to resolve current desktop executable")?;
    let exe_dir = current_exe
        .parent()
        .ok_or_else(|| anyhow!("current desktop executable has no parent directory"))?;
    let base_dir = if exe_dir.ends_with("deps") {
        exe_dir.parent().unwrap_or(exe_dir)
    } else {
        exe_dir
    };

    let sidecar_path = base_dir.join(packaged_sidecar_file_name());
    if !sidecar_path.is_file() {
        return Err(anyhow!(
            "desktop sidecar '{}' does not exist",
            sidecar_path.display()
        ));
    }

    Ok(sidecar_path)
}

fn packaged_sidecar_file_name() -> &'static str {
    if cfg!(windows) {
        "astrcode-server.exe"
    } else {
        "astrcode-server"
    }
}

fn runtime_sidecar_file_name(timestamp_ms: i64) -> String {
    if cfg!(windows) {
        format!(
            "astrcode-server-runtime-{}-{timestamp_ms}.exe",
            std::process::id()
        )
    } else {
        format!(
            "astrcode-server-runtime-{}-{timestamp_ms}",
            std::process::id()
        )
    }
}

fn cleanup_stale_sidecar_copies(sidecar_dir: &Path) -> Result<()> {
    for entry in std::fs::read_dir(sidecar_dir).with_context(|| {
        format!(
            "failed to read runtime sidecar directory '{}'",
            sidecar_dir.display()
        )
    })? {
        let entry = entry?;
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !file_name.starts_with("astrcode-server-runtime-") {
            continue;
        }

        if let Err(error) = std::fs::remove_file(&path) {
            // 正在运行的副本在 Windows 上会返回 PermissionDenied；保留它比误删活进程更安全。
            if error.kind() != ErrorKind::NotFound && error.kind() != ErrorKind::PermissionDenied {
                eprintln!(
                    "[astrcode-server cleanup] failed to remove stale sidecar '{}': {error}",
                    path.display()
                );
            }
        }
    }

    Ok(())
}

fn cleanup_spawned_sidecar(spawned_sidecar_path: &SpawnedSidecarPath) {
    let path = match spawned_sidecar_path.lock() {
        Ok(slot) => slot.clone(),
        Err(_) => {
            eprintln!("[astrcode-server cleanup] spawned sidecar path mutex poisoned");
            return;
        }
    };
    let Some(path) = path else {
        return;
    };

    match std::fs::remove_file(&path) {
        Ok(()) => clear_spawned_sidecar_path(spawned_sidecar_path),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            clear_spawned_sidecar_path(spawned_sidecar_path);
        }
        Err(error) => {
            eprintln!(
                "[astrcode-server cleanup] failed to remove detached sidecar '{}': {error}",
                path.display()
            );
        }
    }
}

fn clear_spawned_sidecar_path(spawned_sidecar_path: &SpawnedSidecarPath) {
    match spawned_sidecar_path.lock() {
        Ok(mut slot) => *slot = None,
        Err(_) => eprintln!("[astrcode-server cleanup] spawned sidecar path mutex poisoned"),
    }
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
