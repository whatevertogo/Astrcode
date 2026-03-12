#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod dto;

use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use astrcode_agent::{
    AgentService, PromptAccepted, ServiceError, SessionMessage, SessionReplaySource,
};
use crate::dto::{
    AgentEventEnvelope, AuthExchangeRequest, AuthExchangeResponse, ConfigView,
    CreateSessionRequest, CurrentModelInfoDto, DeleteProjectResultDto, ModelOptionDto, ProfileView,
    PromptAcceptedResponse, PromptRequest, SaveActiveSelectionRequest, SessionListItem,
    SessionMessageDto, TestConnectionRequest, TestResultDto,
};
use astrcode_tools::tools::{
    edit_file::EditFileTool, find_files::FindFilesTool, grep::GrepTool, list_dir::ListDirTool,
    read_file::ReadFileTool, shell::ShellTool, write_file::WriteFileTool,
};
use async_stream::stream;
use axum::body::Body;
use axum::extract::{Path, Query, Request, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use tower::ServiceExt;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;

const APP_HOME_OVERRIDE_ENV: &str = "ASTRCODE_HOME_DIR";
const AUTH_HEADER_NAME: &str = "x-astrcode-token";
const SESSION_CURSOR_HEADER_NAME: &str = "x-session-cursor";

#[derive(Clone)]
struct AppState {
    service: Arc<AgentService>,
    bootstrap_token: String,
    frontend_build: Option<FrontendBuild>,
}

#[derive(Clone)]
struct FrontendBuild {
    dist_dir: PathBuf,
    index_html: Arc<String>,
}

#[derive(Debug, Serialize)]
struct ErrorPayload {
    error: String,
}

#[derive(Debug)]
struct ApiError {
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteProjectQuery {
    working_dir: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionEventsQuery {
    after_event_id: Option<String>,
    token: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RunInfo {
    port: u16,
    token: String,
    pid: u32,
    started_at: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let registry = astrcode_agent::ToolRegistry::builder()
        .register(Box::new(ShellTool::default()))
        .register(Box::new(ListDirTool::default()))
        .register(Box::new(ReadFileTool::default()))
        .register(Box::new(WriteFileTool::default()))
        .register(Box::new(EditFileTool::default()))
        .register(Box::new(FindFilesTool::default()))
        .register(Box::new(GrepTool::default()))
        .build();
    let service = Arc::new(AgentService::new(registry)?);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .context("failed to bind server listener")?;
    let address: SocketAddr = listener
        .local_addr()
        .context("failed to resolve server listener address")?;
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

    let app = Router::<AppState>::new()
        .route("/api/auth/exchange", post(exchange_auth))
        .route("/api/sessions", post(create_session).get(list_sessions))
        .route("/api/sessions/:id/messages", get(session_messages))
        .route("/api/sessions/:id/prompts", post(submit_prompt))
        .route("/api/sessions/:id/interrupt", post(interrupt_session))
        .route("/api/sessions/:id/events", get(session_events))
        .route("/api/sessions/:id", delete(delete_session))
        .route("/api/projects", delete(delete_project))
        .route("/api/config", get(get_config))
        .route("/api/config/active-selection", post(save_active_selection))
        .route("/api/models/current", get(get_current_model))
        .route("/api/models", get(list_models))
        .route("/api/models/test", post(test_model_connection));
    let app = attach_frontend_build(app, frontend_build);
    let app = app.with_state(state).layer(build_cors_layer());

    axum::serve(listener, app)
        .await
        .context("server terminated unexpectedly")
}

async fn server_root() -> &'static str {
    "AstrCode server is running. API endpoints are available under /api. Build the frontend with `cd frontend && npm run build` or use the Vite dev server on http://127.0.0.1:5173/."
}

fn attach_frontend_build(
    app: Router<AppState>,
    frontend_build: Option<FrontendBuild>,
) -> Router<AppState> {
    if frontend_build.is_some() {
        return app.fallback(serve_frontend_build);
    }

    app.route("/", get(server_root))
}

fn load_frontend_build(server_origin: &str, token: &str) -> Result<Option<FrontendBuild>> {
    let dist_dir = frontend_dist_dir();
    let index_path = dist_dir.join("index.html");
    if !index_path.is_file() {
        return Ok(None);
    }

    let raw_index = std::fs::read_to_string(&index_path)
        .with_context(|| format!("failed to read frontend entry '{}'", index_path.display()))?;
    let injected_index = Arc::new(inject_browser_bootstrap_html(
        &raw_index,
        server_origin,
        token,
    )?);
    Ok(Some(FrontendBuild {
        dist_dir,
        index_html: injected_index,
    }))
}

async fn serve_frontend_build(State(state): State<AppState>, request: Request<Body>) -> Response {
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

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(FsPath::parent)
        .expect("workspace root should exist")
        .to_path_buf()
}

fn inject_browser_bootstrap_html(
    index_html: &str,
    server_origin: &str,
    token: &str,
) -> Result<String> {
    let bootstrap = serde_json::json!({
        "token": token,
        "isDesktopHost": false,
        "serverOrigin": server_origin,
    });
    let script = format!(
        "<script>window.__ASTRCODE_BOOTSTRAP__ = {};</script>",
        serde_json::to_string(&bootstrap)?
    );

    if let Some(head_index) = index_html.find("<head>") {
        let insert_at = head_index + "<head>".len();
        let mut html = String::with_capacity(index_html.len() + script.len());
        html.push_str(&index_html[..insert_at]);
        html.push_str(&script);
        html.push_str(&index_html[insert_at..]);
        return Ok(html);
    }

    if let Some(head_index) = index_html.find("</head>") {
        let mut html = String::with_capacity(index_html.len() + script.len());
        html.push_str(&index_html[..head_index]);
        html.push_str(&script);
        html.push_str(&index_html[head_index..]);
        return Ok(html);
    }

    Ok(format!("{script}{index_html}"))
}

fn browser_index_response(index_html: &str) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from(index_html.to_owned()))
        .expect("browser index response should be valid")
}

fn build_cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin([
            HeaderValue::from_static("http://127.0.0.1:5173"),
            HeaderValue::from_static("http://localhost:5173"),
            HeaderValue::from_static("tauri://localhost"),
            HeaderValue::from_static("http://tauri.localhost"),
            HeaderValue::from_static("https://tauri.localhost"),
        ])
        .allow_methods([Method::GET, Method::POST, Method::DELETE])
        .allow_headers([
            CONTENT_TYPE,
            HeaderName::from_static(AUTH_HEADER_NAME),
            HeaderName::from_static("last-event-id"),
            HeaderName::from_static("cache-control"),
        ])
        .expose_headers([HeaderName::from_static(SESSION_CURSOR_HEADER_NAME)])
}

