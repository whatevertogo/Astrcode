//! # HTTP 路由模块
//!
//! 本模块定义所有 HTTP/SSE API 路由，按业务领域拆分为子模块：
//!
//! - **sessions**：会话 CRUD、提示提交、会话目录事件流（SSE）
//! - **conversation**：统一的产品会话读取面（snapshot/stream/slash candidates）
//! - **composer**：输入框候选列表
//! - **config**：配置查看和活跃选择保存
//! - **model**：模型列表、当前模型、连接测试
//! - **agents**：Agent profile 查询与子会话执行控制
//!
//! ## 路由约定
//!
//! - 所有业务端点挂载在 `/api` 前缀下
//! - Bootstrap 相关端点（`/__astrcode__/run-info`）在路由构建时直接挂载
//! - 认证交换端点 `/api/auth/exchange` 在此模块定义，不走子模块

pub(crate) mod agents;
pub(crate) mod composer;
pub(crate) mod config;
pub(crate) mod conversation;
pub(crate) mod logs;
pub(crate) mod mcp;
pub(crate) mod model;
pub(crate) mod sessions;

use astrcode_protocol::http::{AuthExchangeRequest, AuthExchangeResponse};
use axum::{
    Json, Router,
    extract::State,
    routing::{delete, get, post},
};

use crate::{ApiError, AppState, bootstrap::serve_run_info};

/// 构建完整的 API 路由器。
///
/// 将所有子模块的路由和 bootstrap 端点组装到一个 `Router<AppState>` 中。
/// 路由按业务领域分组，每个端点绑定到对应的处理器函数。
///
/// ## 路由清单
///
/// ### Bootstrap
/// - `GET /__astrcode__/run-info` — 返回 bootstrap token 和 server origin（浏览器开发用）
///
/// ### 认证
/// - `POST /api/auth/exchange` — 用 bootstrap token 交换 API 会话 token
///
/// ### 会话
/// - `POST /api/sessions` — 创建新会话
/// - `GET /api/sessions` — 列出所有会话
/// - `GET /api/session-events` — 订阅会话目录事件（SSE）
/// - `GET /api/sessions/:id/composer/options` — 获取输入框候选列表
/// - `POST /api/sessions/:id/prompts` — 提交用户提示
/// - `POST /api/sessions/:id/compact` — 压缩会话上下文
/// - `POST /api/sessions/:id/interrupt` — 中断会话执行
/// - `DELETE /api/sessions/:id` — 删除单个会话
/// - `DELETE /api/projects` — 删除整个项目（级联删除所有会话）
///
/// ### Conversation
/// - `GET /api/v1/conversation/sessions/{id}/snapshot` — 获取 authoritative conversation snapshot
/// - `GET /api/v1/conversation/sessions/{id}/stream` — 订阅 authoritative conversation delta 流
/// - `GET /api/v1/conversation/sessions/{id}/slash-candidates` — 获取 slash candidates
///
/// ### 配置
/// - `GET /api/config` — 获取当前配置视图
/// - `POST /api/config/reload` — 通过治理入口重载配置、MCP、plugin 与统一 capability surface
/// - `POST /api/config/active-selection` — 保存活跃的 profile/model 选择
///
/// ### 日志
/// - `POST /api/logs` — 前端日志上报，写入服务端日志系统
///
/// ### 模型
/// - `GET /api/models/current` — 获取当前激活的模型信息
/// - `GET /api/models` — 列出所有可用模型选项
/// - `POST /api/models/test` — 测试模型连接
///
/// ### Agent 与子会话
/// - `GET /api/v1/agents` — 列出可用 Agent Profiles
/// - `POST /api/v1/agents/{id}/execute` — 创建 root execution 并返回 session/turn 标识
/// - `GET /api/v1/sessions/{id}/subruns/{sub_run_id}` — 查询子会话执行状态
/// - `POST /api/v1/sessions/{id}/agents/{agent_id}/close` — 关闭 agent 及其子树
pub(crate) fn build_api_router() -> Router<AppState> {
    Router::<AppState>::new()
        .route("/__astrcode__/run-info", get(serve_run_info))
        .route("/api/auth/exchange", post(exchange_auth))
        .route(
            "/api/sessions",
            post(sessions::create_session).get(sessions::list_sessions),
        )
        .route("/api/session-events", get(sessions::session_catalog_events))
        .route(
            "/api/sessions/{id}/composer/options",
            get(composer::session_composer_options),
        )
        .route("/api/sessions/{id}/prompts", post(sessions::submit_prompt))
        .route(
            "/api/sessions/{id}/compact",
            post(sessions::compact_session),
        )
        .route("/api/sessions/{id}/fork", post(sessions::fork_session))
        .route(
            "/api/sessions/{id}/interrupt",
            post(sessions::interrupt_session),
        )
        .route("/api/sessions/{id}", delete(sessions::delete_session))
        .route("/api/projects", delete(sessions::delete_project))
        .route("/api/config", get(config::get_config))
        .route("/api/config/reload", post(config::reload_config))
        .route(
            "/api/config/active-selection",
            post(config::save_active_selection),
        )
        .route("/api/models/current", get(model::get_current_model))
        .route("/api/models", get(model::list_models))
        .route("/api/models/test", post(model::test_model_connection))
        .route("/api/logs", post(logs::ingest))
        .route("/api/v1/agents", get(agents::list_agents))
        .route("/api/v1/agents/{id}/execute", post(agents::execute_agent))
        .route(
            "/api/v1/sessions/{id}/subruns/{sub_run_id}",
            get(agents::get_subrun_status),
        )
        .route(
            "/api/v1/sessions/{id}/agents/{agent_id}/close",
            post(agents::close_agent),
        )
        .route(
            "/api/v1/conversation/sessions/{id}/snapshot",
            get(conversation::conversation_snapshot),
        )
        .route(
            "/api/v1/conversation/sessions/{id}/stream",
            get(conversation::conversation_stream),
        )
        .route(
            "/api/v1/conversation/sessions/{id}/slash-candidates",
            get(conversation::conversation_slash_candidates),
        )
        // MCP 管理
        .route("/api/mcp/status", get(mcp::get_mcp_status))
        .route("/api/mcp/approve", post(mcp::approve_mcp_server))
        .route("/api/mcp/reject", post(mcp::reject_mcp_server))
        .route("/api/mcp/reconnect", post(mcp::reconnect_mcp_server))
        .route(
            "/api/mcp/reset-project-choices",
            post(mcp::reset_project_mcp_choices),
        )
        .route("/api/mcp/server", post(mcp::upsert_mcp_server))
        .route("/api/mcp/server/remove", post(mcp::remove_mcp_server))
        .route("/api/mcp/server/enabled", post(mcp::set_mcp_server_enabled))
}

async fn exchange_auth(
    State(state): State<AppState>,
    Json(request): Json<AuthExchangeRequest>,
) -> Result<Json<AuthExchangeResponse>, ApiError> {
    if !state.bootstrap_auth.validate(&request.token) {
        return Err(ApiError::unauthorized());
    }

    let summary = state.auth_sessions.issue_exchange_summary();
    Ok(Json(AuthExchangeResponse {
        ok: summary.ok,
        token: summary.token,
        expires_at_ms: summary.expires_at_ms,
    }))
}
