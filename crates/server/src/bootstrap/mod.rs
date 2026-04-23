//! # 服务器引导模块
//!
//! 本模块包含 Tauri 桌面端和浏览器开发服务器引导所需的全部基础设施。
//!
//! ## 职责范围
//!
//! - **运行时组合根**（`runtime`）：组装 `App`、`AppGovernance` 等
//! - **前端静态文件服务**：加载 `frontend/dist/` 构建产物并注入 bootstrap token
//! - **运行信息管理**：写入/清理 `~/.astrcode/run.json`，供浏览器桥接和诊断读取
//! - **浏览器引导 token 注入**：将 `window.__ASTRCODE_BOOTSTRAP__` 嵌入 HTML
//! - **CORS 配置**：开发模式下允许 Vite dev server (5173) 跨域访问
//! - **Token 生成**：32 字节随机 hex token，bootstrap token 有效期 24 小时
//!
//! ## 启动流程
//!
//! 这些功能在启动时按顺序调用：
//! 1. 引导运行时 → 2. 生成 bootstrap token → 3. 写 `run.json` → 4. 配置 CORS →
//! 5. 加载前端构建产物 → 6. 注入 token 到 HTML → 7. 挂载路由
//!
//! `run.json` 不再承担桌面端单实例发现职责；桌面端进程间协调改走宿主 IPC。
//! 它保留的原因是浏览器开发模式仍需要一个稳定的本地文件来读取 bootstrap token。

mod capabilities;
mod composer_skills;
mod deps;
mod governance;
mod mcp;
mod plugins;
mod prompt_facts;
mod providers;
pub(crate) mod runtime;
mod runtime_coordinator;
mod watch;
use std::path::{Path as FsPath, PathBuf};

use anyhow::{Context, Result as AnyhowResult, anyhow};
use astrcode_support::hostpaths::resolve_home_dir;
use axum::{
    Json, Router,
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use rand::RngExt;
#[cfg(test)]
pub(crate) use runtime::{ServerBootstrapOptions, bootstrap_server_runtime_with_options};
pub(crate) use runtime::{ServerRuntimeHandles, bootstrap_server_runtime};
use serde::Serialize;
use tower::ServiceExt;
use tower_http::{cors::CorsLayer, services::ServeDir};

use self::deps::core::{LocalServerInfo, format_local_rfc3339};
use crate::{AUTH_HEADER_NAME, ApiError, AppState, FrontendBuild, auth::BootstrapAuth};

/// Bootstrap token 有效期（小时）。
///
/// 桌面端 sidecar 启动后 24 小时内有效，过期后需要重启 server 才能获取新 token。
pub(crate) const BOOTSTRAP_TOKEN_TTL_HOURS: i64 = 24;

/// 浏览器 bootstrap 桥接端点返回的载荷。
///
/// 始终返回真实的 server origin 和 token，避免前端再根据当前页面 origin 做推断。
#[derive(Debug, Serialize)]
pub(crate) struct BrowserBootstrapResponse {
    token: String,
    #[serde(rename = "serverOrigin")]
    server_origin: String,
}

pub(crate) struct PreparedServerLaunch {
    pub bootstrap_auth: BootstrapAuth,
    pub frontend_build: Option<FrontendBuild>,
}

/// 组装 server 启动期需要的 bootstrap 产物并写入 run info。
///
/// 为什么下沉到 bootstrap：
/// 让 `main.rs` 只关心启动流程编排，不再直接拼装 `LocalServerInfo`
/// 等底层细节，减少入口文件对基础设施细节的感知。
pub(crate) fn prepare_server_launch(
    port: u16,
    started_at: chrono::DateTime<chrono::Utc>,
) -> AnyhowResult<PreparedServerLaunch> {
    let token = random_hex_token();
    let bootstrap_expires_at_ms = bootstrap_token_expires_at_ms(started_at);
    let bootstrap_auth = BootstrapAuth::new(token, bootstrap_expires_at_ms);
    let server_origin = format!("http://127.0.0.1:{port}");
    let frontend_build = load_frontend_build(&server_origin, bootstrap_auth.token())?;
    let local_server_info = LocalServerInfo {
        port,
        token: bootstrap_auth.token().to_string(),
        pid: std::process::id(),
        started_at: format_local_rfc3339(started_at),
        expires_at_ms: bootstrap_auth.expires_at_ms(),
    };
    write_run_info(&local_server_info)?;
    println!(
        "{}",
        local_server_info
            .to_ready_line()
            .map_err(|error| anyhow!("failed to encode sidecar ready payload: {error}"))?
    );

    Ok(PreparedServerLaunch {
        bootstrap_auth,
        frontend_build,
    })
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
    headers: HeaderMap,
) -> Result<Json<BrowserBootstrapResponse>, ApiError> {
    require_allowed_run_info_origin(&headers)?;

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

    let raw = std::fs::read_to_string(&run_info_path).map_err(|e| ApiError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: e.to_string(),
    })?;

    let run_info: LocalServerInfo = serde_json::from_str(&raw).map_err(|e| ApiError {
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
        server_origin: format!("http://127.0.0.1:{}", run_info.port),
    }))
}