async fn exchange_auth(
    State(state): State<AppState>,
    Json(request): Json<AuthExchangeRequest>,
) -> Result<Json<AuthExchangeResponse>, ApiError> {
    if request.token != state.bootstrap_token {
        return Err(ApiError::unauthorized());
    }

    Ok(Json(AuthExchangeResponse { ok: true }))
}

async fn create_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateSessionRequest>,
) -> Result<Json<SessionListItem>, ApiError> {
    require_auth(&state, &headers, None)?;
    let meta = state
        .service
        .create_session(request.working_dir)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(to_session_list_item(meta)))
}

async fn list_sessions(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<SessionListItem>>, ApiError> {
    require_auth(&state, &headers, None)?;
    let sessions = state
        .service
        .list_sessions_with_meta()
        .await
        .map_err(ApiError::from)?
        .into_iter()
        .map(to_session_list_item)
        .collect();
    Ok(Json(sessions))
}

async fn session_messages(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<Response, ApiError> {
    require_auth(&state, &headers, None)?;
    let (messages, cursor) = state
        .service
        .load_session_snapshot(&session_id)
        .map_err(ApiError::from)?;
    let payload = messages
        .into_iter()
        .map(to_session_message_dto)
        .collect::<Vec<_>>();

    let mut response = Json(payload).into_response();
    if let Some(cursor) = cursor {
        response.headers_mut().insert(
            SESSION_CURSOR_HEADER_NAME,
            cursor
                .parse::<axum::http::HeaderValue>()
                .map_err(|error| ApiError {
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    message: error.to_string(),
                })?,
        );
    }
    Ok(response)
}

async fn submit_prompt(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Json(request): Json<PromptRequest>,
) -> Result<(StatusCode, Json<PromptAcceptedResponse>), ApiError> {
    require_auth(&state, &headers, None)?;
    let accepted: PromptAccepted = state
        .service
        .submit_prompt(&session_id, request.text)
        .await
        .map_err(ApiError::from)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(PromptAcceptedResponse {
            turn_id: accepted.turn_id,
        }),
    ))
}

