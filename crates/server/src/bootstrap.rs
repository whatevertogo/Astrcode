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
use serde::{Deserialize, Serialize};
use tower::ServiceExt;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;

use crate::{AppState, FrontendBuild, AUTH_HEADER_NAME, SESSION_CURSOR_HEADER_NAME};

pub(crate) const APP_HOME_OVERRIDE_ENV: &str = "ASTRCODE_HOME_DIR";
const RUN_INFO_TTL_HOURS: i64 = 24;

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RunInfo {
    port: u16,
    token: String,
    pid: u32,
    started_at: String,
    expires_at_ms: i64,
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

    let started_at = chrono::Utc::now();
    let payload = RunInfo {
        port,
        token: token.to_string(),
        pid: std::process::id(),
        started_at: started_at.to_rfc3339(),
        expires_at_ms: (started_at + chrono::Duration::hours(RUN_INFO_TTL_HOURS))
            .timestamp_millis(),
    };
    let json = serde_json::to_string_pretty(&payload).context("failed to serialize run info")?;
    std::fs::write(&path, json)
        .with_context(|| format!("failed to write run info '{}'", path.display()))?;
    Ok(())
}

pub(crate) fn clear_run_info(expected_pid: u32) -> AnyhowResult<()> {
    let path = run_info_path()?;
    if !path.is_file() {
        return Ok(());
    }

    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read run info '{}'", path.display()))?;
    let run_info: RunInfo = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse run info '{}'", path.display()))?;
    if run_info.pid != expected_pid {
        return Ok(());
    }

    std::fs::remove_file(&path)
        .with_context(|| format!("failed to remove run info '{}'", path.display()))?;
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

#[cfg(test)]
mod tests {
    use crate::test_support::ServerTestEnvGuard;

    use super::{clear_run_info, run_info_path, write_run_info};

    #[test]
    fn write_run_info_persists_expiry_and_clear_run_info_removes_matching_pid() {
        let _guard = ServerTestEnvGuard::new();
        write_run_info(62000, "bootstrap-token").expect("run info should be written");

        let path = run_info_path().expect("run info path should resolve");
        let payload = std::fs::read_to_string(&path).expect("run info should be readable");
        let json: serde_json::Value =
            serde_json::from_str(&payload).expect("run info should be valid json");
        assert_eq!(
            json.get("port").and_then(|value| value.as_u64()),
            Some(62000)
        );
        assert_eq!(
            json.get("token").and_then(|value| value.as_str()),
            Some("bootstrap-token")
        );
        assert!(
            json.get("expiresAtMs")
                .and_then(|value| value.as_i64())
                .is_some(),
            "run info should carry an expiry for the bootstrap token"
        );

        clear_run_info(std::process::id()).expect("matching pid should clear run info");
        assert!(
            !path.exists(),
            "graceful shutdown should remove the bootstrap token file for the active server pid"
        );
    }

    #[test]
    fn clear_run_info_keeps_files_for_other_server_pids() {
        let _guard = ServerTestEnvGuard::new();
        let path = run_info_path().expect("run info path should resolve");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("run info parent should exist");
        }
        std::fs::write(
            &path,
            serde_json::json!({
                "port": 62000,
                "token": "bootstrap-token",
                "pid": std::process::id() + 1,
                "startedAt": "2026-03-25T00:00:00Z",
                "expiresAtMs": 9_999_999_999_999_i64
            })
            .to_string(),
        )
        .expect("run info fixture should be written");

        clear_run_info(std::process::id()).expect("non-matching pid should be ignored");
        assert!(
            path.exists(),
            "cleanup must not delete a newer server's run info"
        );
    }
}