/// 判断给定 origin 是否在本地 bootstrap 白名单内。
///
/// Why: `run-info` 会返回明文 bootstrap token，必须把消费方限制在明确允许的
/// 本地前端 origin，避免任意网页直接读取本地 sidecar 引导凭证。
pub(crate) fn is_allowed_local_origin(origin: &str) -> bool {
    matches!(
        origin,
        "http://localhost:5173"
            | "http://127.0.0.1:5173"
            | "tauri://localhost"
            | "http://tauri.localhost"
            | "https://tauri.localhost"
    ) || origin.ends_with(".tauri.localhost")
        && (origin.starts_with("http://") || origin.starts_with("https://"))
}

fn require_allowed_run_info_origin(headers: &HeaderMap) -> Result<(), ApiError> {
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            ApiError::bad_request(
                "run-info requires an allowed Origin header from the local frontend".to_string(),
            )
        })?;

    if is_allowed_local_origin(origin) {
        Ok(())
    } else {
        Err(ApiError {
            status: StatusCode::FORBIDDEN,
            message: format!("origin '{origin}' is not allowed to access run-info"),
        })
    }
}

/// 将前端路由挂载到 Axum 路由器。
///
/// 如果有前端构建产物，使用 fallback 处理器拦截所有未匹配的路由；
/// 否则只挂载 `/` 返回服务器根路径提示。
/// 这样开发模式下 API 可用，生产模式下 SPA 路由正常工作。
pub(crate) fn attach_frontend_build(
    app: Router<AppState>,
    frontend_build: Option<FrontendBuild>,
) -> Router<AppState> {
    if frontend_build.is_some() {
        return app.fallback(serve_frontend_build);
    }

    app.route("/", get(server_root))
}

/// 加载前端构建产物。
///
/// 检查 `frontend/dist/index.html` 是否存在，如果存在则读取内容
/// 并注入 bootstrap token 脚本。返回 `None` 表示未构建前端，
/// 服务器将只提供 API 路由。
///
/// 注入的 token 用于前端 Vite dev server 与 server 进行鉴权交换。
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

/// 提供前端静态文件服务。
///
/// 处理 SPA 路由：
/// - 已知静态资源（含 `.` 的路径段）→ 直接从 `dist/` 提供
/// - 未知路径 → 返回 `index.html`（前端路由接管）
/// - 已知静态资源但 404 → 返回 404（不 fallback 到 index.html）
///
/// 仅响应 GET 和 HEAD 请求，其他方法返回 404。
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

/// 解析前端 dist 目录路径。
///
/// 路径为 `<workspace_root>/frontend/dist`，
/// 由 Vite 构建产物输出位置决定。
fn frontend_dist_dir() -> PathBuf {
    workspace_root().join("frontend").join("dist")
}

/// 解析工作区根目录。
///
/// 基于 `CARGO_MANIFEST_DIR`（即 `crates/server/`）向上两级
/// 到达项目根目录。用于定位前端 dist 目录等相对路径。
pub(crate) fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(FsPath::parent)
        .expect("workspace root should exist")
        .to_path_buf()
}

/// 将 bootstrap token 注入到 HTML 的 `<head>` 中。
///
/// 在 `</head>` 前插入 `<script>window.__ASTRCODE_BOOTSTRAP__ = ...</script>`，
/// 前端通过读取该全局变量获取 server 地址和 token，
/// 然后进行鉴权交换获取长期 API token。
///
/// 如果 HTML 中没有 `</head>` 标签则返回错误，
/// 因为这通常意味着构建产物损坏或不是有效的 HTML。
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

