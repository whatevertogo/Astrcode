//! # SSE 传输实现（兼容回退）
//!
//! MCP 旧版远程传输模式：通过 SSE 连接接收服务端消息，
//! 通过 HTTP POST 发送客户端消息。
//! 仅在新版 Streamable HTTP 不可用时使用。

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use astrcode_core::{AstrError, Result};
use async_trait::async_trait;
use log::{debug, info, warn};
use reqwest::Client;

use super::McpTransport;
use crate::protocol::types::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};

/// SSE 传输（兼容回退）。
///
/// 通过 SSE GET 请求建立长连接接收服务端推送消息，
/// 通过 HTTP POST 发送客户端请求到独立的 endpoint。
pub struct SseTransport {
    /// SSE 连接 URL（GET 请求建立连接）。
    sse_url: String,
    /// 消息发送 URL（POST 请求发送消息）。
    message_url: String,
    /// 静态 HTTP headers。
    headers: Vec<(String, String)>,
    /// reqwest 客户端。
    client: Option<Client>,
    /// 传输是否活跃。
    active: Arc<AtomicBool>,
}

impl SseTransport {
    /// 创建 SSE 传输。
    ///
    /// `sse_url` 为 SSE 连接端点，消息 URL 默认相同。
    pub fn new(sse_url: impl Into<String>, headers: Vec<(String, String)>) -> Self {
        let sse_url = sse_url.into();
        let message_url = sse_url.clone();
        Self {
            sse_url,
            message_url,
            headers,
            client: None,
            active: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[async_trait]
impl McpTransport for SseTransport {
    async fn start(&mut self) -> Result<()> {
        self.client = Some(Client::new());
        self.active.store(true, Ordering::SeqCst);

        info!("MCP SSE transport ready: {}", self.sse_url);
        Ok(())
    }

    async fn send_request(&self, request: JsonRpcRequest) -> Result<JsonRpcResponse> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| AstrError::Internal("SSE transport not started".into()))?;

        if !self.active.load(Ordering::SeqCst) {
            return Err(AstrError::Network("SSE transport not active".into()));
        }

        let body = serde_json::to_string(&request)
            .map_err(|e| AstrError::parse("serialize JSON-RPC request", e))?;

        debug!("MCP SSE POST to {}: {} bytes", self.message_url, body.len());

        let mut req_builder = client
            .post(&self.message_url)
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
            .map_err(|e| AstrError::http("MCP SSE request", e))?;

        let status = response.status();
        if !status.is_success() {
            let status_code = status.as_u16();
            let body_text = response.text().await.unwrap_or_default();
            return Err(AstrError::Network(format!(
                "MCP SSE server returned HTTP {}: {}",
                status_code, body_text
            )));
        }

        // SSE 模式下 POST 响应通常是 202 Accepted
        // 实际的 JSON-RPC 响应通过 SSE 流返回
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        if content_type.contains("application/json") {
            let text = response
                .text()
                .await
                .map_err(|e| AstrError::Network(format!("read MCP SSE response: {}", e)))?;
            serde_json::from_str(&text).map_err(|e| AstrError::parse("parse MCP SSE response", e))
        } else {
            // 202 Accepted 或空响应——实际响应会通过 SSE 流异步到达
            warn!("SSE transport: POST returned non-JSON, actual response expected via SSE stream");
            Err(AstrError::Network(
                "SSE transport requires async stream processing for responses".into(),
            ))
        }
    }

    async fn send_notification(&self, notification: JsonRpcNotification) -> Result<()> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| AstrError::Internal("SSE transport not started".into()))?;

        let body = serde_json::to_string(&notification)
            .map_err(|e| AstrError::parse("serialize JSON-RPC notification", e))?;

        let mut req_builder = client
            .post(&self.message_url)
            .header("Content-Type", "application/json")
            .body(body);

        for (key, value) in &self.headers {
            if let Ok(header_name) = reqwest::header::HeaderName::from_bytes(key.as_bytes()) {
                req_builder = req_builder.header(header_name, value.as_str());
            }
        }

        let _ = req_builder.send().await;
        Ok(())
    }

    async fn close(&mut self) -> Result<()> {
        self.active.store(false, Ordering::SeqCst);
        self.client = None;
        info!("MCP SSE transport closed: {}", self.sse_url);
        Ok(())
    }

    fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst) && self.client.is_some()
    }

    fn transport_type(&self) -> &'static str {
        "sse"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sse_transport_creation() {
        let transport = SseTransport::new("http://localhost:8080/sse", Vec::new());
        assert!(!transport.is_active());
        assert_eq!(transport.transport_type(), "sse");
        assert_eq!(transport.sse_url, "http://localhost:8080/sse");
    }

    #[tokio::test]
    async fn test_start_sets_active() {
        let mut transport = SseTransport::new("http://localhost:8080/sse", Vec::new());
        transport.start().await.unwrap();
        assert!(transport.is_active());
    }

    #[tokio::test]
    async fn test_close_deactivates() {
        let mut transport = SseTransport::new("http://localhost:8080/sse", Vec::new());
        transport.start().await.unwrap();
        transport.close().await.unwrap();
        assert!(!transport.is_active());
    }

    #[tokio::test]
    async fn test_send_before_start_errors() {
        let transport = SseTransport::new("http://localhost:8080/sse", Vec::new());
        let request = JsonRpcRequest::new(1, "test");
        let result = transport.send_request(request).await;
        assert!(result.is_err());
    }
}
