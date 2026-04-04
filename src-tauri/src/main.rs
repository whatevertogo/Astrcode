#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod commands;
mod instance;
mod paths;
use std::{
    io::{ErrorKind, Read, Write},
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, sync_channel},
    },
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use astrcode_core::LocalServerInfo;
use instance::{DesktopInstanceCoordinator, InstanceBootstrap};
use tauri::{Manager, Url, WebviewUrl, WebviewWindowBuilder, async_runtime};
use tauri_plugin_shell::{
    ShellExt,
    process::{CommandChild, CommandEvent},
};

use crate::paths::{resolve_home_dir, runtime_sidecar_dir};

type SpawnedSidecarPath = Arc<Mutex<Option<PathBuf>>>;

struct ServerState {
    child: Mutex<Option<CommandChild>>,
    shutting_down: Arc<AtomicBool>,
    spawned_sidecar_path: SpawnedSidecarPath,
}

fn main() {
    if let Err(error) = run_desktop_shell() {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}

fn run_desktop_shell() -> Result<()> {
    let instance_coordinator = match DesktopInstanceCoordinator::bootstrap()? {
        InstanceBootstrap::Primary(coordinator) => coordinator,
        InstanceBootstrap::ActivatedExisting => return Ok(()),
    };
    let instance_for_setup = Arc::clone(&instance_coordinator);
    let instance_for_run = Arc::clone(&instance_coordinator);

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(move |app| {
            instance_for_setup.attach_app_handle(app.handle().clone());
            let (server_state, bootstrap_script) = initialize_server(app.handle())?;
            app.manage(server_state);
            let app_handle = app.handle().clone();
            let instance_for_window = Arc::clone(&instance_for_setup);
            std::thread::spawn(move || {
                if let Err(error) = create_main_window(&app_handle, &bootstrap_script) {
                    eprintln!("[astrcode-window] failed to create main window: {error:#}");
                    app_handle.exit(1);
                    return;
                }
                instance_for_window.mark_main_window_ready();
            });
            Ok(())
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
        .run(move |app_handle, event| {
            if matches!(event, tauri::RunEvent::ExitRequested { .. }) {
                instance_for_run.shutdown();
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

    Ok(())
}

fn initialize_server(app_handle: &tauri::AppHandle) -> Result<(ServerState, String)> {
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
    let (child, ready_rx) = spawn_server_process(
        app_handle,
        shutting_down.clone(),
        spawned_sidecar_path.clone(),
    )?;

    // 使用 Option 包装 child，以便在错误路径中 take 出来进行清理
    let mut child = Some(child);
    let mut cleanup_failed_spawn = || {
        if let Some(c) = child.take() {
            let _ = c.kill();
        }
        cleanup_spawned_sidecar(&spawned_sidecar_path);
    };

    let run_info = wait_for_sidecar_ready(ready_rx).inspect_err(|_| {
        cleanup_failed_spawn();
    })?;

    let started_at = run_info.started_at.as_str();
    wait_for_server_http_ready(run_info.port).map_err(|error| {
        cleanup_failed_spawn();
        anyhow!(
            "server pid {} (startedAt={started_at}) did not become ready on port {}: {error}",
            run_info.pid,
            run_info.port
        )
    })?;

    Ok((
        ServerState {
            child: Mutex::new(child),
            shutting_down,
            spawned_sidecar_path,
        },
        build_bootstrap_script(&run_info)?,
    ))
}

fn create_main_window(
    app_handle: &tauri::AppHandle,
    bootstrap_script: &str,
) -> Result<tauri::WebviewWindow> {
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

    // Windows 上同步创建 WebView 和同步 `eval` 都踩过 WebView2 死锁面。
    // 这里保留初始化脚本注入，并配合 setup 里的独立线程创建窗口，避开阻塞主 UI 线程。
    WebviewWindowBuilder::from_config(app_handle, &window_config)
        .context("failed to build main window from config")?
        .initialization_script(bootstrap_script)
        .build()
        .context("failed to create main window")
}

fn build_bootstrap_script(run_info: &LocalServerInfo) -> Result<String> {
    let bootstrap = serde_json::json!({
        "token": run_info.token,
        "isDesktopHost": true,
        "serverOrigin": format!("http://127.0.0.1:{}", run_info.port),
    });
    Ok(format!(
        "window.__ASTRCODE_BOOTSTRAP__ = {};",
        serde_json::to_string(&bootstrap)?
    ))
}

fn try_connect_existing_server() -> Result<Option<LocalServerInfo>> {
    let path = run_info_path()?;
    if !path.is_file() {
        return Ok(None);
    }

    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read run info '{}'", path.display()))?;
    let run_info: LocalServerInfo = serde_json::from_str(&raw)
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
) -> Result<(CommandChild, Receiver<Result<LocalServerInfo>>)> {
    let (ready_tx, ready_rx) = sync_channel(1);
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
                        "[astrcode-server cleanup] failed to remove unspawned detached sidecar \
                         '{}': {remove_error}",
                        detached_sidecar_path.display()
                    );
                }
            }
            return Err(error);
        },
    };
    {
        let mut slot = spawned_sidecar_path
            .lock()
            .map_err(|_| anyhow!("spawned sidecar path mutex poisoned"))?;
        *slot = Some(detached_sidecar_path.clone());
    }
    let app_handle = app_handle.clone();
    let cleanup_path = spawned_sidecar_path.clone();
    let sidecar_pid = child.pid();
    async_runtime::spawn(async move {
        let mut ready_tx = Some(ready_tx);
        let mut stdout_buffer = String::new();
        while let Some(event) = events.recv().await {
            match event {
                CommandEvent::Stdout(chunk) => {
                    #[cfg(debug_assertions)]
                    eprintln!(
                        "[astrcode-server stdout] {}",
                        String::from_utf8_lossy(&chunk)
                    );
                    if let Some(tx) = ready_tx.as_ref() {
                        match try_parse_sidecar_ready_chunk(&mut stdout_buffer, &chunk) {
                            Ok(Some(info)) => {
                                let _ = tx.send(Ok(info));
                                ready_tx = None;
                            },
                            Ok(None) => {},
                            Err(error) => {
                                let _ = tx.send(Err(error));
                                ready_tx = None;
                            },
                        }
                    }
                },
                CommandEvent::Stderr(line) => {
                    eprintln!(
                        "[astrcode-server stderr] {}",
                        String::from_utf8_lossy(&line)
                    );
                },
                CommandEvent::Error(error) => {
                    if let Some(tx) = ready_tx.take() {
                        let _ = tx.send(Err(anyhow!(
                            "sidecar pid {} reported an error before becoming ready: {}",
                            sidecar_pid,
                            error
                        )));
                    }
                    eprintln!("[astrcode-server error] {error}");
                    if !shutting_down.load(Ordering::SeqCst) {
                        eprintln!(
                            "[astrcode-server] sidecar reported an error, closing desktop host"
                        );
                        app_handle.exit(1);
                    }
                },
                CommandEvent::Terminated(payload) => {
                    if let Some(tx) = ready_tx.take() {
                        let _ = tx.send(Err(anyhow!(
                            "sidecar pid {} exited before reporting ready: code={:?} signal={:?}",
                            sidecar_pid,
                            payload.code,
                            payload.signal
                        )));
                    }
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
                },
                _ => {},
            }
        }

        if let Some(tx) = ready_tx.take() {
            let _ = tx.send(Err(anyhow!(
                "sidecar pid {} closed stdout before reporting ready",
                sidecar_pid
            )));
        }
        cleanup_spawned_sidecar(&cleanup_path);
    });
    Ok((child, ready_rx))
}

fn dev_server_is_reachable(dev_url: &Url) -> bool {
    let Some(host) = dev_url.host_str() else {
        return false;
    };
    let Some(port) = dev_url.port_or_known_default() else {
        return false;
    };

    let Ok(mut stream) = connect_host_with_timeout(host, port, Duration::from_millis(100)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));

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
    let pid = std::process::id();
    if cfg!(windows) {
        format!("astrcode-server-runtime-{pid}-{timestamp_ms}.exe")
    } else {
        format!("astrcode-server-runtime-{pid}-{timestamp_ms}")
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
        Ok(mut slot) => slot.take(),
        Err(_) => {
            eprintln!("[astrcode-server cleanup] spawned sidecar path mutex poisoned");
            return;
        },
    };
    let Some(path) = path else {
        return;
    };

    if let Err(error) = std::fs::remove_file(&path) {
        // 正在运行或刚终止的 sidecar 在 Windows 上会返回 PermissionDenied；
        // 保留它比误删活进程更安全，下次启动时 cleanup_stale_sidecar_copies 会清理。
        if error.kind() != ErrorKind::NotFound && error.kind() != ErrorKind::PermissionDenied {
            eprintln!(
                "[astrcode-server cleanup] failed to remove detached sidecar '{}': {error}",
                path.display()
            );
        }
    }
}