/// 构造浏览器 index.html 响应。
///
/// 设置正确的 `Content-Type` 为 `text/html; charset=utf-8`，
/// 确保浏览器正确解析注入的 bootstrap 脚本。
fn browser_index_response(index_html: &str) -> Response {
    let mut response = index_html.to_owned().into_response();
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    response
}

/// 构建 CORS 层。
///
/// 允许的来源：
/// - `http://localhost:5173` — Vite dev server
/// - `http://127.0.0.1:5173` — Vite dev server（IP 形式）
/// - `tauri://localhost` — 桌面端显式 app scheme
/// - `http://tauri.localhost` / `https://tauri.localhost` — Tauri WebView 资源域名
/// - `http://*.tauri.localhost` / `https://*.tauri.localhost` — 部分平台/版本会为 webview 分配子域
///
/// 允许的方法：GET、POST、DELETE、OPTIONS
///
/// 允许的请求头：
/// - `x-astrcode-token` — 认证 token
/// - `content-type` — JSON 请求体
/// - `last-event-id` — SSE 断点续传
/// - `cache-control` — 缓存控制
pub(crate) fn build_cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(tower_http::cors::AllowOrigin::predicate(
            |origin: &HeaderValue, _request_parts| {
                let Ok(origin) = origin.to_str() else {
                    return false;
                };

                is_allowed_local_origin(origin)
            },
        ))
        .allow_methods([Method::GET, Method::POST, Method::DELETE, Method::OPTIONS])
        .allow_headers([
            HeaderName::from_static(AUTH_HEADER_NAME),
            HeaderName::from_static("content-type"),
            HeaderName::from_static("last-event-id"),
            HeaderName::from_static("cache-control"),
        ])
}

/// 生成 32 字节随机 hex token。
///
/// 用于 bootstrap 认证和 API 会话 token，
/// 64 字符 hex 字符串提供 256 位熵，防止暴力破解。
pub(crate) fn random_hex_token() -> String {
    let mut bytes = [0_u8; 32];
    rand::rng().fill(&mut bytes);
    bytes.iter().map(|byte| format!("{:02x}", byte)).collect()
}

/// 计算 bootstrap token 过期时间戳（毫秒）。
///
/// 基于给定的启动时间加上 `BOOTSTRAP_TOKEN_TTL_HOURS`（24 小时），
/// 用于写入 `run.json` 和验证 token 有效性。
pub(crate) fn bootstrap_token_expires_at_ms(started_at: chrono::DateTime<chrono::Utc>) -> i64 {
    (started_at + chrono::Duration::hours(BOOTSTRAP_TOKEN_TTL_HOURS)).timestamp_millis()
}

/// 写入运行信息到 `~/.astrcode/run.json`。
///
/// 包含端口、token、进程 ID、启动时间和过期时间。
/// 浏览器开发桥接和人工排障都依赖它，但桌面端单实例不再靠它做主发现。
///
/// 如果目录不存在会自动创建。写入失败会携带路径上下文信息。
pub(crate) fn write_run_info(run_info: &LocalServerInfo) -> AnyhowResult<()> {
    let path = run_info_path()?;
    write_run_info_at_path(&path, run_info)
}

#[cfg(test)]
pub(crate) fn write_run_info_in_home(
    home_dir: &FsPath,
    run_info: &LocalServerInfo,
) -> AnyhowResult<()> {
    let path = run_info_path_in_home(home_dir);
    write_run_info_at_path(&path, run_info)
}

fn write_run_info_at_path(path: &FsPath, run_info: &LocalServerInfo) -> AnyhowResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!("failed to create run info directory '{}'", parent.display())
        })?;
    }

    let json = serde_json::to_string_pretty(run_info).context("failed to serialize run info")?;
    std::fs::write(path, json)
        .with_context(|| format!("failed to write run info '{}'", path.display()))?;
    Ok(())
}

/// 清理运行信息文件。
///
/// 仅在文件存在且 PID 匹配时才删除，
/// 避免误删其他 server 实例的 `run.json`。
/// 文件不存在时静默返回 Ok，属于正常关闭流程。
pub(crate) fn clear_run_info(expected_pid: u32) -> AnyhowResult<()> {
    let path = run_info_path()?;
    clear_run_info_at_path(&path, expected_pid)
}

