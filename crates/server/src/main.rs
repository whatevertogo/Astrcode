#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod auth;
mod bootstrap;
mod mapper;
mod routes;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Result as AnyhowResult};
use astrcode_core::{AstrError, CapabilityRouter, ToolRegistry};
use astrcode_plugin::{PluginLoader, Supervisor};
use astrcode_protocol::plugin::{PeerDescriptor, PeerRole};
use astrcode_runtime::{RuntimeService, ServiceError};
use astrcode_tools::tools::{
    edit_file::EditFileTool, find_files::FindFilesTool, grep::GrepTool, list_dir::ListDirTool,
    read_file::ReadFileTool, shell::ShellTool, write_file::WriteFileTool,
};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde::Serialize;

#[cfg(all(test, target_os = "windows"))]
use crate::bootstrap::workspace_root;
use crate::bootstrap::{
    attach_frontend_build, build_cors_layer, load_frontend_build, random_hex_token, write_run_info,
};
#[cfg(test)]
use crate::bootstrap::{inject_browser_bootstrap_html, serve_frontend_build};
#[cfg(test)]
use crate::mapper::api_key_preview;
use crate::routes::build_api_router;
#[cfg(test)]
use crate::routes::sessions::session_messages;
#[cfg(test)]
use auth::secure_token_eq;

pub(crate) const AUTH_HEADER_NAME: &str = "x-astrcode-token";
pub(crate) const SESSION_CURSOR_HEADER_NAME: &str = "x-session-cursor";

#[derive(Clone)]
pub(crate) struct AppState {
    service: Arc<RuntimeService>,
    bootstrap_token: String,
    frontend_build: Option<FrontendBuild>,
}

#[derive(Clone)]
pub(crate) struct FrontendBuild {
    dist_dir: PathBuf,
    index_html: Arc<String>,
}

#[derive(Debug, Serialize)]
struct ErrorPayload {
    error: String,
}

#[derive(Debug)]
pub(crate) struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn unauthorized() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: "unauthorized".to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorPayload {
                error: self.message,
            }),
        )
            .into_response()
    }
}

impl From<ServiceError> for ApiError {
    fn from(value: ServiceError) -> Self {
        match value {
            ServiceError::NotFound(message) => Self {
                status: StatusCode::NOT_FOUND,
                message,
            },
            ServiceError::Conflict(message) => Self {
                status: StatusCode::CONFLICT,
                message,
            },
            ServiceError::InvalidInput(message) => Self {
                status: StatusCode::BAD_REQUEST,
                message,
            },
            ServiceError::Internal(error) => Self {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                message: error.to_string(),
            },
        }
    }
}

#[tokio::main]
async fn main() -> AnyhowResult<()> {
    let registry = ToolRegistry::builder()
        .register(Box::new(ShellTool::default()))
        .register(Box::new(ListDirTool::default()))
        .register(Box::new(ReadFileTool::default()))
        .register(Box::new(WriteFileTool::default()))
        .register(Box::new(EditFileTool::default()))
        .register(Box::new(FindFilesTool::default()))
        .register(Box::new(GrepTool::default()))
        .build();
    let (capabilities, _plugin_supervisors) = load_runtime_capabilities(registry)
        .await
        .map_err(|error| anyhow!(error.to_string()))?;
    let service = Arc::new(
        RuntimeService::from_capabilities(capabilities)
            .map_err(|error| anyhow!(error.to_string()))?,
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| AstrError::io("failed to bind server listener", e))?;
    let address: SocketAddr = listener
        .local_addr()
        .map_err(|e| AstrError::io("failed to resolve server listener address", e))?;
    let token = random_hex_token();
    let server_origin = format!("http://127.0.0.1:{}", address.port());
    let frontend_build = load_frontend_build(&server_origin, &token)?;
    write_run_info(address.port(), &token)?;
    println!(
        "Ready: http://localhost:{}/ (API routes live under /api)",
        address.port()
    );

    let state = AppState {
        service,
        bootstrap_token: token.clone(),
        frontend_build: frontend_build.clone(),
    };

    let app: Router<AppState> = build_api_router();
    let app = attach_frontend_build(app, frontend_build);
    let app = app.with_state(state).layer(build_cors_layer());

    Ok(axum::serve(listener, app)
        .await
        .map_err(|e| AstrError::io("server terminated unexpectedly", e))?)
}

