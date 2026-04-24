//! # Astrcode HTTP 服务器
//!
//! 本 crate 是 Astrcode 的 HTTP/SSE 服务器入口，负责：
//!
//! - **API 路由**: 暴露 REST API 和 SSE 端点，所有业务逻辑的唯一入口
//! - **认证**: Bootstrap token 验证和 API 会话管理
//! - **静态资源**: 托管前端构建产物（生产模式）或提供 API-only 模式（开发模式）
//! - **优雅关闭**: 处理 Ctrl+C 和 SIGTERM 信号，确保运行时正确清理
//!
//! ## 架构原则
//!
//! - **Server Is The Truth**: 所有会话、配置、模型、事件流业务入口只通过 HTTP/SSE API
//! - 前端和 Tauri 都不得直接调用 `runtime`；Tauri 只保留窗口控制与宿主 GUI 能力
//! - `main.rs` 只保留启动与装配；新增逻辑优先落到 `routes/`、`mapper.rs`、`bootstrap.rs`

#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

#[path = "agent/mod.rs"]
mod agent;
#[path = "http/agent_api.rs"]
mod agent_api;
#[path = "agent_control_bridge.rs"]
mod agent_control_bridge;
#[path = "agent_control_registry/mod.rs"]
mod agent_control_registry;
#[cfg(test)]
#[path = "tests/agent_routes_tests.rs"]
mod agent_routes_tests;
#[path = "agent_runtime_bridge.rs"]
mod agent_runtime_bridge;
#[path = "application_error_bridge.rs"]
mod application_error_bridge;
#[path = "http/auth.rs"]
mod auth;
#[cfg(test)]
#[path = "tests/auth_routes_tests.rs"]
mod auth_routes_tests;
#[path = "bootstrap/mod.rs"]
mod bootstrap;
#[path = "capability_router.rs"]
mod capability_router;
#[path = "http/composer_catalog.rs"]
mod composer_catalog;
#[cfg(test)]
#[path = "tests/composer_routes_tests.rs"]
mod composer_routes_tests;
#[path = "config/mod.rs"]
mod config;
#[path = "config_mode_helpers.rs"]
mod config_mode_helpers;
#[cfg(test)]
#[path = "tests/config_routes_tests.rs"]
mod config_routes_tests;
#[path = "config_service_bridge.rs"]
mod config_service_bridge;
#[path = "conversation_read_model.rs"]
mod conversation_read_model;
#[path = "errors.rs"]
mod errors;
#[path = "execution/mod.rs"]
mod execution;
#[path = "governance_service.rs"]
mod governance_service;
#[path = "governance_surface/mod.rs"]
mod governance_surface;
#[path = "lifecycle/mod.rs"]
mod lifecycle;
#[path = "logging.rs"]
mod logging;
#[path = "http/mapper.rs"]
mod mapper;
#[path = "mcp/mod.rs"]
mod mcp;
#[path = "mcp_service.rs"]
mod mcp_service;
#[path = "mode/mod.rs"]
mod mode;
#[path = "mode_catalog_service.rs"]
mod mode_catalog_service;
#[path = "observability/mod.rs"]
mod observability;
#[path = "ports/mod.rs"]
mod ports;
#[path = "profile_service.rs"]
mod profile_service;
#[path = "root_execute_service.rs"]
mod root_execute_service;
#[path = "http/routes/mod.rs"]
mod routes;
#[path = "runtime_owner_bridge.rs"]
mod runtime_owner_bridge;
#[cfg(test)]
#[path = "tests/session_contract_tests.rs"]
mod session_contract_tests;
#[path = "session_identity.rs"]
mod session_identity;
#[path = "session_runtime_owner_bridge.rs"]
mod session_runtime_owner_bridge;
#[path = "session_runtime_port.rs"]
mod session_runtime_port;
#[path = "session_use_cases.rs"]
mod session_use_cases;
#[path = "http/terminal_projection.rs"]
mod terminal_projection;
#[cfg(test)]
#[path = "tests/test_support.rs"]
mod test_support;
#[path = "tool_capability_invoker.rs"]
mod tool_capability_invoker;
#[path = "view_projection.rs"]
mod view_projection;
#[path = "watch_service.rs"]
mod watch_service;

use std::{net::SocketAddr, path::PathBuf, sync::Arc};

