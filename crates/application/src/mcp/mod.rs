//! MCP 管理用例。
//!
//! 提供 MCP 服务器的生命周期管理用例：
//! - 查询服务器状态
//! - 审批/拒绝服务器
//! - 配置管理（注册/移除/启禁用）
//! - 重连服务器
//!
//! IO 和连接管理通过 `McpPort` 端口委托给 adapter 层。
//! 传输协议细节（stdio/http/sse）不属于 application 层。

use std::sync::Arc;

use async_trait::async_trait;

use crate::ApplicationError;

// ============================================================
// 业务模型
// ============================================================

/// MCP 配置所属作用域。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpConfigScope {
    User,
    Project,
    Local,
}

/// MCP 服务器状态的业务视图。
///
/// 只包含业务关心的信息，不暴露连接协议细节。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerStatusView {
    pub name: String,
    pub scope: String,
    pub enabled: bool,
    pub state: String,
    pub error: Option<String>,
    pub tool_count: usize,
    pub prompt_count: usize,
    pub resource_count: usize,
    pub pending_approval: bool,
    pub server_signature: String,
}

/// 注册 MCP 服务器的业务输入。
///
/// 包含业务层面的配置信息（名称、超时等），
/// 传输协议细节由调用方（server handler）自行解析后传给 adapter。
#[derive(Debug, Clone)]
pub struct RegisterMcpServerInput {
    pub name: String,
    pub scope: McpConfigScope,
    pub enabled: bool,
    pub timeout_secs: u64,
    pub init_timeout_secs: u64,
    pub max_reconnect_attempts: u32,
    /// 序列化的传输配置，由 adapter 层解析。
    /// 用 `serde_json::Value` 避免在 application 层定义传输协议类型。
    pub transport_config: serde_json::Value,
}

// ============================================================
// MCP 用例端口
// ============================================================

/// MCP 操作端口，由 adapter-mcp 实现。
///
/// 将 MCP 连接管理和协议细节从 application 层剥离。
/// 方法为 async 以支持底层异步 I/O 操作（连接、握手、传输层管理）。
#[async_trait]
pub trait McpPort: Send + Sync {
    /// 列出所有 MCP 服务器状态。
    async fn list_server_status(&self) -> Vec<McpServerStatusView>;
    /// 审批服务器连接。
    async fn approve_server(&self, server_signature: &str) -> Result<(), ApplicationError>;
    /// 拒绝服务器连接。
    async fn reject_server(&self, server_signature: &str) -> Result<(), ApplicationError>;
    /// 重新连接指定服务器。
    async fn reconnect_server(&self, name: &str) -> Result<(), ApplicationError>;
    /// 重置项目级审批选择。
    async fn reset_project_choices(&self) -> Result<(), ApplicationError>;
    /// 注册或更新 MCP 服务器配置。
    async fn upsert_server(&self, input: &RegisterMcpServerInput) -> Result<(), ApplicationError>;
    /// 移除指定作用域和名称的 MCP 服务器配置。
    async fn remove_server(
        &self,
        scope: McpConfigScope,
        name: &str,
    ) -> Result<(), ApplicationError>;
    /// 启用或禁用 MCP 服务器。
    async fn set_server_enabled(
        &self,
        scope: McpConfigScope,
        name: &str,
        enabled: bool,
    ) -> Result<(), ApplicationError>;
}

// ============================================================
// MCP 用例服务
// ============================================================

/// MCP 管理用例入口。
///
/// 所有方法都是业务操作，通过 `McpPort` 委托给适配器层。
pub struct McpService {
    port: Arc<dyn McpPort>,
}

impl McpService {
    pub fn new(port: Arc<dyn McpPort>) -> Self {
        Self { port }
    }

    /// 用例：查看所有 MCP 服务器状态。
    pub async fn list_status(&self) -> Vec<McpServerStatusView> {
        self.port.list_server_status().await
    }

    /// 用例：审批 MCP 服务器。
    pub async fn approve_server(&self, server_signature: &str) -> Result<(), ApplicationError> {
        self.port.approve_server(server_signature).await
    }

    /// 用例：拒绝 MCP 服务器。
    pub async fn reject_server(&self, server_signature: &str) -> Result<(), ApplicationError> {
        self.port.reject_server(server_signature).await
    }

    /// 用例：重新连接 MCP 服务器。
    pub async fn reconnect_server(&self, name: &str) -> Result<(), ApplicationError> {
        self.port.reconnect_server(name).await
    }

    /// 用例：重置项目级审批选择。
    pub async fn reset_project_choices(&self) -> Result<(), ApplicationError> {
        self.port.reset_project_choices().await
    }

    /// 用例：注册或更新 MCP 服务器。
    pub async fn upsert_config(
        &self,
        input: RegisterMcpServerInput,
    ) -> Result<(), ApplicationError> {
        self.port.upsert_server(&input).await
    }

    /// 用例：移除 MCP 服务器配置。
    pub async fn remove_config(
        &self,
        scope: McpConfigScope,
        name: &str,
    ) -> Result<(), ApplicationError> {
        self.port.remove_server(scope, name).await
    }

    /// 用例：启用或禁用 MCP 服务器。
    pub async fn set_enabled(
        &self,
        scope: McpConfigScope,
        name: &str,
        enabled: bool,
    ) -> Result<(), ApplicationError> {
        self.port.set_server_enabled(scope, name, enabled).await
    }
}

impl std::fmt::Debug for McpService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpService").finish_non_exhaustive()
    }
}
