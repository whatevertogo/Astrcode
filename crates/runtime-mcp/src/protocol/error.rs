//! # MCP 协议错误类型
//!
//! 定义 MCP 层面的错误类型，并实现到 AstrError 的转换。

use astrcode_core::{AstrError, Result};
use thiserror::Error;

/// MCP 协议层错误。
#[derive(Debug, Error)]
pub enum McpProtocolError {
    /// 握手失败（协议版本不兼容、服务器拒绝等）。
    #[error("MCP handshake failed: {0}")]
    HandshakeFailed(String),

    /// 协议版本不兼容。
    #[error("MCP protocol version incompatible: server={server}, client={client}")]
    VersionMismatch { server: String, client: String },

    /// 工具调用失败（服务器返回 error）。
    #[error("MCP tool call failed: {tool} - {reason}")]
    ToolCallFailed { tool: String, reason: String },

    /// 工具未找到。
    #[error("MCP tool not found: {0}")]
    ToolNotFound(String),

    /// 请求超时。
    #[error("MCP request timeout: {method} after {timeout_secs}s")]
    Timeout { method: String, timeout_secs: u64 },

    /// 响应解析失败。
    #[error("MCP response parse error: {0}")]
    ParseError(String),

    /// JSON-RPC 错误响应。
    #[error("JSON-RPC error {code}: {message}")]
    JsonRpcError { code: i64, message: String },

    /// 连接已断开。
    #[error("MCP connection lost: {server}")]
    ConnectionLost { server: String },

    /// 服务器未连接。
    #[error("MCP server not connected: {0}")]
    NotConnected(String),

    /// 服务器初始化失败。
    #[error("MCP server initialization failed: {server} - {reason}")]
    InitFailed { server: String, reason: String },
}

impl From<McpProtocolError> for AstrError {
    fn from(e: McpProtocolError) -> Self {
        match e {
            McpProtocolError::Timeout { .. } => AstrError::Internal(e.to_string()),
            McpProtocolError::ConnectionLost { .. } => AstrError::Network(e.to_string()),
            _ => AstrError::Internal(e.to_string()),
        }
    }
}

impl From<McpProtocolError> for Result<()> {
    fn from(e: McpProtocolError) -> Self {
        Err(e.into())
    }
}