async fn load_runtime_capabilities(
    registry: ToolRegistry,
) -> std::result::Result<(CapabilityRouter, Vec<Supervisor>), AstrError> {
    let mut builder = CapabilityRouter::builder().register_tool_registry(registry);
    let mut supervisors = Vec::new();

    let Some(raw_paths) = std::env::var_os("ASTRCODE_PLUGIN_DIRS") else {
        return builder.build().map(|router| (router, supervisors));
    };

    let search_paths = std::env::split_paths(&raw_paths).collect::<Vec<_>>();
    if search_paths.is_empty() {
        return builder.build().map(|router| (router, supervisors));
    }

    let loader = PluginLoader { search_paths };
    for manifest in loader.discover()? {
        let supervisor = Supervisor::start(
            &manifest,
            PeerDescriptor {
                id: "astrcode-server".to_string(),
                name: "astrcode-server".to_string(),
                role: PeerRole::Supervisor,
                version: env!("CARGO_PKG_VERSION").to_string(),
                supported_profiles: vec!["coding".to_string()],
                metadata: serde_json::Value::Null,
            },
        )
        .await?;
        for invoker in supervisor.capability_invokers() {
            builder = builder.register_invoker(invoker);
        }
        log::info!("loaded plugin '{}'", manifest.name);
        supervisors.push(supervisor);
    }

    builder.build().map(|router| (router, supervisors))
}

#[cfg(test)]
mod browser_bootstrap_tests {
    use super::{
        api_key_preview, build_cors_layer, inject_browser_bootstrap_html, secure_token_eq,
        serve_frontend_build, session_messages, AppState, FrontendBuild, AUTH_HEADER_NAME,
        SESSION_CURSOR_HEADER_NAME,
    };
    use std::ffi::OsString;
    use std::sync::Arc;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    use astrcode_core::ToolRegistry;
    use astrcode_runtime::RuntimeService;
    use axum::body::{to_bytes, Body};
    use axum::extract::State;
    use axum::http::{Method, Request, StatusCode};
    use axum::routing::get;
    use axum::Router;
    use tempfile::TempDir;
    use tower::ServiceExt;

    const APP_HOME_OVERRIDE_ENV: &str = "ASTRCODE_HOME_DIR";

    #[test]
    fn injects_browser_bootstrap_into_head() {
        let html = inject_browser_bootstrap_html(
            "<!doctype html><html><head><title>AstrCode</title></head><body><div id=\"root\"></div></body></html>",
            "http://127.0.0.1:62000",
            "browser-token",
        )
        .expect("bootstrap injection should succeed");

        assert!(html.contains("window.__ASTRCODE_BOOTSTRAP__"));
        assert!(html.contains("\"token\":\"browser-token\""));
        assert!(html.contains("\"serverOrigin\":\"http://127.0.0.1:62000\""));
        assert!(
            html.find("window.__ASTRCODE_BOOTSTRAP__")
                .expect("html should contain bootstrap script")
                < html.find("</head>").expect("html should contain head")
        );
    }

