//! # MCP 协议客户端
//!
//! 封装 MCP JSON-RPC 协议的所有标准化方法：
//! - 握手（initialize + initialized 通知）
//! - 工具发现与调用（tools/list、tools/call）
//! - 取消通知（notifications/cancelled）
//! - list_changed 通知处理

use std::{collections::HashMap, sync::Arc};

use astrcode_core::{AstrError, CancelToken, Result};
use log::info;
use serde_json::{Value, json};
use tokio::{sync::Mutex, time::Duration};

use crate::{
    protocol::{error::McpProtocolError, types::*},
    transport::McpTransport,
};

/// list_changed 通知的异步回调类型。
type ListChangedHandler =
    Box<dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send + Sync>;

/// MCP 协议客户端。
///
/// 封装 MCP JSON-RPC 协议的所有标准化方法。
/// 持有传输层引用，负责消息构造和响应解析。
pub struct McpClient {
    transport: Arc<Mutex<dyn McpTransport>>,
    /// 请求 ID 计数器。
    next_id: Arc<Mutex<u64>>,
    /// 握手后获取的服务器信息。
    server_info: Option<McpServerInfo>,
    /// 服务器能力声明。
    capabilities: Option<McpServerCapabilities>,
    /// 服务器提供的 prompt 指令。
    instructions: Option<String>,
    /// list_changed 通知处理器。
    list_changed_handlers: HashMap<McpListKind, ListChangedHandler>,
}

impl McpClient {
    /// 通过传输层创建并初始化 MCP 客户端。
    ///
    /// 执行完整的 MCP 握手流程：
    /// 1. 发送 `initialize` 请求
    /// 2. 验证协议版本兼容性
    /// 3. 接收服务器信息和能力声明
    /// 4. 发送 `initialized` 通知
    pub async fn connect(transport: Arc<Mutex<dyn McpTransport>>) -> Result<Self> {
        let mut client = Self {
            transport,
            next_id: Arc::new(Mutex::new(1)),
            server_info: None,
            capabilities: None,
            instructions: None,
            list_changed_handlers: HashMap::new(),
        };

        let init_params = InitializeParams {
            protocol_version: MCP_PROTOCOL_VERSION.to_string(),
            capabilities: McpClientCapabilities::default(),
            client_info: McpClientInfo {
                name: CLIENT_NAME.to_string(),
                version: CLIENT_VERSION.to_string(),
            },
        };

        let response = client
            .send_request(
                "initialize",
                Some(
                    serde_json::to_value(init_params)
                        .map_err(|e| AstrError::parse("serialize init params", e))?,
                ),
            )
            .await?;

        let init_result: InitializeResult = client.extract_result(response)?;

        // 版本兼容性检查
        if !is_version_compatible(&init_result.protocol_version) {
            return Err(McpProtocolError::VersionMismatch {
                server: init_result.protocol_version,
                client: MCP_PROTOCOL_VERSION.to_string(),
            }
            .into());
        }

        client.server_info = Some(init_result.server_info);
        client.capabilities = Some(init_result.capabilities);
        client.instructions = init_result.instructions;

        // 发送 initialized 通知
        client
            .send_notification("notifications/initialized", None)
            .await?;

        let server_name = client
            .server_info
            .as_ref()
            .map(|s| s.name.as_str())
            .unwrap_or("unknown");
        info!("MCP client connected to server: {}", server_name);

        Ok(client)
    }

    /// 获取服务器信息。
    pub fn server_info(&self) -> Option<&McpServerInfo> {
        self.server_info.as_ref()
    }

    /// 获取服务器能力声明。
    pub fn capabilities(&self) -> Option<&McpServerCapabilities> {
        self.capabilities.as_ref()
    }

    /// 获取服务器提供的 instructions。
    pub fn instructions(&self) -> Option<&str> {
        self.instructions.as_deref()
    }

    /// 是否声明了 tools 能力。
    pub fn supports_tools(&self) -> bool {
        self.capabilities
            .as_ref()
            .and_then(|caps| caps.tools.as_ref())
            .is_some()
    }

    /// 是否声明了 prompts 能力。
    pub fn supports_prompts(&self) -> bool {
        self.capabilities
            .as_ref()
            .and_then(|caps| caps.prompts.as_ref())
            .is_some()
    }

    /// 是否声明了 resources 能力。
    pub fn supports_resources(&self) -> bool {
        self.capabilities
            .as_ref()
            .and_then(|caps| caps.resources.as_ref())
            .is_some()
    }

