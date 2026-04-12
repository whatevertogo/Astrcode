# Contract: McpTransport Trait

**Feature**: 009-mcp-integration
**Purpose**: 定义 MCP 传输层的统一抽象接口

## 接口签名

```rust
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

    /// 发送一条 JSON-RPC 消息并等待响应。
    ///
    /// 此方法负责消息的序列化和传输。
    /// 对于支持响应关联的传输，此方法等待匹配的响应。
    async fn send_request(&self, request: JsonRpcRequest) -> Result<JsonRpcResponse>;

    /// 发送一条 JSON-RPC 通知（无响应）。
    async fn send_notification(&self, notification: JsonRpcNotification) -> Result<()>;

    /// 关闭传输通道，释放所有资源。
    ///
    /// 对于 stdio，按 SIGINT → SIGTERM → SIGKILL 顺序优雅关闭子进程。
    /// 对于 HTTP/SSE，关闭连接。
    async fn close(&mut self) -> Result<()>;

    /// 传输通道是否处于活跃状态。
    fn is_active(&self) -> bool;

    /// 传输类型的标识符（用于日志）。
    fn transport_type(&self) -> &'static str;
}
```

## 实现

| 实现 | 文件 | 说明 |
|------|------|------|
| StdioTransport | `transport/stdio.rs` | 子进程 stdin/stdout JSON-RPC |
| StreamableHttpTransport | `transport/http.rs` | HTTP POST + SSE 响应流 |
| SseTransport | `transport/sse.rs` | SSE 连接 + HTTP POST 请求 |

## 错误处理

所有传输实现必须：
- 网络错误包装为 `AstrError::Io` 或 `AstrError::Connection`
- 超时错误包装为 `AstrError::Timeout`
- 协议错误（非 JSON、无效 JSON-RPC）包装为 `AstrError::Protocol`
- 连接断开时将 `is_active()` 置为 false