    #[tokio::test]
    async fn serves_bootstrapped_index_for_spa_routes() {
        let temp_dir = TempDir::new().expect("temp dir should be creatable");
        std::fs::write(
            temp_dir.path().join("index.html"),
            "<!doctype html><html><head><title>AstrCode</title></head><body><div id=\"root\"></div></body></html>",
        )
        .expect("index.html should be writable");
        std::fs::create_dir_all(temp_dir.path().join("assets"))
            .expect("assets dir should be creatable");
        std::fs::write(
            temp_dir.path().join("assets").join("app.js"),
            "console.log('ok');",
        )
        .expect("asset file should be writable");

        let frontend_build = FrontendBuild {
            dist_dir: temp_dir.path().to_path_buf(),
            index_html: Arc::new(
                inject_browser_bootstrap_html(
                    "<!doctype html><html><head><title>AstrCode</title></head><body><div id=\"root\"></div></body></html>",
                    "http://127.0.0.1:65000",
                    "browser-token",
                )
                .expect("bootstrap injection should succeed"),
            ),
        };
        let (state, _guard) = test_state(Some(frontend_build));

        let root = serve_frontend_build(
            State(state.clone()),
            Request::builder()
                .uri("/")
                .body(Body::empty())
                .expect("root request should be valid"),
        )
        .await;
        assert_eq!(root.status(), StatusCode::OK);
        let root_body = to_bytes(root.into_body(), usize::MAX)
            .await
            .expect("root response body should be readable");
        let root_body = String::from_utf8(root_body.to_vec()).expect("root body should be utf8");
        assert!(root_body.contains("window.__ASTRCODE_BOOTSTRAP__"));
        assert!(root_body.contains("<div id=\"root\"></div>"));

        let spa = serve_frontend_build(
            State(state.clone()),
            Request::builder()
                .uri("/projects/demo")
                .body(Body::empty())
                .expect("spa request should be valid"),
        )
        .await;
        assert_eq!(spa.status(), StatusCode::OK);
        let spa_body = to_bytes(spa.into_body(), usize::MAX)
            .await
            .expect("spa response body should be readable");
        let spa_body = String::from_utf8(spa_body.to_vec()).expect("spa body should be utf8");
        assert!(spa_body.contains("window.__ASTRCODE_BOOTSTRAP__"));

        let asset = serve_frontend_build(
            State(state.clone()),
            Request::builder()
                .uri("/assets/app.js")
                .body(Body::empty())
                .expect("asset request should be valid"),
        )
        .await;
        assert_eq!(asset.status(), StatusCode::OK);
        let asset_body = to_bytes(asset.into_body(), usize::MAX)
            .await
            .expect("asset response body should be readable");
        assert_eq!(asset_body.as_ref(), b"console.log('ok');");

        let missing_asset = serve_frontend_build(
            State(state),
            Request::builder()
                .uri("/assets/missing.js")
                .body(Body::empty())
                .expect("missing asset request should be valid"),
        )
        .await;
        assert_eq!(missing_asset.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn cors_preflight_allows_cache_control_for_sse_requests() {
        let app = Router::new()
            .route(
                "/api/sessions/demo/events",
                get(|| async { StatusCode::OK }),
            )
            .layer(build_cors_layer());

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::OPTIONS)
                    .uri("/api/sessions/demo/events")
                    .header("origin", "http://127.0.0.1:5173")
                    .header("access-control-request-method", "GET")
                    .header(
                        "access-control-request-headers",
                        "x-astrcode-token,cache-control",
                    )
                    .body(Body::empty())
                    .expect("preflight request should be valid"),
            )
            .await
            .expect("preflight response should be returned");

        assert!(response.status().is_success());
        let allowed_headers = response
            .headers()
            .get("access-control-allow-headers")
            .and_then(|value| value.to_str().ok())
            .expect("cors preflight should expose allowed headers")
            .to_ascii_lowercase();
        assert!(allowed_headers.contains(AUTH_HEADER_NAME));
        assert!(allowed_headers.contains("cache-control"));
    }