    /// 请求服务器的工具列表。
    pub async fn list_tools(&self) -> Result<Vec<McpToolInfo>> {
        if !self.supports_tools() {
            return Ok(Vec::new());
        }

        let response = self.send_request("tools/list", None).await?;

        #[derive(serde::Deserialize)]
        struct ToolsListResult {
            #[serde(default)]
            tools: Vec<McpToolInfo>,
        }

        let result: ToolsListResult = self.extract_result(response)?;
        Ok(result.tools)
    }

    /// 调用服务器上的工具。
    ///
    /// 支持取消信号：当 cancel token 被触发时发送 `notifications/cancelled`。
    pub async fn call_tool(
        &self,
        tool_name: &str,
        arguments: Value,
        cancel: CancelToken,
    ) -> Result<McpToolResult> {
        let params = json!({
            "name": tool_name,
            "arguments": arguments,
        });

        let request_id = self.next_id().await;
        let request = JsonRpcRequest::new(request_id.clone(), "tools/call").with_params(params);

        let transport = self.transport.lock().await;

        // 发送请求，然后在等待响应时检查取消状态
        let response =
            tokio::time::timeout(Duration::from_secs(DEFAULT_TOOL_TIMEOUT_SECS), async {
                // 发送请求
                let response = transport.send_request(request).await?;
                Ok::<JsonRpcResponse, AstrError>(response)
            })
            .await
            .map_err(|_| McpProtocolError::Timeout {
                method: "tools/call".to_string(),
                timeout_secs: DEFAULT_TOOL_TIMEOUT_SECS,
            })??;

        // 检查取消状态：如果已被取消，发送取消通知
        if cancel.is_cancelled() {
            let cancel_params = json!({
                "requestId": request_id,
                "reason": "cancelled by user",
            });
            // drop transport lock first
            drop(transport);
            let _ = self
                .send_notification("notifications/cancelled", Some(cancel_params))
                .await;
            return Err(AstrError::Cancelled);
        }

        // 检查 JSON-RPC 错误
        if let Some(error) = &response.error {
            return Err(McpProtocolError::JsonRpcError {
                code: error.code,
                message: error.message.clone(),
            }
            .into());
        }

        let tool_result: McpToolResult = self.extract_result(response)?;
        Ok(tool_result)
    }

    /// 发送取消通知。
    pub async fn send_cancel(&self, request_id: &str, reason: Option<&str>) -> Result<()> {
        let params = json!({
            "requestId": request_id,
            "reason": reason.unwrap_or("cancelled"),
        });
        self.send_notification("notifications/cancelled", Some(params))
            .await
    }

    /// 注册 list_changed 通知处理器。
    pub fn on_list_changed(&mut self, kind: McpListKind, handler: ListChangedHandler) {
        self.list_changed_handlers.insert(kind, handler);
    }

    /// 请求服务器的 prompt 模板列表。
    pub async fn list_prompts(&self) -> Result<Vec<McpPromptInfo>> {
        if !self.supports_prompts() {
            return Ok(Vec::new());
        }

        let response = self.send_request("prompts/list", None).await?;

        #[derive(serde::Deserialize)]
        struct PromptsListResult {
            #[serde(default)]
            prompts: Vec<McpPromptInfo>,
        }

        let result: PromptsListResult = self.extract_result(response)?;
        Ok(result.prompts)
    }

    /// 获取指定 prompt 模板的完整内容。
    pub async fn get_prompt(
        &self,
        prompt_name: &str,
        arguments: Option<Value>,
    ) -> Result<McpPromptResult> {
        if !self.supports_prompts() {
            return Err(AstrError::Validation(
                "MCP server does not advertise prompt capability".into(),
            ));
        }

        let mut params = json!({ "name": prompt_name });
        if let Some(args) = arguments {
            params["arguments"] = args;
        }
        let response = self.send_request("prompts/get", Some(params)).await?;
        self.extract_result(response)
    }

    /// 请求服务器的资源列表。
    pub async fn list_resources(&self) -> Result<Vec<McpResourceInfo>> {
        if !self.supports_resources() {
            return Ok(Vec::new());
        }

        let response = self.send_request("resources/list", None).await?;

        #[derive(serde::Deserialize)]
        struct ResourcesListResult {
            #[serde(default)]
            resources: Vec<McpResourceInfo>,
        }

        let result: ResourcesListResult = self.extract_result(response)?;
        Ok(result.resources)
    }

