use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use astrcode_agent::{
    AgentService, PromptAccepted, ServiceError, SessionMessage, SessionReplaySource,
};
use astrcode_contracts::{
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
use axum::extract::{Path, Query, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use tower_http::cors::CorsLayer;

const APP_HOME_OVERRIDE_ENV: &str = "ASTRCODE_HOME_DIR";
const AUTH_HEADER_NAME: &str = "x-astrcode-token";

#[derive(Clone)]
struct AppState {
    service: Arc<AgentService>,
    bootstrap_token: String,
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
    write_run_info(address.port(), &token)?;
    println!(
        "Ready: http://localhost:{}/?token={}",
        address.port(),
        token
    );

    let state = AppState {
        service,
        bootstrap_token: token,
    };
    let cors = CorsLayer::new()
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
        ]);

    let app = Router::new()
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
        .route("/api/models/test", post(test_model_connection))
        .with_state(state)
        .layer(cors);

    axum::serve(listener, app)
        .await
        .context("server terminated unexpectedly")
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
            "x-session-cursor",
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
            success,
            duration_ms,
        } => SessionMessageDto::ToolCall {
            tool_call_id,
            tool_name,
            args,
            output,
            success,
            duration_ms,
        },
    }
}

fn to_sse_event(record: astrcode_agent::SessionEventRecord) -> Event {
    let payload =
        serde_json::to_string(&AgentEventEnvelope::from(record.event)).unwrap_or_else(|error| {
            serde_json::json!({
                "protocolVersion": astrcode_contracts::PROTOCOL_VERSION,
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