async fn interrupt_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    require_auth(&state, &headers, None)?;
    state
        .service
        .interrupt(&session_id)
        .await
        .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    require_auth(&state, &headers, None)?;
    state
        .service
        .delete_session(&session_id)
        .await
        .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<DeleteProjectQuery>,
) -> Result<Json<DeleteProjectResultDto>, ApiError> {
    require_auth(&state, &headers, None)?;
    let result = state
        .service
        .delete_project(&query.working_dir)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(DeleteProjectResultDto {
        success_count: result.success_count,
        failed_session_ids: result.failed_session_ids,
    }))
}

async fn session_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Query(query): Query<SessionEventsQuery>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    require_auth(&state, &headers, query.token.as_deref())?;
    let last_event_id = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string())
        .or(query.after_event_id);
    let mut replay = state
        .service
        .replay(&session_id, last_event_id.as_deref())
        .map_err(ApiError::from)?;
    let mut last_sent = last_event_id.as_deref().and_then(parse_event_id);

    let event_stream = stream! {
        for record in replay.history {
            if let Some(id) = parse_event_id(&record.event_id) {
                last_sent = Some(id);
            }
            yield Ok::<Event, Infallible>(to_sse_event(record));
        }

        loop {
            match replay.receiver.recv().await {
                Ok(record) => {
                    let Some(current_id) = parse_event_id(&record.event_id) else {
                        continue;
                    };
                    if let Some(last_id) = last_sent {
                        if current_id <= last_id {
                            continue;
                        }
                    }
                    last_sent = Some(current_id);
                    yield Ok::<Event, Infallible>(to_sse_event(record));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };

    Ok(Sse::new(event_stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keepalive"),
    ))
}

async fn get_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ConfigView>, ApiError> {
    require_auth(&state, &headers, None)?;
    let config = state.service.get_config().await;
    let config_path = state
        .service
        .current_config_path()
        .await
        .map_err(ApiError::from)?
        .to_string_lossy()
        .to_string();
    Ok(Json(build_config_view(&config, config_path)?))
}

async fn save_active_selection(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SaveActiveSelectionRequest>,
) -> Result<StatusCode, ApiError> {
    require_auth(&state, &headers, None)?;
    state
        .service
        .save_active_selection(request.active_profile, request.active_model)
        .await
        .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_current_model(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<CurrentModelInfoDto>, ApiError> {
    require_auth(&state, &headers, None)?;
    let config = state.service.get_config().await;
    Ok(Json(resolve_current_model(&config)?))
}

async fn list_models(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ModelOptionDto>>, ApiError> {
    require_auth(&state, &headers, None)?;
    let config = state.service.get_config().await;
    Ok(Json(list_model_options(&config)))
}

async fn test_model_connection(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<TestConnectionRequest>,
) -> Result<Json<TestResultDto>, ApiError> {
    require_auth(&state, &headers, None)?;
    let result = state
        .service
        .test_connection(&request.profile_name, &request.model)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(TestResultDto {
        success: result.success,
        provider: result.provider,
        model: result.model,
        error: result.error,
    }))
}

fn require_auth(
    state: &AppState,
    headers: &HeaderMap,
    query_token: Option<&str>,
) -> Result<(), ApiError> {
    let header_token = headers
        .get(AUTH_HEADER_NAME)
        .and_then(|value| value.to_str().ok());
    let authorized = header_token
        .or(query_token)
        .map(|token| token == state.bootstrap_token)
        .unwrap_or(false);
    if authorized {
        Ok(())
    } else {
        Err(ApiError::unauthorized())
    }
}

fn to_session_list_item(meta: astrcode_agent::SessionMeta) -> SessionListItem {
    SessionListItem {
        session_id: meta.session_id,
        working_dir: meta.working_dir,
        display_name: meta.display_name,
        title: meta.title,
        created_at: meta.created_at.to_rfc3339(),
        updated_at: meta.updated_at.to_rfc3339(),
        phase: meta.phase,
    }
}

fn to_session_message_dto(message: SessionMessage) -> SessionMessageDto {
    match message {
        SessionMessage::User { content, timestamp } => {
            SessionMessageDto::User { content, timestamp }
        }
        SessionMessage::Assistant { content, timestamp } => {
            SessionMessageDto::Assistant { content, timestamp }
        }
        SessionMessage::ToolCall {
            tool_call_id,
            tool_name,
            args,
            output,
            ok,
            duration_ms,
        } => SessionMessageDto::ToolCall {
            tool_call_id,
            tool_name,
            args,
            output,
            ok,
            duration_ms,
        },
    }
}

fn to_sse_event(record: astrcode_agent::SessionEventRecord) -> Event {
    let payload =
        serde_json::to_string(&AgentEventEnvelope::from(record.event)).unwrap_or_else(|error| {
            serde_json::json!({
                "protocolVersion": crate::dto::PROTOCOL_VERSION,
                "event": "error",
                "data": {
                    "turnId": null,
                    "code": "serialization_error",
                    "message": error.to_string()
                }
            })
            .to_string()
        });
    Event::default().id(record.event_id).data(payload)
}

fn parse_event_id(raw: &str) -> Option<(u64, u32)> {
    let (storage_seq, subindex) = raw.split_once('.')?;
    Some((storage_seq.parse().ok()?, subindex.parse().ok()?))
}

fn build_config_view(
    config: &astrcode_agent::Config,
    config_path: String,
) -> Result<ConfigView, ApiError> {
    if config.profiles.is_empty() {
        return Ok(ConfigView {
            config_path,
            active_profile: String::new(),
            active_model: String::new(),
            profiles: Vec::new(),
            warning: Some("no profiles configured".to_string()),
        });
    }

    let profiles = config
        .profiles
        .iter()
        .map(|profile| ProfileView {
            name: profile.name.clone(),
            base_url: profile.base_url.clone(),
            api_key_preview: api_key_preview(profile.api_key.as_deref()),
            models: profile.models.clone(),
        })
        .collect::<Vec<_>>();

    let (active_profile, active_model, warning) = resolve_active_selection(
        &config.active_profile,
        &config.active_model,
        &config.profiles,
    )?;

    Ok(ConfigView {
        config_path,
        active_profile,
        active_model,
        profiles,
        warning,
    })
}

fn resolve_current_model(config: &astrcode_agent::Config) -> Result<CurrentModelInfoDto, ApiError> {
    let profile = config
        .profiles
        .iter()
        .find(|profile| profile.name == config.active_profile)
        .or_else(|| config.profiles.first())
        .ok_or_else(|| ApiError {
            status: StatusCode::BAD_REQUEST,
            message: "no profiles configured".to_string(),
        })?;

    let model = if profile
        .models
        .iter()
        .any(|item| item == &config.active_model)
    {
        config.active_model.clone()
    } else {
        profile.models.first().cloned().ok_or_else(|| ApiError {
            status: StatusCode::BAD_REQUEST,
            message: format!("profile '{}' has no models", profile.name),
        })?
    };

    Ok(CurrentModelInfoDto {
        profile_name: profile.name.clone(),
        model,
        provider_kind: profile.provider_kind.clone(),
    })
}

fn list_model_options(config: &astrcode_agent::Config) -> Vec<ModelOptionDto> {
    config
        .profiles
        .iter()
        .flat_map(|profile| {
            profile.models.iter().map(|model| ModelOptionDto {
                profile_name: profile.name.clone(),
                model: model.clone(),
                provider_kind: profile.provider_kind.clone(),
            })
        })
        .collect()
}

fn resolve_active_selection(
    active_profile: &str,
    active_model: &str,
    profiles: &[astrcode_agent::Profile],
) -> Result<(String, String, Option<String>), ApiError> {
    let fallback_profile = profiles.first().ok_or_else(|| ApiError {
        status: StatusCode::BAD_REQUEST,
        message: "no profiles configured".to_string(),
    })?;

    let selected_profile = profiles
        .iter()
        .find(|profile| profile.name == active_profile)
        .unwrap_or(fallback_profile);

    if selected_profile.models.is_empty() {
        return Err(ApiError {
            status: StatusCode::BAD_REQUEST,
            message: format!("profile '{}' has no models", selected_profile.name),
        });
    }

    if selected_profile.name != active_profile {
        return Ok((
            selected_profile.name.clone(),
            selected_profile.models[0].clone(),
            Some(format!(
                "配置中的 Profile 不存在，已自动选择 {}",
                selected_profile.name
            )),
        ));
    }

    if let Some(model) = selected_profile
        .models
        .iter()
        .find(|model| *model == active_model)
    {
        return Ok((selected_profile.name.clone(), model.clone(), None));
    }

    Ok((
        selected_profile.name.clone(),
        selected_profile.models[0].clone(),
        Some(format!(
            "配置中的 {} 在当前 Profile 下不存在，已自动选择 {}",
            active_model, selected_profile.models[0]
        )),
    ))
}

fn api_key_preview(api_key: Option<&str>) -> String {
    match api_key.map(str::trim) {
        None => "未配置".to_string(),
        Some("") => "未配置".to_string(),
        Some(value) if is_env_var_name(value) => format!("环境变量: {}", value),
        Some(value) if value.chars().count() > 4 => {
            let suffix = value
                .chars()
                .rev()
                .take(4)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<String>();
            format!("****{}", suffix)
        }
        Some(_) => "****".to_string(),
    }
}

fn is_env_var_name(value: &str) -> bool {
    value
        .chars()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && value.contains('_')
}

fn random_hex_token() -> String {
    let mut bytes = [0_u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{:02x}", byte)).collect()
}

fn write_run_info(port: u16, token: &str) -> Result<()> {
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

fn run_info_path() -> Result<PathBuf> {
    Ok(resolve_home_dir()?.join(".astrcode").join("run.json"))
}

fn resolve_home_dir() -> Result<PathBuf> {
    if let Some(home) = std::env::var_os(APP_HOME_OVERRIDE_ENV) {
        if !home.is_empty() {
            return Ok(PathBuf::from(home));
        }
    }

    dirs::home_dir().ok_or_else(|| anyhow!("unable to resolve home directory"))
}

#[cfg(test)]
mod browser_bootstrap_tests {
    use super::{
        build_cors_layer, inject_browser_bootstrap_html, serve_frontend_build, session_messages,
        AppState, FrontendBuild, AUTH_HEADER_NAME, SESSION_CURSOR_HEADER_NAME,
    };
    use std::ffi::OsString;
    use std::sync::Arc;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    use astrcode_agent::{AgentService, ToolRegistry};
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

    fn test_state(frontend_build: Option<FrontendBuild>) -> (AppState, ServerTestEnvGuard) {
        let guard = ServerTestEnvGuard::new();
        let registry = ToolRegistry::builder().build();
        let service = AgentService::new(registry).expect("agent service should initialize");
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
    use std::path::{Path, PathBuf};
    use std::process::Command;

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

    fn workspace_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root should exist")
            .to_path_buf()
    }

    fn read_pe_subsystem(path: &Path) -> u16 {
        let binary = fs::read(path)
            .unwrap_or_else(|error| panic!("failed to read '{}': {error}", path.display()));

        assert!(
            binary.len() >= 0x40,
            "binary '{}' is too small to contain a PE header",
            path.display()
        );
        assert_eq!(
            &binary[..2],
            b"MZ",
            "binary '{}' is missing the DOS header signature",
            path.display()
        );

        let pe_offset = u32::from_le_bytes(
            binary[0x3c..0x40]
                .try_into()
                .expect("DOS header should expose PE offset"),
        ) as usize;
        assert!(
            binary.len() >= pe_offset + 24 + 70,
            "binary '{}' is too small to contain the PE optional header",
            path.display()
        );
        assert_eq!(
            &binary[pe_offset..pe_offset + 4],
            b"PE\0\0",
            "binary '{}' is missing the PE signature",
            path.display()
        );

        let subsystem_offset = pe_offset + 24 + 68;
        u16::from_le_bytes(
            binary[subsystem_offset..subsystem_offset + 2]
                .try_into()
                .expect("PE optional header should expose subsystem field"),
        )
    }
}
