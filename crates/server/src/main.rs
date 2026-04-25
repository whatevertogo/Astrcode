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

mod application;
mod bootstrap;
mod config;
mod config_mode_helpers;
mod http;
mod logging;
mod mcp;
mod mode;
mod observability;
mod read_model;
mod runtime_bridge;
mod session_identity;
#[cfg(test)]
mod tests;
use std::{net::SocketAddr, sync::Arc};

use anyhow::{Result as AnyhowResult, anyhow};
pub(crate) use application::{
    agent,
    agent::AgentOrchestrationService,
    error as errors,
    error::ServerApplicationError,
    execution,
    execution::{ExecutionControl, ProfileProvider, ProfileResolutionService},
    governance_surface,
    governance_surface::{
        GovernanceSurfaceAssembler, ResolvedGovernanceSurface, RootGovernanceInput,
    },
    lifecycle,
    lifecycle::{
        TaskRegistry,
        governance::{
            AppGovernance, ObservabilitySnapshotProvider, RuntimeGovernancePort,
            RuntimeGovernanceSnapshot, RuntimeReloader, SessionInfoProvider,
        },
    },
    root_execute as root_execute_service, route_error as application_error_bridge,
};
use astrcode_core::AstrError;
use axum::Router;
pub(crate) use config::ConfigService;
pub(crate) use http::{
    AUTH_HEADER_NAME, ApiError, AppState, FrontendBuild, agent_api, auth, composer_catalog, mapper,
    routes,
};
pub(crate) use mcp::{McpConfigScope, RegisterMcpServerInput};
pub(crate) use mode::{
    CompiledModeEnvelope, compile_mode_envelope, compile_mode_envelope_for_child,
};
pub(crate) use observability::{GovernanceSnapshot, RuntimeObservabilityCollector};
pub(crate) use read_model::{
    conversation as conversation_read_model, terminal as terminal_projection, view_projection,
};
pub(crate) use runtime_bridge::{
    agent_control as agent_control_bridge, agent_control_registry,
    agent_runtime as agent_runtime_bridge, capability_router,
    config_service as config_service_bridge, governance_service, hook_dispatcher as hook_adapter,
    mcp_service, mode_catalog as mode_catalog_service, ports,
    ports::{
        AgentKernelPort, AgentSessionPort, AppAgentPromptSubmission, RecoverableParentDelivery,
        SessionTurnOutcomeSummary,
    },
    profile_service, runtime_owner as runtime_owner_bridge,
    session_owner as session_runtime_owner_bridge, session_port as session_runtime_port,
    tool_capability as tool_capability_invoker, watch_service,
};
#[cfg(test)]
pub(crate) use tests::test_support;
use tokio::io::{AsyncRead, AsyncReadExt};

use crate::{
    auth::AuthSessionManager,
    bootstrap::{attach_frontend_build, build_cors_layer, clear_run_info, prepare_server_launch},
    routes::build_api_router,
};

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
        config: Arc::clone(&runtime.config),
        session_catalog: Arc::clone(&runtime.session_catalog),
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