    #[tokio::test]
    async fn session_messages_exposes_cursor_header_to_cross_origin_clients() {
        let temp_dir = TempDir::new().expect("temp dir should be creatable");
        let (state, _guard) = test_state(None);
        let meta = state
            .service
            .create_session(temp_dir.path())
            .await
            .expect("session should be created");
        let app = Router::new()
            .route("/api/sessions/:id/messages", get(session_messages))
            .with_state(state)
            .layer(build_cors_layer());

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!("/api/sessions/{}/messages", meta.session_id))
                    .header("origin", "http://127.0.0.1:5173")
                    .header(AUTH_HEADER_NAME, "browser-token")
                    .body(Body::empty())
                    .expect("messages request should be valid"),
            )
            .await
            .expect("messages response should be returned");

        assert_eq!(response.status(), StatusCode::OK);
        let cursor = response
            .headers()
            .get(SESSION_CURSOR_HEADER_NAME)
            .and_then(|value| value.to_str().ok())
            .expect("messages response should include cursor header");
        assert!(!cursor.is_empty());
        let exposed_headers = response
            .headers()
            .get("access-control-expose-headers")
            .and_then(|value| value.to_str().ok())
            .expect("cross-origin response should expose cursor header")
            .to_ascii_lowercase();
        assert!(exposed_headers.contains(SESSION_CURSOR_HEADER_NAME));
    }

    #[tokio::test]
    async fn session_messages_requires_authentication() {
        let temp_dir = TempDir::new().expect("temp dir should be creatable");
        let (state, _guard) = test_state(None);
        let meta = state
            .service
            .create_session(temp_dir.path())
            .await
            .expect("session should be created");
        let app = Router::new()
            .route("/api/sessions/:id/messages", get(session_messages))
            .with_state(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!("/api/sessions/{}/messages", meta.session_id))
                    .body(Body::empty())
                    .expect("messages request should be valid"),
            )
            .await
            .expect("messages response should be returned");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn session_messages_returns_not_found_for_unknown_session() {
        let (state, _guard) = test_state(None);
        let app = Router::new()
            .route("/api/sessions/:id/messages", get(session_messages))
            .with_state(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/sessions/missing-session/messages")
                    .header(AUTH_HEADER_NAME, "browser-token")
                    .body(Body::empty())
                    .expect("messages request should be valid"),
            )
            .await
            .expect("messages response should be returned");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn secure_token_eq_requires_exact_match() {
        assert!(secure_token_eq("browser-token", "browser-token"));
        assert!(!secure_token_eq("browser-token", "browser-token-x"));
        assert!(!secure_token_eq("browser-token", "browser-tokem"));
    }

    #[test]
    fn api_key_preview_supports_explicit_env_and_literal_prefixes() {
        assert_eq!(
            api_key_preview(Some("env:DEEPSEEK_API_KEY")),
            "环境变量: DEEPSEEK_API_KEY"
        );
        assert_eq!(api_key_preview(Some("literal:ABCD1234")), "****1234");
    }

    fn test_state(frontend_build: Option<FrontendBuild>) -> (AppState, ServerTestEnvGuard) {
        let guard = ServerTestEnvGuard::new();
        let registry = ToolRegistry::builder().build();
        let service = RuntimeService::new(registry).expect("runtime service should initialize");
        (
            AppState {
                service: Arc::new(service),
                bootstrap_token: "browser-token".to_string(),
                frontend_build,
            },
            guard,
        )
    }

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    struct ServerTestEnvGuard {
        _lock: MutexGuard<'static, ()>,
        _temp_home: TempDir,
        previous_home_override: Option<OsString>,
    }

    impl ServerTestEnvGuard {
        fn new() -> Self {
            let lock = env_lock().lock().expect("env lock should be acquired");
            let temp_home = tempfile::tempdir().expect("tempdir should be created");
            let previous_home_override = std::env::var_os(APP_HOME_OVERRIDE_ENV);
            std::env::set_var(APP_HOME_OVERRIDE_ENV, temp_home.path());

            Self {
                _lock: lock,
                _temp_home: temp_home,
                previous_home_override,
            }
        }
    }

    impl Drop for ServerTestEnvGuard {
        fn drop(&mut self) {
            match &self.previous_home_override {
                Some(value) => std::env::set_var(APP_HOME_OVERRIDE_ENV, value),
                None => std::env::remove_var(APP_HOME_OVERRIDE_ENV),
            }
        }
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use super::workspace_root;

    const IMAGE_SUBSYSTEM_WINDOWS_GUI: u16 = 2;

    #[test]
    fn release_binary_uses_windows_gui_subsystem() {
        let status = Command::new(cargo_command())
            .args(["build", "-p", "astrcode-server", "--release"])
            .current_dir(workspace_root())
            .status()
            .expect("failed to build astrcode-server release binary");
        assert!(
            status.success(),
            "cargo build -p astrcode-server --release failed with status {status}"
        );

        let binary = workspace_root()
            .join("target")
            .join("release")
            .join("astrcode-server.exe");
        let subsystem = read_pe_subsystem(&binary);
        assert_eq!(
            subsystem,
            IMAGE_SUBSYSTEM_WINDOWS_GUI,
            "expected '{}' to use the Windows GUI subsystem so the Tauri sidecar does not spawn a terminal window",
            binary.display()
        );
    }

    fn cargo_command() -> OsString {
        std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"))
    }

    fn read_pe_subsystem(path: &Path) -> u16 {
        let bytes = fs::read(path).unwrap_or_else(|error| {
            panic!("failed to read PE binary '{}': {error}", path.display())
        });
        assert!(
            bytes.len() >= 0x40,
            "PE binary '{}' is too small",
            path.display()
        );

        let pe_offset = u32::from_le_bytes(bytes[0x3C..0x40].try_into().unwrap()) as usize;
        let optional_header_offset = pe_offset + 4 + 20;
        let subsystem_offset = optional_header_offset + 68;
        assert!(
            subsystem_offset + 2 <= bytes.len(),
            "PE binary '{}' is truncated before subsystem field",
            path.display()
        );

        u16::from_le_bytes(
            bytes[subsystem_offset..subsystem_offset + 2]
                .try_into()
                .unwrap(),
        )
    }
}