pub(crate) use agent::AgentOrchestrationService;
use anyhow::{Result as AnyhowResult, anyhow};
use astrcode_core::{AstrError, SkillCatalog};
use astrcode_host_session::{SessionCatalog, SubAgentExecutor};
use astrcode_plugin_host::ResourceCatalog;
use axum::{
    Json, Router,
    http::StatusCode,
    response::{IntoResponse, Response},
};
pub(crate) use config::ConfigService;
pub(crate) use errors::ApplicationError;
pub(crate) use execution::{ExecutionControl, ProfileProvider, ProfileResolutionService};
pub(crate) use governance_surface::{
    GovernanceSurfaceAssembler, ResolvedGovernanceSurface, RootGovernanceInput,
};
pub(crate) use lifecycle::{
    TaskRegistry,
    governance::{
        AppGovernance, ObservabilitySnapshotProvider, RuntimeGovernancePort,
        RuntimeGovernanceSnapshot, RuntimeReloader, SessionInfoProvider,
    },
};
pub(crate) use mcp::{McpConfigScope, RegisterMcpServerInput};
pub(crate) use mode::{
    CompiledModeEnvelope, ModeCatalog, builtin_mode_catalog, compile_mode_envelope,
    compile_mode_envelope_for_child,
};
pub(crate) use observability::{GovernanceSnapshot, RuntimeObservabilityCollector};
pub(crate) use ports::{
    AgentKernelPort, AgentSessionPort, AppAgentPromptSubmission, RecoverableParentDelivery,
    SessionTurnOutcomeSummary,
};
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt};

use crate::{
    agent_api::ServerAgentApi,
    application_error_bridge::ServerRouteError,
    auth::{AuthSessionManager, BootstrapAuth},
    bootstrap::{
        ServerRuntimeHandles, attach_frontend_build, build_cors_layer, clear_run_info,
        prepare_server_launch,
    },
    config_service_bridge::ServerConfigService,
    governance_service::ServerGovernanceService,
    mcp_service::ServerMcpService,
    mode_catalog_service::ServerModeCatalog,
    profile_service::ServerProfileService,
    routes::build_api_router,
};

/// 认证请求头名称。
///
/// 所有 API 请求通过此请求头携带认证 token，
/// TODO:备选方案是通过 `token` 查询参数传递（SSE EventSource 不支持自定义请求头）。
pub(crate) const AUTH_HEADER_NAME: &str = "x-astrcode-token";

/// 应用状态（共享给所有路由处理器）。
///
/// 通过 Axum 的 `State` 提取器注入到每个路由处理器中，
/// 包含运行时入口、server 侧 owner bridge、治理模型、认证管理器和前端构建产物。
/// 所有字段均为 `Arc` 或可 `Clone` 类型，支持多线程共享。
#[derive(Clone)]
#[allow(dead_code)]
pub(crate) struct AppState {
    /// server-owned agent route bridge；agent routes 不再经由 `application::agent` 用例入口。
    agent_api: Arc<ServerAgentApi>,
    /// server-owned agent control bridge；测试和路由不直接暴露底层 kernel。
    agent_control: Arc<dyn agent_control_bridge::ServerAgentControlPort>,
    /// server-owned 配置服务桥接；配置/模型 API 不再经由 App 访问配置。
    config: Arc<ServerConfigService>,
    /// server-owned 会话目录桥接；catalog API 不再经由 App 访问 session catalog。
    session_catalog: Arc<SessionCatalog>,
    /// server-owned profile resolver；watch/profile 测试不再经由 `App::profiles()`.
    profiles: Arc<ServerProfileService>,
    /// subagent 启动桥接；测试直接消费 host-session 合同。
    subagent_executor: Arc<dyn SubAgentExecutor>,
    /// server-owned MCP service；MCP API 不再经由 App facade。
    mcp_service: Arc<ServerMcpService>,
    /// server-owned skill catalog bridge；composer/skill discovery 不再经由 App facade。
    skill_catalog: Arc<dyn SkillCatalog>,
    /// server-owned plugin resource catalog；commands/prompts/themes/resources discovery 统一走
    /// plugin-host。
    resource_catalog: Arc<std::sync::RwLock<ResourceCatalog>>,
    /// server-owned mode catalog bridge；mode API 不再经由 App 访问 mode catalog。
    mode_catalog: Arc<ServerModeCatalog>,
    /// server-owned 治理层（快照/shutdown/reload）。
    governance: Arc<ServerGovernanceService>,
    /// 认证会话管理器
    auth_sessions: Arc<AuthSessionManager>,
    /// Bootstrap 阶段的认证（短期 token）
    bootstrap_auth: BootstrapAuth,
    /// 前端构建产物（可选）
    frontend_build: Option<FrontendBuild>,
    /// server 侧运行时资源守卫。
    _runtime_handles: Arc<ServerRuntimeHandles>,
}

