//! # Streamable HTTP 传输实现
//!
//! MCP 的推荐远程传输模式：通过 HTTP POST 发送请求，
//! 响应可能是普通 JSON 或 SSE 流（服务端推送多个消息）。

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use astrcode_core::{AstrError, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use log::{debug, info, warn};
use reqwest::Client;

use super::McpTransport;
use crate::protocol::types::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};

/// Streamable HTTP 传输：通过 HTTP POST 发送 JSON-RPC 请求。
///
/// MCP 协议规定远程传输使用 HTTP POST 发送请求，
/// 响应可能是普通 JSON 或 SSE 事件流。
pub struct StreamableHttpTransport {
    /// MCP 服务器 URL。
    url: String,
    /// 静态 HTTP headers（用于认证等）。
    headers: Vec<(String, String)>,
    /// reqwest 客户端（复用连接池）。
    client: Option<Client>,
    /// 传输是否活跃。
    active: Arc<AtomicBool>,
}

impl StreamableHttpTransport {
    /// 创建 Streamable HTTP 传输。
    pub fn new(url: impl Into<String>, headers: Vec<(String, String)>) -> Self {
        Self {
            url: url.into(),
            headers,
            client: None,
            active: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[async_trait]
impl McpTransport for StreamableHttpTransport {
    async fn start(&mut self) -> Result<()> {
        self.client = Some(Client::new());
        self.active.store(true, Ordering::SeqCst);

        info!("MCP Streamable HTTP transport ready: {}", self.url);
        Ok(())
    }

    async fn send_request(&self, request: JsonRpcRequest) -> Result<JsonRpcResponse> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| AstrError::Internal("HTTP transport not started".into()))?;

        if !self.active.load(Ordering::SeqCst) {
            return Err(AstrError::Network("HTTP transport not active".into()));
        }

        let body = serde_json::to_string(&request)
            .map_err(|e| AstrError::parse("serialize JSON-RPC request", e))?;

        debug!("MCP HTTP POST to {}: {} bytes", self.url, body.len());

        let mut req_builder = client
            .post(&self.url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .body(body);

        for (key, value) in &self.headers {
            if let Ok(header_name) = reqwest::header::HeaderName::from_bytes(key.as_bytes()) {
                req_builder = req_builder.header(header_name, value.as_str());
            }
        }

        let response = req_builder
            .send()
            .await
            .map_err(|e| AstrError::http("MCP HTTP request", e))?;

        let status = response.status();
        if !status.is_success() {
            let status_code = status.as_u16();
            let body_text = response.text().await.unwrap_or_default();
            return Err(AstrError::Network(format!(
                "MCP server returned HTTP {}: {}",
                status_code, body_text
            )));
        }

        // 根据响应 Content-Type 决定解析方式
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        if content_type.contains("text/event-stream") {
            // SSE 响应流：读取第一个事件
            parse_sse_response(response).await
        } else {
            // 普通 JSON 响应
            let text = response
                .text()
                .await
                .map_err(|e| AstrError::Network(format!("read MCP HTTP response: {}", e)))?;

            serde_json::from_str(&text).map_err(|e| AstrError::parse("parse MCP HTTP response", e))
        }
    }

    async fn send_notification(&self, notification: JsonRpcNotification) -> Result<()> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| AstrError::Internal("HTTP transport not started".into()))?;

        let body = serde_json::to_string(&notification)
            .map_err(|e| AstrError::parse("serialize JSON-RPC notification", e))?;

        let mut req_builder = client
            .post(&self.url)
            .header("Content-Type", "application/json")
            .body(body);

        for (key, value) in &self.headers {
            if let Ok(header_name) = reqwest::header::HeaderName::from_bytes(key.as_bytes()) {
                req_builder = req_builder.header(header_name, value.as_str());
            }
        }

        let response = req_builder
            .send()
            .await
            .map_err(|e| AstrError::http("MCP HTTP notification", e))?;

        if !response.status().is_success() {
            warn!(
                "MCP HTTP notification returned {} (notifications are fire-and-forget)",
                response.status()
            );
        }

        Ok(())
    }

    async fn close(&mut self) -> Result<()> {
        self.active.store(false, Ordering::SeqCst);
        self.client = None;
        info!("MCP Streamable HTTP transport closed: {}", self.url);
        Ok(())
    }

    fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst) && self.client.is_some()
    }

    fn transport_type(&self) -> &'static str {
        "http"
    }
}

/// 从 SSE 响应流中解析第一个完整 JSON-RPC 消息。
async fn parse_sse_response(response: reqwest::Response) -> Result<JsonRpcResponse> {
    let mut stream = response.bytes_stream();

    while let Some(chunk_result) = stream.next().await {
        let chunk =
            chunk_result.map_err(|e| AstrError::Network(format!("read SSE stream: {}", e)))?;
        let text = String::from_utf8_lossy(&chunk);

        // 解析 SSE 事件格式
        for line in text.lines() {
            if let Some(data) = line.strip_prefix("data: ") {
                let data = data.trim();
                if data.is_empty() || data == "[DONE]" {
                    continue;
                }
                if let Ok(response) = serde_json::from_str::<JsonRpcResponse>(data) {
                    return Ok(response);
                }
            }
        }
    }

    Err(AstrError::Network(
        "SSE stream ended without JSON-RPC response".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_transport_creation() {
        let transport = StreamableHttpTransport::new(
            "http://localhost:8080/mcp",
            vec![("Authorization".to_string(), "Bearer test".to_string())],
        );
        assert!(!transport.is_active());
        assert_eq!(transport.transport_type(), "http");
    }

    #[tokio::test]
    async fn test_start_sets_active() {
        let mut transport = StreamableHttpTransport::new("http://localhost:8080/mcp", Vec::new());
        transport.start().await.unwrap();
        assert!(transport.is_active());
    }

    #[tokio::test]
    async fn test_close_deactivates() {
        let mut transport = StreamableHttpTransport::new("http://localhost:8080/mcp", Vec::new());
        transport.start().await.unwrap();
        transport.close().await.unwrap();
        assert!(!transport.is_active());
    }

    #[tokio::test]
    async fn test_send_before_start_errors() {
        let transport = StreamableHttpTransport::new("http://localhost:8080/mcp", Vec::new());
        let request = JsonRpcRequest::new(1, "test");
        let result = transport.send_request(request).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_notification_before_start_errors() {
        let transport = StreamableHttpTransport::new("http://localhost:8080/mcp", Vec::new());
        let notification = JsonRpcNotification::new("test");
        let result = transport.send_notification(notification).await;
        assert!(result.is_err());
    }
}
