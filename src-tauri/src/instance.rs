use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use fs2::FileExt;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::paths::{desktop_instance_info_path, desktop_instance_lock_path};

const INSTANCE_CONNECT_TIMEOUT: Duration = Duration::from_millis(250);
const INSTANCE_RETRY_TIMEOUT: Duration = Duration::from_secs(5);
const INSTANCE_RETRY_INTERVAL: Duration = Duration::from_millis(100);

pub enum InstanceBootstrap {
    Primary(Arc<DesktopInstanceCoordinator>),
    ActivatedExisting,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopInstanceInfo {
    port: u16,
    token: String,
    pid: u32,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopActivationRequest {
    token: String,
    action: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopActivationResponse {
    ok: bool,
}

pub struct DesktopInstanceCoordinator {
    _lock_file: File,
    info_path: std::path::PathBuf,
    info: DesktopInstanceInfo,
    app_handle: Arc<Mutex<Option<AppHandle>>>,
    listener_shutdown: Arc<AtomicBool>,
    main_window_ready: Arc<AtomicBool>,
    pending_focus_request: Arc<AtomicBool>,
    listener_thread: Mutex<Option<JoinHandle<()>>>,
}

impl DesktopInstanceCoordinator {
    pub fn bootstrap() -> Result<InstanceBootstrap> {
        let lock_path = desktop_instance_lock_path()?;
        if let Some(parent) = lock_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create desktop instance runtime directory '{}'",
                    parent.display()
                )
            })?;
        }

        let lock_file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&lock_path)
            .with_context(|| {
                format!(
                    "failed to open desktop instance lock '{}'",
                    lock_path.display()
                )
            })?;

        match lock_file.try_lock_exclusive() {
            Ok(()) => Self::start_primary(lock_file).map(InstanceBootstrap::Primary),
            Err(error) if is_lock_busy(&error) => {
                notify_existing_instance()?;
                Ok(InstanceBootstrap::ActivatedExisting)
            }
            Err(error) => Err(error).with_context(|| {
                format!(
                    "failed to acquire desktop instance lock '{}'",
                    lock_path.display()
                )
            }),
        }
    }

    fn start_primary(lock_file: File) -> Result<Arc<Self>> {
        let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
            .context("failed to bind desktop instance loopback listener")?;
        listener
            .set_nonblocking(true)
            .context("failed to make desktop instance listener non-blocking")?;

        let info_path = desktop_instance_info_path()?;
        if let Some(parent) = info_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create desktop instance info directory '{}'",
                    parent.display()
                )
            })?;
        }

        let info = DesktopInstanceInfo {
            port: listener
                .local_addr()
                .context("failed to resolve desktop instance listener address")?
                .port(),
            token: random_hex_token(),
            pid: std::process::id(),
        };
        write_instance_info(&info_path, &info)?;

        let listener_shutdown = Arc::new(AtomicBool::new(false));
        let pending_focus_request = Arc::new(AtomicBool::new(false));
        let app_handle = Arc::new(Mutex::new(None));
        let main_window_ready = Arc::new(AtomicBool::new(false));
        let listener_thread = {
            let listener_shutdown = Arc::clone(&listener_shutdown);
            let pending_focus_request = Arc::clone(&pending_focus_request);
            let app_handle = Arc::clone(&app_handle);
            let main_window_ready = Arc::clone(&main_window_ready);
            let token = info.token.clone();
            std::thread::spawn(move || {
                run_instance_listener(
                    listener,
                    listener_shutdown,
                    app_handle,
                    main_window_ready,
                    pending_focus_request,
                    token,
                );
            })
        };

        Ok(Arc::new(Self {
            _lock_file: lock_file,
            info_path,
            info,
            app_handle,
            listener_shutdown,
            main_window_ready,
            pending_focus_request,
            listener_thread: Mutex::new(Some(listener_thread)),
        }))
    }

    pub fn attach_app_handle(&self, app_handle: AppHandle) {
        if let Ok(mut slot) = self.app_handle.lock() {
            *slot = Some(app_handle);
        }
        self.flush_pending_focus_request();
    }

    pub fn mark_main_window_ready(&self) {
        self.main_window_ready.store(true, Ordering::SeqCst);
        self.flush_pending_focus_request();
    }

    pub fn shutdown(&self) {
        self.listener_shutdown.store(true, Ordering::SeqCst);
        if let Ok(mut slot) = self.listener_thread.lock() {
            if let Some(handle) = slot.take() {
                let _ = handle.join();
            }
        }

        if let Err(error) = remove_instance_info(&self.info_path, self.info.pid) {
            eprintln!(
                "[astrcode-instance] failed to remove desktop instance info '{}': {error}",
                self.info_path.display()
            );
        }
    }

    fn flush_pending_focus_request(&self) {
        if !self.pending_focus_request.swap(false, Ordering::SeqCst) {
            return;
        }

        trigger_or_queue_focus(
            &self.app_handle,
            &self.main_window_ready,
            &self.pending_focus_request,
        );
    }
}

