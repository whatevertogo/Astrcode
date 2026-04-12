//! # Mock 传输实现（测试用）
//!
//! 用于单元测试的可编程 mock 传输。
//! 支持预设响应序列、请求验证和可编程断连行为。

#[cfg(test)]
pub mod testsupport {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use astrcode_core::{AstrError, Result};
    use async_trait::async_trait;
    use tokio::sync::Mutex;

    use crate::{
        protocol::types::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse},
        transport::McpTransport,
    };

    /// Mock 传输：支持预设响应序列和可编程断连行为。
    pub struct MockTransport {
        /// 预设的响应队列。
        responses: Arc<Mutex<Vec<JsonRpcResponse>>>,
        /// 已发送的请求记录。
        sent_requests: Arc<Mutex<Vec<JsonRpcRequest>>>,
        /// 已发送的通知记录。
        sent_notifications: Arc<Mutex<Vec<JsonRpcNotification>>>,
        /// 是否活跃。
        active: Arc<AtomicBool>,
        /// 是否模拟断连（send_request 时返回错误）。
        disconnect_on_next: Arc<AtomicBool>,
    }

    impl Default for MockTransport {
        fn default() -> Self {
            Self::new()
        }
    }

    impl MockTransport {
        pub fn new() -> Self {
            Self {
                responses: Arc::new(Mutex::new(Vec::new())),
                sent_requests: Arc::new(Mutex::new(Vec::new())),
                sent_notifications: Arc::new(Mutex::new(Vec::new())),
                active: Arc::new(AtomicBool::new(false)),
                disconnect_on_next: Arc::new(AtomicBool::new(false)),
            }
        }

        /// 添加一个预设响应到队列。
        pub async fn add_response(&self, response: JsonRpcResponse) {
            self.responses.lock().await.push(response);
        }

        /// 设置下次请求时模拟断连。
        pub fn set_disconnect_on_next(&self, value: bool) {
            self.disconnect_on_next.store(value, Ordering::SeqCst);
        }

        /// 获取已发送的请求记录。
        pub async fn sent_requests(&self) -> Vec<JsonRpcRequest> {
            self.sent_requests.lock().await.clone()
        }

        /// 获取已发送的通知记录。
        pub async fn sent_notifications(&self) -> Vec<JsonRpcNotification> {
            self.sent_notifications.lock().await.clone()
        }
    }

    #[async_trait]
    impl McpTransport for MockTransport {
        async fn start(&mut self) -> Result<()> {
            self.active.store(true, Ordering::SeqCst);
            Ok(())
        }

        async fn send_request(&self, request: JsonRpcRequest) -> Result<JsonRpcResponse> {
            // 记录请求
            self.sent_requests.lock().await.push(request);

            // 检查是否模拟断连
            if self.disconnect_on_next.load(Ordering::SeqCst) {
                self.active.store(false, Ordering::SeqCst);
                return Err(AstrError::Network("mock transport disconnected".into()));
            }

            // 弹出预设响应
            let mut responses = self.responses.lock().await;
            responses
                .pop()
                .ok_or_else(|| AstrError::Internal("mock transport: no preset response".into()))
        }

        async fn send_notification(&self, notification: JsonRpcNotification) -> Result<()> {
            self.sent_notifications.lock().await.push(notification);
            Ok(())
        }

        async fn close(&mut self) -> Result<()> {
            self.active.store(false, Ordering::SeqCst);
            Ok(())
        }

        fn is_active(&self) -> bool {
            self.active.load(Ordering::SeqCst)
        }

        fn transport_type(&self) -> &'static str {
            "mock"
        }
    }

    /// 创建一个带有 initialize 成功响应的 mock 传输。
    pub async fn create_connected_mock() -> (Arc<Mutex<MockTransport>>, Vec<JsonRpcResponse>) {
        let mock = MockTransport::new();

        use serde_json::json;

        // initialize 成功响应
        let init_response = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(1)),
            result: Some(json!({
                "protocolVersion": "2025-03-26",
                "capabilities": {
                    "tools": { "listChanged": true },
                    "prompts": { "listChanged": false },
                    "resources": { "subscribe": false, "listChanged": false }
                },
                "serverInfo": { "name": "test-server", "version": "1.0.0" },
                "instructions": "Test server instructions"
            })),
            error: None,
        };

        mock.add_response(init_response).await;
        let responses = vec![];

        (Arc::new(Mutex::new(mock)), responses)
    }
}