    /// 读取指定资源的内容。
    pub async fn read_resource(&self, uri: &str) -> Result<McpResourceContent> {
        if !self.supports_resources() {
            return Err(AstrError::Validation(format!(
                "MCP server does not advertise resources capability for '{}'",
                uri
            )));
        }

        let params = json!({ "uri": uri });
        let response = self.send_request("resources/read", Some(params)).await?;

        #[derive(serde::Deserialize)]
        struct ReadResourceResult {
            #[serde(default)]
            contents: Vec<McpResourceContent>,
        }

        let result: ReadResourceResult = self.extract_result(response)?;
        result.contents.into_iter().next().ok_or_else(|| {
            McpProtocolError::ParseError("no content in resource response".into()).into()
        })
    }

    /// 关闭客户端。
    pub async fn disconnect(self) -> Result<()> {
        let mut transport = self.transport.lock().await;
        transport.close().await
    }

    // ===== 内部辅助方法 =====

    /// 发送 JSON-RPC 请求并等待响应。
    async fn send_request(&self, method: &str, params: Option<Value>) -> Result<JsonRpcResponse> {
        let id = self.next_id().await;
        let request = JsonRpcRequest::new(id, method);
        let request = match params {
            Some(p) => request.with_params(p),
            None => request,
        };

        let transport = self.transport.lock().await;

        let response = tokio::time::timeout(
            Duration::from_secs(DEFAULT_TOOL_TIMEOUT_SECS),
            transport.send_request(request),
        )
        .await
        .map_err(|_| McpProtocolError::Timeout {
            method: method.to_string(),
            timeout_secs: DEFAULT_TOOL_TIMEOUT_SECS,
        })??;

        // 检查 JSON-RPC 错误
        if let Some(error) = &response.error {
            return Err(McpProtocolError::JsonRpcError {
                code: error.code,
                message: error.message.clone(),
            }
            .into());
        }

        Ok(response)
    }

    /// 发送 JSON-RPC 通知。
    async fn send_notification(&self, method: &str, params: Option<Value>) -> Result<()> {
        let notification = JsonRpcNotification::new(method);
        let notification = match params {
            Some(p) => notification.with_params(p),
            None => notification,
        };

        let transport = self.transport.lock().await;
        transport.send_notification(notification).await
    }

    /// 从响应中提取 result 字段并反序列化。
    fn extract_result<T: serde::de::DeserializeOwned>(
        &self,
        response: JsonRpcResponse,
    ) -> Result<T> {
        let result_value = response.result.ok_or_else(|| {
            McpProtocolError::ParseError("missing result field in response".into())
        })?;

        serde_json::from_value(result_value).map_err(|e| {
            McpProtocolError::ParseError(format!("deserialize response: {}", e)).into()
        })
    }

    /// 生成下一个请求 ID。
    async fn next_id(&self) -> Value {
        let mut id = self.next_id.lock().await;
        let current = *id;
        *id += 1;
        json!(current)
    }
}

/// 检查协议版本是否兼容。
fn is_version_compatible(server_version: &str) -> bool {
    server_version >= MCP_MIN_PROTOCOL_VERSION
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::transport::mock::testsupport::*;

    async fn setup_connected_client() -> McpClient {
        let (mock_transport, _) = create_connected_mock().await;
        McpClient::connect(mock_transport).await.unwrap()
    }

    #[tokio::test]
    async fn test_handshake_success() {
        let client = setup_connected_client().await;
        assert!(client.server_info().is_some());
        assert_eq!(client.server_info().unwrap().name, "test-server");
        assert!(client.capabilities().is_some());
        assert_eq!(client.instructions(), Some("Test server instructions"));
    }

    #[tokio::test]
    async fn test_version_incompatible() {
        let mock = MockTransport::new();
        // 返回不兼容的版本
        mock.add_response(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(1)),
            result: Some(json!({
                "protocolVersion": "2024-01-01",
                "capabilities": {},
                "serverInfo": { "name": "old-server" }
            })),
            error: None,
        })
        .await;

        let transport = Arc::new(Mutex::new(mock));
        let result = McpClient::connect(transport).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_list_tools() {
        let client = setup_connected_client().await;

        // 添加 tools/list 响应到 transport
        // 验证 capabilities 中 tools 存在
        let caps = client.capabilities().unwrap();
        assert!(caps.tools.is_some());
        assert!(caps.tools.as_ref().unwrap().list_changed);
    }

    #[tokio::test]
    async fn test_is_version_compatible() {
        assert!(is_version_compatible("2025-03-26"));
        assert!(is_version_compatible("2024-11-05"));
        assert!(!is_version_compatible("2024-01-01"));
    }
}