fn notify_existing_instance() -> Result<()> {
    let deadline = Instant::now() + INSTANCE_RETRY_TIMEOUT;
    let info_path = desktop_instance_info_path()?;
    loop {
        if let Ok(info) = read_instance_info(&info_path) {
            if send_focus_request(&info).is_ok() {
                return Ok(());
            }
        }

        if Instant::now() >= deadline {
            return Err(anyhow!(
                "another AstrCode desktop instance holds the single-instance lock, but it did not accept activation within {:?}",
                INSTANCE_RETRY_TIMEOUT
            ));
        }

        std::thread::sleep(INSTANCE_RETRY_INTERVAL);
    }
}

fn send_focus_request(info: &DesktopInstanceInfo) -> Result<()> {
    let mut stream = TcpStream::connect_timeout(
        &SocketAddr::from(([127, 0, 0, 1], info.port)),
        INSTANCE_CONNECT_TIMEOUT,
    )
    .with_context(|| {
        format!(
            "failed to connect to desktop instance IPC port {}",
            info.port
        )
    })?;
    stream
        .set_read_timeout(Some(INSTANCE_CONNECT_TIMEOUT))
        .context("failed to configure desktop instance IPC read timeout")?;
    stream
        .set_write_timeout(Some(INSTANCE_CONNECT_TIMEOUT))
        .context("failed to configure desktop instance IPC write timeout")?;

    let payload = serde_json::to_vec(&DesktopActivationRequest {
        token: info.token.clone(),
        action: "focus".to_string(),
    })
    .context("failed to serialize desktop activation request")?;
    stream
        .write_all(&payload)
        .context("failed to write desktop activation request")?;
    stream
        .shutdown(Shutdown::Write)
        .context("failed to finish desktop activation request")?;

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .context("failed to read desktop activation response")?;
    let response: DesktopActivationResponse = serde_json::from_str(response.trim())
        .context("failed to parse desktop activation response")?;
    if !response.ok {
        return Err(anyhow!("desktop activation request was rejected"));
    }

    Ok(())
}

fn run_instance_listener(
    listener: TcpListener,
    listener_shutdown: Arc<AtomicBool>,
    app_handle: Arc<Mutex<Option<AppHandle>>>,
    main_window_ready: Arc<AtomicBool>,
    pending_focus_request: Arc<AtomicBool>,
    token: String,
) {
    while !listener_shutdown.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let accepted = handle_activation_stream(&mut stream, &token);
                let response = DesktopActivationResponse { ok: accepted };
                if let Ok(payload) = serde_json::to_vec(&response) {
                    let _ = stream.write_all(&payload);
                }
                if accepted {
                    trigger_or_queue_focus(&app_handle, &main_window_ready, &pending_focus_request);
                }
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(INSTANCE_RETRY_INTERVAL);
            }
            Err(error) => {
                eprintln!("[astrcode-instance] desktop IPC listener failed: {error}");
                std::thread::sleep(INSTANCE_RETRY_INTERVAL);
            }
        }
    }
}

fn handle_activation_stream(stream: &mut TcpStream, expected_token: &str) -> bool {
    let mut payload = String::new();
    if stream.read_to_string(&mut payload).is_err() {
        return false;
    }

    let Ok(request) = serde_json::from_str::<DesktopActivationRequest>(payload.trim()) else {
        return false;
    };

    request.token == expected_token && request.action == "focus"
}

fn read_instance_info(path: &Path) -> Result<DesktopInstanceInfo> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read desktop instance info '{}'", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse desktop instance info '{}'", path.display()))
}

fn write_instance_info(path: &Path, info: &DesktopInstanceInfo) -> Result<()> {
    let payload =
        serde_json::to_string_pretty(info).context("failed to serialize desktop instance info")?;
    fs::write(path, payload)
        .with_context(|| format!("failed to write desktop instance info '{}'", path.display()))
}

fn remove_instance_info(path: &Path, expected_pid: u32) -> Result<()> {
    if !path.is_file() {
        return Ok(());
    }

    let info = read_instance_info(path)?;
    if info.pid != expected_pid {
        return Ok(());
    }

    fs::remove_file(path).with_context(|| {
        format!(
            "failed to remove desktop instance info '{}'",
            path.display()
        )
    })
}

fn random_hex_token() -> String {
    let mut bytes = [0_u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn trigger_or_queue_focus(
    app_handle: &Arc<Mutex<Option<AppHandle>>>,
    main_window_ready: &AtomicBool,
    pending_focus_request: &AtomicBool,
) {
    if !main_window_ready.load(Ordering::SeqCst) {
        pending_focus_request.store(true, Ordering::SeqCst);
        return;
    }

    let app_handle = match app_handle.lock() {
        Ok(slot) => slot.clone(),
        Err(_) => {
            pending_focus_request.store(true, Ordering::SeqCst);
            return;
        }
    };
    let Some(app_handle) = app_handle else {
        pending_focus_request.store(true, Ordering::SeqCst);
        return;
    };

    if app_handle.get_webview_window("main").is_none() {
        pending_focus_request.store(true, Ordering::SeqCst);
        return;
    }

    let app_handle_for_ui = app_handle.clone();
    if let Err(error) = app_handle.run_on_main_thread(move || {
        if let Some(window) = app_handle_for_ui.get_webview_window("main") {
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.set_focus();
        }
    }) {
        eprintln!("[astrcode-instance] failed to focus existing main window: {error}");
        pending_focus_request.store(true, Ordering::SeqCst);
    }
}

fn is_lock_busy(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::WouldBlock | ErrorKind::PermissionDenied
    )
}
