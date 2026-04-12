//! # MCP 配置数据类型
//!
//! 定义 MCP 服务器配置、传输配置和配置作用域等数据结构。
//! 从 `.mcp.json` 和 settings JSON 反序列化。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::protocol::types::McpOAuthConfig;

/// MCP 配置作用域（优先级从低到高）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum McpConfigScope {
    /// 用户全局配置（`~/.astrcode/config.json`）
    User,
    /// 项目级配置（`.mcp.json`）
    Project,
    /// 项目本地私有配置（`.astrcode/config.json`）
    Local,
}

/// MCP 服务器完整配置。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// 服务器唯一标识（仅允许 `[a-zA-Z0-9_-]`）。
    pub name: String,
    /// 传输配置。
    pub transport: McpTransportConfig,
    /// 配置来源作用域。
    pub scope: McpConfigScope,
    /// 是否启用（默认 true）。
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 单次请求超时（秒，默认 120）。
    #[serde(default = "default_tool_timeout")]
    pub timeout_secs: u64,
    /// 握手超时（秒，默认 30）。
    #[serde(default = "default_init_timeout")]
    pub init_timeout_secs: u64,
    /// 最大重连次数（默认 5，仅远程传输）。
    #[serde(default = "default_max_reconnect")]
    pub max_reconnect_attempts: u32,
}

/// MCP 传输配置（联合类型）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum McpTransportConfig {
    /// stdio 传输：启动子进程通过 stdin/stdout 通信。
    #[serde(rename = "stdio")]
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: HashMap<String, String>,
    },
    /// Streamable HTTP 传输（推荐的远程模式）。
    #[serde(rename = "http")]
    StreamableHttp {
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        oauth: Option<McpOAuthConfig>,
    },
    /// SSE 传输（兼容回退远程模式）。
    #[serde(rename = "sse")]
    Sse {
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        oauth: Option<McpOAuthConfig>,
    },
}

/// 从 `.mcp.json` 文件反序列化的顶层结构。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpJsonFile {
    #[serde(rename = "mcpServers")]
    pub mcp_servers: HashMap<String, McpJsonServerEntry>,
}

/// `.mcp.json` 中单个服务器条目（传输类型由字段推断）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpJsonServerEntry {
    /// stdio 命令（存在时推断为 stdio 传输）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// stdio 参数。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    /// stdio 环境变量。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
    /// 传输类型显式声明（"http" / "sse"，缺省时从 command 推断为 stdio）。
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub transport_type: Option<String>,
    /// 远程 URL（http/sse 传输时必填）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// 远程传输的 HTTP headers。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,
    /// 是否禁用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
    /// 超时覆盖。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    /// 初始化握手超时覆盖。
    #[serde(
        default,
        rename = "initTimeout",
        skip_serializing_if = "Option::is_none"
    )]
    pub init_timeout: Option<u64>,
    /// 最大重连次数覆盖。
    #[serde(
        default,
        rename = "maxReconnectAttempts",
        skip_serializing_if = "Option::is_none"
    )]
    pub max_reconnect_attempts: Option<u32>,
}

impl McpTransportConfig {
    /// 是否为远程传输（HTTP/SSE）。
    pub fn is_remote(&self) -> bool {
        matches!(self, Self::StreamableHttp { .. } | Self::Sse { .. })
    }
}

// ========== 辅助函数 ==========

const fn default_true() -> bool {
    true
}

const fn default_tool_timeout() -> u64 {
    120
}

const fn default_init_timeout() -> u64 {
    30
}

const fn default_max_reconnect() -> u32 {
    5
}
