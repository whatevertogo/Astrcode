use std::path::{Path as FsPath, PathBuf};

use anyhow::{anyhow, Context, Result as AnyhowResult};
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderName, HeaderValue, Method, StatusCode};
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use rand::RngCore;
use serde::Serialize;
use tower::ServiceExt;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;

use crate::{AppState, FrontendBuild, AUTH_HEADER_NAME, SESSION_CURSOR_HEADER_NAME};

pub(crate) const APP_HOME_OVERRIDE_ENV: &str = "ASTRCODE_HOME_DIR";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RunInfo {
    port: u16,
    token: String,
    pid: u32,
    started_at: String,
}

async fn server_root() -> &'static str {
    "AstrCode server is running. API endpoints are available under /api. Build the frontend with `cd frontend && npm run build` or use the Vite dev server on http://127.0.0.1:5173/."
}

pub(crate) fn attach_frontend_build(
    app: Router<AppState>,
    frontend_build: Option<FrontendBuild>,
) -> Router<AppState> {
    if frontend_build.is_some() {
        return app.fallback(serve_frontend_build);
    }

    app.route("/", get(server_root))
}

pub(crate) fn load_frontend_build(
    server_origin: &str,
    token: &str,
) -> AnyhowResult<Option<FrontendBuild>> {
    let dist_dir = frontend_dist_dir();
    let index_path = dist_dir.join("index.html");
    if !index_path.is_file() {
        return Ok(None);
    }

    let raw_index = std::fs::read_to_string(&index_path)
        .with_context(|| format!("failed to read frontend entry '{}'", index_path.display()))?;
    let injected_index = std::sync::Arc::new(inject_browser_bootstrap_html(
        &raw_index,
        server_origin,
        token,
    )?);
    Ok(Some(FrontendBuild {
        dist_dir,
        index_html: injected_index,
    }))
}

pub(crate) async fn serve_frontend_build(
    State(state): State<AppState>,
    request: Request<Body>,
) -> Response {
    let Some(frontend_build) = state.frontend_build else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if request.method() != Method::GET && request.method() != Method::HEAD {
        return StatusCode::NOT_FOUND.into_response();
    }

    let request_path = request.uri().path().trim_start_matches('/').to_string();
    let looks_like_asset = request_path
        .rsplit('/')
        .next()
        .map(|segment| segment.contains('.'))
        .unwrap_or(false);

    match ServeDir::new(&frontend_build.dist_dir)
        .append_index_html_on_directories(false)
        .oneshot(request)
        .await
    {
        Ok(response) if response.status() != StatusCode::NOT_FOUND => response.into_response(),
        Ok(_) if looks_like_asset => StatusCode::NOT_FOUND.into_response(),
        Ok(_) => browser_index_response(&frontend_build.index_html),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to serve frontend build: {error}"),
        )
            .into_response(),
    }
}

fn frontend_dist_dir() -> PathBuf {
    workspace_root().join("frontend").join("dist")
}

pub(crate) fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(FsPath::parent)
        .expect("workspace root should exist")
        .to_path_buf()
}

pub(crate) fn inject_browser_bootstrap_html(
    index_html: &str,
    server_origin: &str,
    token: &str,
) -> AnyhowResult<String> {
    let injection = serde_json::json!({
        "serverOrigin": server_origin,
        "token": token,
    });
    let script = format!(
        r#"<script>window.__ASTRCODE_BOOTSTRAP__ = Object.freeze({});</script>"#,
        serde_json::to_string(&injection).context("failed to serialize browser bootstrap")?
    );

    if let Some(head_index) = index_html.find("</head>") {
        let mut html = String::with_capacity(index_html.len() + script.len());
        html.push_str(&index_html[..head_index]);
        html.push_str(&script);
        html.push_str(&index_html[head_index..]);
        return Ok(html);
    }

    Err(anyhow!(
        "frontend index.html is missing </head>; cannot inject browser bootstrap"
    ))
}

fn browser_index_response(index_html: &str) -> Response {
    let mut response = index_html.to_owned().into_response();
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    response
}

pub(crate) fn build_cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin([
            "http://localhost:5173".parse().unwrap(),
            "http://127.0.0.1:5173".parse().unwrap(),
        ])
        .allow_methods([Method::GET, Method::POST, Method::DELETE, Method::OPTIONS])
        .allow_headers([
            HeaderName::from_static(AUTH_HEADER_NAME),
            HeaderName::from_static("content-type"),
            HeaderName::from_static("last-event-id"),
            HeaderName::from_static("cache-control"),
        ])
        .expose_headers([HeaderName::from_static(SESSION_CURSOR_HEADER_NAME)])
}

pub(crate) fn random_hex_token() -> String {
    let mut bytes = [0_u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{:02x}", byte)).collect()
}

pub(crate) fn write_run_info(port: u16, token: &str) -> AnyhowResult<()> {
    let path = run_info_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!("failed to create run info directory '{}'", parent.display())
        })?;
    }

    let payload = RunInfo {
        port,
        token: token.to_string(),
        pid: std::process::id(),
        started_at: chrono::Utc::now().to_rfc3339(),
    };
    let json = serde_json::to_string_pretty(&payload).context("failed to serialize run info")?;
    std::fs::write(&path, json)
        .with_context(|| format!("failed to write run info '{}'", path.display()))?;
    Ok(())
}

fn run_info_path() -> AnyhowResult<PathBuf> {
    Ok(resolve_home_dir()?.join(".astrcode").join("run.json"))
}

fn resolve_home_dir() -> AnyhowResult<PathBuf> {
    if let Some(home) = std::env::var_os(APP_HOME_OVERRIDE_ENV) {
        if !home.is_empty() {
            return Ok(PathBuf::from(home));
        }
    }

    dirs::home_dir().ok_or_else(|| anyhow!("unable to resolve home directory"))
}
