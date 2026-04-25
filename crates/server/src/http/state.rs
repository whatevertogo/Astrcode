use std::{path::PathBuf, sync::Arc};

use astrcode_core::SkillCatalog;
use astrcode_host_session::SessionCatalog;
use astrcode_plugin_host::ResourceCatalog;

use crate::{
    agent_api::ServerAgentApi,
    auth::{AuthSessionManager, BootstrapAuth},
    bootstrap::ServerRuntimeHandles,
    config_service_bridge::ServerConfigService,
    governance_service::ServerGovernanceService,
    mcp_service::ServerMcpService,
    mode_catalog_service::ServerModeCatalog,
};

/// 应用状态（共享给所有路由处理器）。
///
/// 通过 Axum 的 `State` 提取器注入到每个路由处理器中，包含运行时入口、server 侧
/// owner bridge、治理模型、认证管理器和前端构建产物。
#[derive(Clone)]
pub(crate) struct AppState {
    /// server-owned agent route 入口。
    pub(crate) agent_api: Arc<ServerAgentApi>,
    /// server-owned 配置服务。
    pub(crate) config: Arc<ServerConfigService>,
    /// server-owned 会话目录。
    pub(crate) session_catalog: Arc<SessionCatalog>,
    /// server-owned MCP service。
    pub(crate) mcp_service: Arc<ServerMcpService>,
    /// server-owned skill catalog。
    pub(crate) skill_catalog: Arc<dyn SkillCatalog>,
    /// server-owned plugin resource catalog。
    pub(crate) resource_catalog: Arc<std::sync::RwLock<ResourceCatalog>>,
    /// server-owned mode catalog。
    pub(crate) mode_catalog: Arc<ServerModeCatalog>,
    /// server-owned 治理层（快照/shutdown/reload）。
    pub(crate) governance: Arc<ServerGovernanceService>,
    /// 认证会话管理器。
    pub(crate) auth_sessions: Arc<AuthSessionManager>,
    /// Bootstrap 阶段的认证（短期 token）。
    pub(crate) bootstrap_auth: BootstrapAuth,
    /// 前端构建产物（可选）。
    pub(crate) frontend_build: Option<FrontendBuild>,
    /// server 侧运行时资源守卫。
    pub(crate) _runtime_handles: Arc<ServerRuntimeHandles>,
}

/// 前端构建产物。
///
/// 包含 dist 目录路径和注入过 bootstrap token 的 index.html 内容。
#[derive(Clone)]
pub(crate) struct FrontendBuild {
    /// dist 目录路径。
    pub(crate) dist_dir: PathBuf,
    /// index.html 内容（已注入 bootstrap token 脚本）。
    pub(crate) index_html: Arc<String>,
}
