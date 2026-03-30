//! # Astrcode HTTP 服务器
//!
//! 本库是 Astrcode 的 HTTP/SSE 服务器入口，负责：
//!
//! - **API 路由**: 暴露 REST API 和 SSE 端点
//! - **认证**: Token 验证和会话管理
//! - **静态资源**: 托管前端构建产物
//! - **优雅关闭**: 处理 Ctrl+C 和 SIGTERM 信号

#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod auth;
#[cfg(test)]
mod auth_routes_tests;
mod bootstrap;
#[cfg(test)]
mod browser_bootstrap_tests;
mod mapper;
mod routes;
#[cfg(test)]
mod runtime_routes_tests;
#[cfg(test)]
mod test_support;
#[cfg(all(test, target_os = "windows"))]
mod windows_subsystem_tests;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Result as AnyhowResult};
use astrcode_core::{AstrError, RuntimeCoordinator};
use astrcode_runtime::{bootstrap_runtime, RuntimeGovernance, RuntimeService, ServiceError};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde::Serialize;

use crate::auth::{AuthSessionManager, BootstrapAuth};
use crate::bootstrap::{
    attach_frontend_build, bootstrap_token_expires_at_ms, build_cors_layer, clear_run_info,
    load_frontend_build, random_hex_token, write_run_info,
};
use crate::routes::build_api_router;

/// 认证请求头名称
pub(crate) const AUTH_HEADER_NAME: &str = "x-astrcode-token";
/// 会话游标响应头名称（用于 SSE 断点续传）
pub(crate) const SESSION_CURSOR_HEADER_NAME: &str = "x-session-cursor";

/// 应用状态（共享给所有路由处理器）
#[derive(Clone)]
pub(crate) struct AppState {
    /// 运行时服务
    service: Arc<RuntimeService>,
    /// 运行时协调器（用于关闭）
    coordinator: Arc<RuntimeCoordinator>,
    /// 运行时治理（重载、配置）
    runtime_governance: Arc<RuntimeGovernance>,
    /// 认证会话管理器
    auth_sessions: Arc<AuthSessionManager>,
    /// Bootstrap 阶段的认证（短期 token）
    bootstrap_auth: BootstrapAuth,
    /// 前端构建产物（可选）
    frontend_build: Option<FrontendBuild>,
}

/// 前端构建产物
#[derive(Clone)]
pub(crate) struct FrontendBuild {
    /// dist 目录路径
    dist_dir: PathBuf,
    /// index.html 内容
    index_html: Arc<String>,
}

/// 错误响应载荷
#[derive(Debug, Serialize)]
struct ErrorPayload {
    error: String,
}

/// API 错误
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

/// 服务器主入口
///
/// 启动流程：
/// 1. Bootstrap 运行时（加载插件、初始化 LLM）
/// 2. 绑定随机端口（127.0.0.1:0）
/// 3. 生成 bootstrap token
/// 4. 加载前端构建产物
/// 5. 写入 run.json（供前端读取）
/// 6. 启动 HTTP 服务器
#[tokio::main]
async fn main() -> AnyhowResult<()> {
    let runtime = bootstrap_runtime()
        .await
        .map_err(|error| anyhow!(error.to_string()))?;
    let service = runtime.service;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| AstrError::io("failed to bind server listener", e))?;
    let address: SocketAddr = listener
        .local_addr()
        .map_err(|e| AstrError::io("failed to resolve server listener address", e))?;
    let token = random_hex_token();
    let bootstrap_expires_at_ms = bootstrap_token_expires_at_ms(chrono::Utc::now());
    let bootstrap_auth = BootstrapAuth::new(token.clone(), bootstrap_expires_at_ms);
    let server_origin = format!("http://127.0.0.1:{}", address.port());
    let frontend_build = load_frontend_build(&server_origin, bootstrap_auth.token())?;
    write_run_info(
        address.port(),
        bootstrap_auth.token(),
        bootstrap_auth.expires_at_ms(),
    )?;
    println!(
        "Ready: http://localhost:{}/ (API routes live under /api)",
        address.port()
    );

    let state = AppState {
        service: Arc::clone(&service),
        coordinator: Arc::clone(&runtime.coordinator),
        runtime_governance: Arc::clone(&runtime.governance),
        auth_sessions: Arc::new(AuthSessionManager::default()),
        bootstrap_auth,
        frontend_build: frontend_build.clone(),
    };
    let shutdown_coordinator = Arc::clone(&state.coordinator);
    let shutdown_pid = std::process::id();

    let app: Router<AppState> = build_api_router();
    let app = attach_frontend_build(app, frontend_build);
    let app = app.with_state(state).layer(build_cors_layer());

    Ok(axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            if let Err(error) = shutdown_coordinator.shutdown(5).await {
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
/// 支持 Ctrl+C（所有平台）和 SIGTERM（Unix）。
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            log::error!("failed to install Ctrl+C shutdown handler: {}", error);
        }
    };

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
    }
}
