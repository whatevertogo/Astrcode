// 本文件包含 Tauri 桌面端引导所需的全部基础设施：
// - 前端静态文件服务（serve_frontend_build / attach_frontend_build / load_frontend_build）
// - 运行信息文件管理（write_run_info / clear_run_info）—— Tauri 通过读取此文件发现 server 端口
// - 浏览器引导 token 注入（inject_browser_bootstrap_html）—— 将认证 token 嵌入 HTML
// - CORS 配置（build_cors_layer）—— 开发模式需要 localhost 双端口互通
// - Token 生成（random_hex_token / bootstrap_token_expires_at_ms）
// - 工作区根目录解析（workspace_root）
//
// 这些功能都与「server 如何被外部发现和认证」相关，放在一起是因为它们在启动流程中
// 按顺序调用：生成 token → 写 run_info → 配 CORS → 挂载前端 → 注入 token 到 HTML。
// 如果未来增加更多引导逻辑，可以考虑拆分为 bootstrap/ 子模块。

use std::path::{Path as FsPath, PathBuf};

use anyhow::{anyhow, Context, Result as AnyhowResult};
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderName, HeaderValue, Method, StatusCode};
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::get;
use axum::{Json, Router};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use tower::ServiceExt;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;

use crate::{ApiError, AppState, FrontendBuild, AUTH_HEADER_NAME, SESSION_CURSOR_HEADER_NAME};

/// 从 core crate 导入，避免重复定义（仅测试使用）
#[cfg(test)]
pub(crate) use astrcode_core::home::ASTRCODE_HOME_DIR_ENV as APP_HOME_OVERRIDE_ENV;
pub(crate) const BOOTSTRAP_TOKEN_TTL_HOURS: i64 = 24;

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RunInfo {
    port: u16,
    token: String,
    pid: u32,
    started_at: String,
    expires_at_ms: i64,
}

/// 浏览器 bootstrap 桥接端点返回的载荷（仅包含 token）
#[derive(Debug, Serialize)]
pub(crate) struct BrowserBootstrapResponse {
    token: String,
}

async fn server_root() -> &'static str {
    "AstrCode server is running. API endpoints are available under /api. Build the frontend with `cd frontend && npm run build` or use the Vite dev server on http://127.0.0.1:5173/."
}

/// 为浏览器开发服务器提供 bootstrap token
///
/// 前端 Vite dev server (port 5173) 通过此端点获取 server 的 bootstrap token，
/// 然后才能进行鉴权交换。
pub(crate) async fn serve_run_info(
    State(_state): State<AppState>,
) -> Result<Json<BrowserBootstrapResponse>, ApiError> {
    let run_info_path = run_info_path().map_err(|e| ApiError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: e.to_string(),
    })?;
    if !run_info_path.is_file() {
        return Err(ApiError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: "run info not available; server may be starting up or shutting down"
                .to_string(),
        });
    }

    let raw = std::fs::read_to_string(&run_info_path)
        .with_context(|| format!("failed to read run info '{}'", run_info_path.display()))
        .map_err(|e| ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: e.to_string(),
        })?;

    let run_info: RunInfo = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse run info '{}'", run_info_path.display()))
        .map_err(|e| ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: e.to_string(),
        })?;

    // 检查 token 是否过期
    if chrono::Utc::now().timestamp_millis() > run_info.expires_at_ms {
        return Err(ApiError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: "bootstrap token has expired; server may need restart".to_string(),
        });
    }

    Ok(Json(BrowserBootstrapResponse {
        token: run_info.token,
    }))
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

pub(crate) fn bootstrap_token_expires_at_ms(started_at: chrono::DateTime<chrono::Utc>) -> i64 {
    (started_at + chrono::Duration::hours(BOOTSTRAP_TOKEN_TTL_HOURS)).timestamp_millis()
}

pub(crate) fn write_run_info(port: u16, token: &str, expires_at_ms: i64) -> AnyhowResult<()> {
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
        expires_at_ms,
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
    Ok(astrcode_core::home::resolve_home_dir()
        .map_err(|e| anyhow!("{e}"))?
        .join(".astrcode")
        .join("run.json"))
}

#[cfg(test)]
mod tests {
    use crate::test_support::ServerTestEnvGuard;

    use super::{bootstrap_token_expires_at_ms, clear_run_info, run_info_path, write_run_info};

    #[test]
    fn write_run_info_persists_expiry_and_clear_run_info_removes_matching_pid() {
        let _guard = ServerTestEnvGuard::new();
        let expires_at_ms = bootstrap_token_expires_at_ms(chrono::Utc::now());
        write_run_info(62000, "bootstrap-token", expires_at_ms)
            .expect("run info should be written");

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
