//! # MCP 传输层
//!
//! 定义 `McpTransport` trait——所有 MCP 传输实现的统一抽象。
//! 传输层负责底层消息收发，不涉及 MCP 协议语义。

pub mod http;
pub mod sse;
pub mod stdio;

#[cfg(test)]
pub mod mock;

use astrcode_core::Result;
use async_trait::async_trait;

use crate::protocol::types::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};

/// MCP 传输层的统一抽象。
///
/// 所有传输实现（stdio、Streamable HTTP、SSE）都必须实现此 trait。
/// 传输层负责底层的消息收发，不涉及 MCP 协议语义。
#[async_trait]
pub trait McpTransport: Send + Sync {
    /// 启动传输通道，准备收发消息。
    ///
    /// 对于 stdio，此方法启动子进程；
    /// 对于 HTTP/SSE，此方法验证 URL 可达性。
    async fn start(&mut self) -> Result<()>;

    /// 发送一条 JSON-RPC 请求并等待响应。
    async fn send_request(&self, request: JsonRpcRequest) -> Result<JsonRpcResponse>;

    /// 发送一条 JSON-RPC 通知（无响应）。
    async fn send_notification(&self, notification: JsonRpcNotification) -> Result<()>;

    /// 关闭传输通道，释放所有资源。
    async fn close(&mut self) -> Result<()>;

    /// 传输通道是否处于活跃状态。
    fn is_active(&self) -> bool;

    /// 传输类型的标识符（用于日志）。
    fn transport_type(&self) -> &'static str;
}