fn wait_for_sidecar_ready(ready_rx: Receiver<Result<LocalServerInfo>>) -> Result<LocalServerInfo> {
    match ready_rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(info)) => Ok(info),
        Ok(Err(error)) => Err(error),
        Err(_) => Err(anyhow!(
            "timed out waiting for astrcode-server sidecar to report ready"
        )),
    }
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
    let mut stream = match TcpStream::connect_timeout(
        &SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(100),
    ) {
        Ok(stream) => stream,
        Err(error) if is_connection_refused(&error) => return Ok(false),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to connect to astrcode-server on port {port}"));
        },
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
        },
        Err(error) if is_connection_refused(&error) => Ok(false),
        Err(error) => Err(error).context("failed to read server readiness probe"),
    }
}

// Sidecar 的 stdout 既承载普通日志，也承载结构化 ready 事件。
// 这里做按行缓冲，只消费显式带前缀的 ready 行，避免把人类可读日志耦合进启动协议。
fn try_parse_sidecar_ready_chunk(
    buffer: &mut String,
    chunk: &[u8],
) -> Result<Option<LocalServerInfo>> {
    buffer.push_str(&String::from_utf8_lossy(chunk));

    while let Some(line_break) = buffer.find('\n') {
        let line = buffer[..line_break].to_string();
        let remainder = buffer[line_break + 1..].to_string();
        buffer.clear();
        buffer.push_str(&remainder);

        match LocalServerInfo::parse_ready_line(&line) {
            Ok(Some(info)) => return Ok(Some(info)),
            Ok(None) => continue,
            Err(error) => {
                return Err(anyhow!(
                    "failed to parse sidecar ready line from stdout: {error}"
                ));
            },
        }
    }

    Ok(None)
}

// 本地探活必须显式限制 connect 超时；Windows 上 stale `run.json` 对应的旧端口
// 不一定会立即拒绝连接，阻塞式 connect 会把桌面端启动线程一起拖住。
fn connect_host_with_timeout(
    host: &str,
    port: u16,
    timeout: Duration,
) -> std::io::Result<TcpStream> {
    let mut last_error = None;
    for address in (host, port).to_socket_addrs()? {
        match TcpStream::connect_timeout(&address, timeout) {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = Some(error),
        }
    }

    Err(last_error.unwrap_or_else(|| {
        std::io::Error::new(
            ErrorKind::AddrNotAvailable,
            format!("no socket address resolved for {host}:{port}"),
        )
    }))
}

fn is_connection_refused(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::ConnectionAborted
            | ErrorKind::ConnectionRefused
            | ErrorKind::ConnectionReset
            | ErrorKind::NotConnected
            | ErrorKind::TimedOut
            | ErrorKind::WouldBlock
    )
}

fn run_info_is_fresh(run_info: &LocalServerInfo) -> Result<bool> {
    Ok(current_time_ms()? <= run_info.expires_at_ms)
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