#[cfg(test)]
pub(crate) fn clear_run_info_in_home(home_dir: &FsPath, expected_pid: u32) -> AnyhowResult<()> {
    let path = run_info_path_in_home(home_dir);
    clear_run_info_at_path(&path, expected_pid)
}

fn clear_run_info_at_path(path: &FsPath, expected_pid: u32) -> AnyhowResult<()> {
    if !path.is_file() {
        return Ok(());
    }

    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read run info '{}'", path.display()))?;
    let run_info: LocalServerInfo = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse run info '{}'", path.display()))?;
    if run_info.pid != expected_pid {
        return Ok(());
    }

    std::fs::remove_file(path)
        .with_context(|| format!("failed to remove run info '{}'", path.display()))?;
    Ok(())
}

fn run_info_path() -> AnyhowResult<PathBuf> {
    let home_dir = resolve_home_dir().map_err(|e| anyhow!("{e}"))?;
    Ok(run_info_path_in_home(home_dir.as_path()))
}

fn run_info_path_in_home(home_dir: &FsPath) -> PathBuf {
    home_dir.join(".astrcode").join("run.json")
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use astrcode_support::hostpaths::ASTRCODE_TEST_HOME_ENV;
    use axum::{
        Router,
        body::{Body, to_bytes},
        http::{Request, StatusCode, header},
        routing::get,
    };
    use tokio::sync::Mutex;
    use tower::ServiceExt;

    use super::{
        bootstrap_token_expires_at_ms, clear_run_info_in_home,
        deps::core::{LocalServerInfo, format_local_rfc3339},
        is_allowed_local_origin, run_info_path_in_home, serve_run_info, write_run_info_in_home,
    };
    use crate::{bootstrap::build_cors_layer, test_support::test_state};

    fn run_info_env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn write_run_info_persists_expiry_and_clear_run_info_removes_matching_pid() {
        let temp_home = tempfile::tempdir().expect("temp home should be created");
        let expires_at_ms = bootstrap_token_expires_at_ms(chrono::Utc::now());
        write_run_info_in_home(
            temp_home.path(),
            &LocalServerInfo {
                port: 62000,
                token: "bootstrap-token".to_string(),
                pid: std::process::id(),
                started_at: format_local_rfc3339(chrono::Utc::now()),
                expires_at_ms,
            },
        )
        .expect("run info should be written");

        let path = run_info_path_in_home(temp_home.path());
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

        clear_run_info_in_home(temp_home.path(), std::process::id())
            .expect("matching pid should clear run info");
        assert!(
            !path.exists(),
            "graceful shutdown should remove the bootstrap token file for the active server pid"
        );
    }

    #[test]
    fn clear_run_info_keeps_files_for_other_server_pids() {
        let temp_home = tempfile::tempdir().expect("temp home should be created");
        let path = run_info_path_in_home(temp_home.path());
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

        clear_run_info_in_home(temp_home.path(), std::process::id())
            .expect("non-matching pid should be ignored");
        assert!(
            path.exists(),
            "cleanup must not delete a newer server's run info"
        );
    }

    #[tokio::test]
    async fn cors_allows_tauri_desktop_origins() {
        let app = Router::new()
            .route("/", get(|| async { "ok" }))
            .layer(build_cors_layer());

        for origin in [
            "tauri://localhost",
            "http://tauri.localhost",
            "https://tauri.localhost",
            "https://main.tauri.localhost",
        ] {
            let response = app
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .method(axum::http::Method::OPTIONS)
                        .uri("/")
                        .header(axum::http::header::ORIGIN, origin)
                        .header(axum::http::header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
                        .body(axum::body::Body::empty())
                        .expect("preflight request should build"),
                )
                .await
                .expect("preflight should succeed");

            assert_eq!(
                response
                    .headers()
                    .get(axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN)
                    .and_then(|value| value.to_str().ok()),
                Some(origin),
                "expected desktop origin '{origin}' to be allowed"
            );
        }
    }

    #[tokio::test]
    async fn cors_rejects_unknown_origins() {
        let app = Router::new()
            .route("/", get(|| async { "ok" }))
            .layer(build_cors_layer());

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method(axum::http::Method::OPTIONS)
                    .uri("/")
                    .header(axum::http::header::ORIGIN, "https://evil.example.com")
                    .header(axum::http::header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
                    .body(axum::body::Body::empty())
                    .expect("preflight request should build"),
            )
            .await
            .expect("preflight should succeed");

        assert!(
            response
                .headers()
                .get(axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .is_none(),
            "unexpected origin should not be echoed back by CORS"
        );
    }

    #[test]
    fn allowed_local_origin_matches_browser_and_tauri_hosts() {
        assert!(is_allowed_local_origin("http://localhost:5173"));
        assert!(is_allowed_local_origin("https://main.tauri.localhost"));
        assert!(!is_allowed_local_origin("https://evil.example.com"));
    }

    #[tokio::test]
    async fn serve_run_info_requires_allowed_origin() {
        let _env_guard = run_info_env_lock().lock().await;
        let (state, guard) = test_state(None).await;
        let expires_at_ms = bootstrap_token_expires_at_ms(chrono::Utc::now());
        write_run_info_in_home(
            guard.home_dir(),
            &LocalServerInfo {
                port: 62000,
                token: "bootstrap-token".to_string(),
                pid: std::process::id(),
                started_at: format_local_rfc3339(chrono::Utc::now()),
                expires_at_ms,
            },
        )
        .expect("run info should be written");
        std::env::set_var(ASTRCODE_TEST_HOME_ENV, guard.home_dir());

        let app = Router::new()
            .route("/__astrcode__/run-info", get(serve_run_info))
            .with_state(state);

        let forbidden = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/__astrcode__/run-info")
                    .header(header::ORIGIN, "https://evil.example.com")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

        let missing = app
            .oneshot(
                Request::builder()
                    .uri("/__astrcode__/run-info")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(missing.status(), StatusCode::BAD_REQUEST);
        std::env::remove_var(ASTRCODE_TEST_HOME_ENV);
    }

    #[tokio::test]
    async fn serve_run_info_returns_payload_for_allowed_origin() {
        let _env_guard = run_info_env_lock().lock().await;
        let (state, guard) = test_state(None).await;
        let expires_at_ms = bootstrap_token_expires_at_ms(chrono::Utc::now());
        write_run_info_in_home(
            guard.home_dir(),
            &LocalServerInfo {
                port: 62000,
                token: "bootstrap-token".to_string(),
                pid: std::process::id(),
                started_at: format_local_rfc3339(chrono::Utc::now()),
                expires_at_ms,
            },
        )
        .expect("run info should be written");
        std::env::set_var(ASTRCODE_TEST_HOME_ENV, guard.home_dir());

        let app = Router::new()
            .route("/__astrcode__/run-info", get(serve_run_info))
            .with_state(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/__astrcode__/run-info")
                    .header(header::ORIGIN, "http://127.0.0.1:5173")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        let payload: serde_json::Value = serde_json::from_slice(
            &to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("body should be readable"),
        )
        .expect("payload should deserialize");
        assert_eq!(payload["token"], "bootstrap-token");
        assert_eq!(payload["serverOrigin"], "http://127.0.0.1:62000");
        std::env::remove_var(ASTRCODE_TEST_HOME_ENV);
    }

    #[tokio::test]
    async fn serve_run_info_returns_service_unavailable_for_expired_bootstrap_token() {
        let _env_guard = run_info_env_lock().lock().await;
        let (state, guard) = test_state(None).await;
        write_run_info_in_home(
            guard.home_dir(),
            &LocalServerInfo {
                port: 62000,
                token: "expired-bootstrap-token".to_string(),
                pid: std::process::id(),
                started_at: format_local_rfc3339(chrono::Utc::now()),
                expires_at_ms: 1,
            },
        )
        .expect("run info should be written");
        std::env::set_var(ASTRCODE_TEST_HOME_ENV, guard.home_dir());

        let app = Router::new()
            .route("/__astrcode__/run-info", get(serve_run_info))
            .with_state(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/__astrcode__/run-info")
                    .header(header::ORIGIN, "http://127.0.0.1:5173")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let payload: serde_json::Value = serde_json::from_slice(
            &to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("body should be readable"),
        )
        .expect("payload should deserialize");
        assert_eq!(
            payload["error"],
            "bootstrap token has expired; server may need restart"
        );
        std::env::remove_var(ASTRCODE_TEST_HOME_ENV);
    }
}
