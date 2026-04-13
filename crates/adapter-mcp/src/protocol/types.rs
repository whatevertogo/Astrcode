//! # MCP 协议 DTO 类型
//!
//! 定义 MCP 协议中使用的所有数据传输对象，包括：
//! - JSON-RPC 2.0 消息类型（Request、Response、Notification、Error）
//! - MCP 工具、Prompt、资源信息
//! - 服务器能力声明
//! - 握手参数和结果

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ========== JSON-RPC 2.0 消息类型 ==========

fn default_jsonrpc() -> String {
    "2.0".to_string()
}

/// JSON-RPC 2.0 请求。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    #[serde(default = "default_jsonrpc")]
    pub jsonrpc: String,
    pub id: Value,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcRequest {
    pub fn new(id: impl Into<Value>, method: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id: id.into(),
            method: method.into(),
            params: None,
        }
    }

    pub fn with_params(mut self, params: Value) -> Self {
        self.params = Some(params);
        self
    }
}

/// JSON-RPC 2.0 响应。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    #[serde(default = "default_jsonrpc")]
    pub jsonrpc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

/// JSON-RPC 2.0 错误。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// JSON-RPC 2.0 通知（无 id，无响应）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    #[serde(default = "default_jsonrpc")]
    pub jsonrpc: String,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcNotification {
    pub fn new(method: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method: method.into(),
            params: None,
        }
    }

    pub fn with_params(mut self, params: Value) -> Self {
        self.params = Some(params);
        self
    }
}

// ========== MCP 工具类型 ==========

/// MCP 服务器声明的工具信息。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolInfo {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<McpToolAnnotations>,
}

/// MCP 工具的能力标注。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolAnnotations {
    #[serde(default)]
    pub read_only_hint: bool,
    #[serde(default)]
    pub destructive_hint: bool,
    #[serde(default)]
    pub open_world_hint: bool,
}

/// MCP 工具调用结果。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolResult {
    pub content: Vec<McpContentBlock>,
    #[serde(default)]
    pub is_error: bool,
}

/// MCP 内容块（工具调用返回的内容类型）。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum McpContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image {
        data: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
    },
    #[serde(rename = "resource")]
    Resource { resource: McpResourceContent },
}

/// MCP 资源内容。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpResourceContent {
    pub uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blob: Option<String>,
}

// ========== MCP Prompt 类型 ==========

/// MCP 服务器声明的 prompt 模板信息。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpPromptInfo {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub arguments: Vec<McpPromptArgument>,
}

/// Prompt 模板参数定义。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpPromptArgument {
    pub name: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Prompt 获取结果中的消息。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpPromptMessage {
    pub role: String,
    pub content: McpContentBlock,
}

/// `prompts/get` 返回结果。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpPromptResult {
    pub messages: Vec<McpPromptMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

// ========== MCP 资源类型 ==========

/// MCP 服务器声明的资源信息。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpResourceInfo {
    pub uri: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

// ========== MCP 握手类型 ==========

/// MCP 客户端信息。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpClientInfo {
    pub name: String,
    pub version: String,
}

/// MCP 服务器信息。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerInfo {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// MCP 客户端能力声明。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct McpClientCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roots: Option<McpRootsCapability>,
}

/// 客户端 roots 能力（支持文件系统根列表）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpRootsCapability {
    #[serde(default)]
    pub list_changed: bool,
}

/// MCP initialize 请求参数。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    pub protocol_version: String,
    pub capabilities: McpClientCapabilities,
    pub client_info: McpClientInfo,
}

/// MCP initialize 响应。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub protocol_version: String,
    pub capabilities: McpServerCapabilities,
    pub server_info: McpServerInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

/// MCP 服务器能力声明。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct McpServerCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolsCapability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompts: Option<PromptsCapability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourcesCapability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logging: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Value>,
}

/// 服务器 tools 能力。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsCapability {
    #[serde(default)]
    pub list_changed: bool,
}

/// 服务器 prompts 能力。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptsCapability {
    #[serde(default)]
    pub list_changed: bool,
}

/// 服务器 resources 能力。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourcesCapability {
    #[serde(default)]
    pub subscribe: bool,
    #[serde(default)]
    pub list_changed: bool,
}

/// list_changed 通知的类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum McpListKind {
    Tools,
    Prompts,
    Resources,
}

// ========== MCP 协议常量 ==========

/// MCP 协议版本（最新稳定版）。
pub const MCP_PROTOCOL_VERSION: &str = "2025-03-26";

/// MCP 最低兼容版本。
pub const MCP_MIN_PROTOCOL_VERSION: &str = "2024-11-05";

/// 默认工具调用超时（秒）。
pub const DEFAULT_TOOL_TIMEOUT_SECS: u64 = 120;

/// 默认握手超时（秒）。
pub const DEFAULT_INIT_TIMEOUT_SECS: u64 = 30;

/// 取消后强制断开等待超时（秒）。
pub const CANCEL_FORCE_DISCONNECT_TIMEOUT_SECS: u64 = 30;

/// 本地服务器最大并发连接数。
pub const MAX_LOCAL_CONCURRENCY: usize = 3;

/// 远程服务器最大并发连接数。
pub const MAX_REMOTE_CONCURRENCY: usize = 10;

/// 最大重连尝试次数。
pub const MAX_RECONNECT_ATTEMPTS: u32 = 5;

/// 重连初始退避时间（毫秒）。
pub const RECONNECT_INITIAL_BACKOFF_MS: u64 = 1000;

/// 重连最大退避时间（毫秒）。
pub const RECONNECT_MAX_BACKOFF_MS: u64 = 30000;

/// 重连退避倍数。
pub const RECONNECT_MULTIPLIER: f64 = 2.0;

/// 客户端名称标识。
pub const CLIENT_NAME: &str = "astrcode";

/// 客户端版本。
pub const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// OAuth 配置。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpOAuthConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callback_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_server_metadata_url: Option<String>,
}

/// 额外 HTTP headers（用于认证）。
pub type HttpHeaders = HashMap<String, String>;

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{McpResourceContent, McpToolInfo, McpToolResult};

    #[test]
    fn deserializes_tool_info_input_schema_from_camel_case_field() {
        let tool: McpToolInfo = serde_json::from_value(json!({
            "name": "web_search_prime",
            "description": "Search web information",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "search_query": { "type": "string" }
                },
                "required": ["search_query"]
            }
        }))
        .expect("tool info should deserialize");

        assert_eq!(tool.name, "web_search_prime");
        assert_eq!(
            tool.input_schema.expect("input schema should exist")["required"],
            json!(["search_query"])
        );
    }

    #[test]
    fn deserializes_tool_result_error_flag_from_camel_case_field() {
        let result: McpToolResult = serde_json::from_value(json!({
            "content": [],
            "isError": true
        }))
        .expect("tool result should deserialize");

        assert!(result.is_error);
    }

    #[test]
    fn deserializes_resource_content_mime_type_from_camel_case_field() {
        let resource: McpResourceContent = serde_json::from_value(json!({
            "uri": "file:///tmp/demo.txt",
            "mimeType": "text/plain",
            "text": "hello"
        }))
        .expect("resource content should deserialize");

        assert_eq!(resource.mime_type.as_deref(), Some("text/plain"));
    }
}
