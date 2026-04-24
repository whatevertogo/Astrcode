//! MCP 管理用例。
//!
//! 提供 MCP 服务器的生命周期管理用例：
//! - 查询服务器状态
//! - 审批/拒绝服务器
//! - 配置管理（注册/移除/启禁用）
//! - 重连服务器
//!
//! 这里保留 HTTP 配置写路径共用的业务输入类型。运行时 MCP 管理端口和服务入口
//! 位于 `server::mcp_service`，由 server 组合根直接接线到 adapter 层。

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