/// 前端构建产物。
///
/// 包含 dist 目录路径和注入过 bootstrap token 的 index.html 内容。
/// 如果前端未构建，此字段为 `None`，服务器将只提供 API 路由。
#[derive(Clone)]
pub(crate) struct FrontendBuild {
    /// dist 目录路径
    dist_dir: PathBuf,
    /// index.html 内容（已注入 bootstrap token 脚本）
    index_html: Arc<String>,
}

/// 错误响应载荷。
///
/// 所有 API 错误统一返回此结构，包含 `error` 字段描述错误信息。
#[derive(Debug, Serialize)]
struct ErrorPayload {
    error: String,
}

/// API 错误。
///
/// 将业务逻辑错误映射为 HTTP 状态码和错误消息。
/// 支持 400（无效输入）、401（未认证）、404（未找到）、
/// 409（冲突）、500（内部错误）。
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

    pub(crate) fn bad_request(message: String) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message,
        }
    }

    pub(crate) fn internal_server_error(message: String) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message,
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

impl From<AstrError> for ApiError {
    fn from(value: AstrError) -> Self {
        let message = value.to_string();
        match value {
            AstrError::SessionNotFound(_) | AstrError::ProjectNotFound(_) => Self {
                status: StatusCode::NOT_FOUND,
                message,
            },
            AstrError::TurnInProgress(_) => Self {
                status: StatusCode::CONFLICT,
                message,
            },
            AstrError::InvalidSessionId(_)
            | AstrError::ConfigError { .. }
            | AstrError::MissingApiKey(_)
            | AstrError::MissingBaseUrl(_)
            | AstrError::NoProfilesConfigured
            | AstrError::ModelNotFound { .. }
            | AstrError::UnsupportedProvider(_)
            | AstrError::Validation(_) => Self {
                status: StatusCode::BAD_REQUEST,
                message,
            },
            AstrError::Cancelled => Self {
                status: StatusCode::CONFLICT,
                message,
            },
            _ => Self {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                message,
            },
        }
    }
}

impl From<ServerRouteError> for ApiError {
    fn from(value: ServerRouteError) -> Self {
        match value {
            ServerRouteError::NotFound(message) => Self {
                status: StatusCode::NOT_FOUND,
                message,
            },
            ServerRouteError::Conflict(message) => Self {
                status: StatusCode::CONFLICT,
                message,
            },
            ServerRouteError::InvalidArgument(message) => Self {
                status: StatusCode::BAD_REQUEST,
                message,
            },
            ServerRouteError::PermissionDenied(message) => Self {
                status: StatusCode::FORBIDDEN,
                message,
            },
            ServerRouteError::Internal(message) => Self {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                message,
            },
        }
    }
}

/// 服务器主入口
///
/// 启动流程：
/// 1. Bootstrap 运行时（加载插件、初始化 LLM）
/// 2. 绑定随机端口（127.0.0.1:0）
/// 3. 生成 bootstrap token
/// 4. 构造共享的本地 server 信息 DTO
/// 5. 写入 run.json（供浏览器桥接/诊断读取）
/// 6. 通过 stdout 发出结构化 ready 事件（供桌面端等待）
/// 7. 启动 HTTP 服务器
#[tokio::main]
async fn main() -> AnyhowResult<()> {
    // 初始化日志：stderr（开发调试）+ 文件（warn+ 持久化）
    logging::init_logger();

    let runtime = crate::bootstrap::bootstrap_server_runtime()
        .await
        .map_err(|error| anyhow!(error.to_string()))?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| AstrError::io("failed to bind server listener", e))?;
    let address: SocketAddr = listener
        .local_addr()
        .map_err(|e| AstrError::io("failed to resolve server listener address", e))?;
    let started_at = chrono::Utc::now();
    let prepared_launch = prepare_server_launch(address.port(), started_at)?;
    log::info!(
        "Ready: http://localhost:{}/ (API routes live under /api)",
        address.port()
    );

    let state = AppState {
        agent_api: Arc::clone(&runtime.agent_api),
        agent_control: Arc::clone(&runtime.agent_control),
        config: Arc::clone(&runtime.config),
        session_catalog: Arc::clone(&runtime.session_catalog),
        profiles: Arc::clone(&runtime.profiles),
        subagent_executor: Arc::clone(&runtime.subagent_executor),
        mcp_service: Arc::clone(&runtime.mcp_service),
        skill_catalog: Arc::clone(&runtime.skill_catalog),
        resource_catalog: Arc::clone(&runtime.resource_catalog),
        mode_catalog: Arc::clone(&runtime.mode_catalog),
        governance: Arc::clone(&runtime.governance),
        auth_sessions: Arc::new(AuthSessionManager::default()),
        bootstrap_auth: prepared_launch.bootstrap_auth,
        frontend_build: prepared_launch.frontend_build.clone(),
        _runtime_handles: Arc::clone(&runtime.handles),
    };
    let shutdown_governance = Arc::clone(&state.governance);
    let shutdown_pid = std::process::id();

    let app: Router<AppState> = build_api_router();
    let app = attach_frontend_build(app, prepared_launch.frontend_build);
    let app = app.with_state(state).layer(build_cors_layer());

    Ok(axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            if let Err(error) = shutdown_governance.shutdown(5).await {
                log::error!("graceful shutdown finished with errors: {}", error);
            }
            if let Err(error) = clear_run_info(shutdown_pid) {
                log::warn!("failed to clear run info during shutdown: {}", error);
            }
        })
        .await
        .map_err(|e| AstrError::io("server terminated unexpectedly", e))?)
}

/// 等待关闭信号
///
/// 支持 Ctrl+C（所有平台）、SIGTERM（Unix）以及桌面宿主关闭 stdin（Tauri sidecar）。
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            log::error!("failed to install Ctrl+C shutdown handler: {}", error);
        }
    };
    let stdin_closed = wait_for_shutdown_pipe(tokio::io::stdin());

    #[cfg(unix)]
    let terminate = async {
        let mut signal = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("SIGTERM handler should install");
        signal.recv().await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
        _ = stdin_closed => {}
    }
}

/// 等待宿主关闭 stdin 管道。
///
/// 为什么单独拆出来：
/// 让“读到 EOF 才触发优雅关闭”这条语义能被单测锁住，避免以后改
/// `shutdown_signal()` 时把 stdin 生命周期行为意外改掉。
async fn wait_for_shutdown_pipe<R>(mut reader: R)
where
    R: AsyncRead + Unpin,
{
    let mut buffer = [0_u8; 64];
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) => break,
            Ok(_) => {},
            Err(error) => {
                // stdin 读取失败时宁可尽快结束，也不要把桌面端退出卡成僵尸 sidecar。
                log::warn!("failed to read stdin shutdown pipe: {}", error);
                break;
            },
        }
    }
}

#[cfg(test)]
mod shutdown_tests {
    use std::{
        io,
        pin::Pin,
        task::{Context, Poll},
    };

    use tokio::{
        io::{AsyncRead, DuplexStream, ReadBuf, duplex},
        time::{Duration, timeout},
    };

    use super::wait_for_shutdown_pipe;

    #[tokio::test]
    async fn shutdown_pipe_waits_for_eof_even_after_receiving_data() {
        let (reader, mut writer): (DuplexStream, DuplexStream) = duplex(64);
        let mut waiter = tokio::spawn(wait_for_shutdown_pipe(reader));

        tokio::io::AsyncWriteExt::write_all(&mut writer, b"still-alive")
            .await
            .expect("writer should accept probe bytes");

        let still_waiting = timeout(Duration::from_millis(80), &mut waiter).await;
        assert!(
            still_waiting.is_err(),
            "shutdown pipe should not resolve before stdin reaches EOF"
        );

        drop(writer);
        timeout(Duration::from_millis(300), waiter)
            .await
            .expect("waiter should resolve once stdin reaches EOF")
            .expect("waiter task should complete cleanly");
    }

    #[tokio::test]
    async fn shutdown_pipe_accepts_immediate_eof() {
        timeout(
            Duration::from_millis(100),
            wait_for_shutdown_pipe(tokio::io::empty()),
        )
        .await
        .expect("an already-closed stdin pipe should resolve immediately");
    }

    #[derive(Default)]
    struct ErrorReader;

    impl AsyncRead for ErrorReader {
        fn poll_read(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            _buf: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            Poll::Ready(Err(io::Error::other("synthetic stdin failure")))
        }
    }

    #[tokio::test]
    async fn shutdown_pipe_treats_read_errors_as_shutdown() {
        timeout(
            Duration::from_millis(100),
            wait_for_shutdown_pipe(ErrorReader),
        )
        .await
        .expect("stdin read errors should end the shutdown wait promptly");
    }
}
