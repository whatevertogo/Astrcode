//! # MCP 工具桥接
//!
//! 将单个 MCP 工具转换为 Astrcode 的 `CapabilityInvoker`，
//! 负责 JSON-RPC 请求构造、响应解析和结果大小控制。

use std::{sync::Arc, time::Instant};

use astrcode_core::{
    AstrError, CapabilityContext, CapabilityExecutionResult, CapabilityInvoker, Result,
};
use astrcode_protocol::capability::{CapabilityDescriptor, CapabilityKind, SideEffectLevel};
use async_trait::async_trait;
use log::{info, warn};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use super::build_mcp_tool_name;
use crate::protocol::{
    McpClient,
    types::{McpContentBlock, McpToolAnnotations, McpToolResult},
};

/// MCP 工具桥接适配器。
///
/// 将单个 MCP 工具转换为 Astrcode 的 CapabilityInvoker，
/// 负责 JSON-RPC 请求构造、响应解析和结果大小控制。
pub struct McpToolBridge {
    /// 所属 MCP 服务器名称。
    server_name: String,
    /// MCP 服务器声明的原始工具名。
    tool_name: String,
    /// 全限定名称 `mcp__{server}__{tool}`。
    fully_qualified_name: String,
    /// 工具描述。
    description: String,
    /// JSON Schema 参数定义。
    input_schema: Value,
    /// 工具能力标注。
    annotations: McpToolAnnotations,
    /// MCP 协议客户端（共享引用）。
    client: Arc<Mutex<McpClient>>,
}

impl McpToolBridge {
    /// 创建 MCP 工具桥接。
    pub fn new(
        server_name: impl Into<String>,
        tool_name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
        annotations: Option<McpToolAnnotations>,
        client: Arc<Mutex<McpClient>>,
    ) -> Self {
        let server_name = server_name.into();
        let tool_name = tool_name.into();
        let fully_qualified_name = build_mcp_tool_name(&server_name, &tool_name);

        Self {
            server_name,
            tool_name,
            fully_qualified_name,
            description: description.into(),
            input_schema,
            annotations: annotations.unwrap_or_default(),
            client,
        }
    }

    /// 返回全限定工具名称。
    pub fn fully_qualified_name(&self) -> &str {
        &self.fully_qualified_name
    }

    /// 返回原始 MCP 工具名称。
    pub fn tool_name(&self) -> &str {
        &self.tool_name
    }

    /// 返回所属服务器名称。
    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    /// 构建 CapabilityDescriptor。
    fn build_descriptor(&self) -> CapabilityDescriptor {
        let mut builder =
            CapabilityDescriptor::builder(&self.fully_qualified_name, CapabilityKind::tool())
                .description(&self.description)
                .schema(self.input_schema.clone(), json!({ "type": "string" }))
                .tag("source:mcp"); // 标记为 MCP 来源，用于 prompt 分层和搜索

        // 从 annotations 映射能力属性
        if self.annotations.read_only_hint {
            builder = builder.concurrency_safe(true);
        }
        if self.annotations.destructive_hint {
            builder = builder.side_effect(SideEffectLevel::External);
        }
        if self.annotations.open_world_hint {
            builder =
                builder.permission_with_rationale("network", "MCP tool with openWorldHint: true");
        }

        builder.build().unwrap_or_else(|_| {
            // 构建失败时使用最小描述符
            CapabilityDescriptor::builder(&self.fully_qualified_name, CapabilityKind::tool())
                .description(&self.description)
                .schema(json!({"type":"object"}), json!({"type":"string"}))
                .tag("source:mcp")
                .build()
                .expect("minimal descriptor must build")
        })
    }

    /// 将 MCP 工具结果映射为 CapabilityExecutionResult。
    fn map_result(&self, mcp_result: McpToolResult, duration_ms: u64) -> CapabilityExecutionResult {
        if mcp_result.is_error {
            let error_text = mcp_result
                .content
                .iter()
                .filter_map(|block| match block {
                    McpContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");

            let mut result = CapabilityExecutionResult::failure(
                &self.fully_qualified_name,
                if error_text.is_empty() {
                    "MCP tool returned error without message".to_string()
                } else {
                    error_text
                },
                json!(null),
            );
            result.duration_ms = duration_ms;
            return result;
        }

        let output = map_content_blocks(&mcp_result.content);
        let mut result = CapabilityExecutionResult::ok(&self.fully_qualified_name, output);
        result.duration_ms = duration_ms;
        result
    }
}

/// 将 MCP 内容块列表映射为 JSON Value。
fn map_content_blocks(blocks: &[McpContentBlock]) -> Value {
    if blocks.is_empty() {
        return Value::Null;
    }

    let texts: Vec<&str> = blocks
        .iter()
        .filter_map(|block| match block {
            McpContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();

    // 只有文本时直接拼接
    if texts.len() == blocks.len() {
        if texts.len() == 1 {
            return Value::String(texts[0].to_string());
        }
        return Value::Array(
            texts
                .into_iter()
                .map(|t| Value::String(t.to_string()))
                .collect(),
        );
    }

    // 混合内容：逐块映射
    let mapped: Vec<Value> = blocks
        .iter()
        .map(|block| match block {
            McpContentBlock::Text { text } => Value::String(text.clone()),
            McpContentBlock::Image { data, mime_type } => json!({
                "type": "image",
                "data": data,
                "mimeType": mime_type,
            }),
            McpContentBlock::Resource { resource } => json!({
                "type": "resource",
                "uri": resource.uri,
                "mimeType": resource.mime_type,
                "text": resource.text,
            }),
        })
        .collect();

    Value::Array(mapped)
}

#[async_trait]
impl CapabilityInvoker for McpToolBridge {
    fn descriptor(&self) -> CapabilityDescriptor {
        self.build_descriptor()
    }

    async fn invoke(
        &self,
        payload: Value,
        ctx: &CapabilityContext,
    ) -> Result<CapabilityExecutionResult> {
        let start = Instant::now();
        info!(
            "MCP tool invoke: {} on server {}",
            self.tool_name, self.server_name
        );

        let client = self.client.lock().await;
        let result = client
            .call_tool(&self.tool_name, payload, ctx.cancel.clone())
            .await;

        let duration_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(mcp_result) => Ok(self.map_result(mcp_result, duration_ms)),
            Err(AstrError::Cancelled) => {
                warn!("MCP tool cancelled: {}", self.fully_qualified_name);
                Err(AstrError::Cancelled)
            },
            Err(e) => {
                warn!("MCP tool error: {} - {}", self.fully_qualified_name, e);
                let mut result = CapabilityExecutionResult::failure(
                    &self.fully_qualified_name,
                    e.to_string(),
                    json!(null),
                );
                result.duration_ms = duration_ms;
                Ok(result)
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::transport::mock::testsupport::*;

    #[test]
    fn test_build_mcp_tool_name() {
        assert_eq!(
            build_mcp_tool_name("github", "create_issue"),
            "mcp__github__create_issue"
        );
    }

    #[tokio::test]
    async fn test_bridge_descriptor() {
        let (mock_transport, _) = create_connected_mock().await;
        let client = McpClient::connect(mock_transport).await.unwrap();

        let bridge = McpToolBridge::new(
            "test-server",
            "read_file",
            "Read a file from the filesystem",
            json!({"type": "object", "properties": {"path": {"type": "string"}}}),
            Some(McpToolAnnotations {
                read_only_hint: true,
                destructive_hint: false,
                open_world_hint: false,
            }),
            Arc::new(Mutex::new(client)),
        );

        assert_eq!(bridge.fully_qualified_name(), "mcp__test-server__read_file");
        assert_eq!(bridge.tool_name(), "read_file");
        assert_eq!(bridge.server_name(), "test-server");

        let descriptor = bridge.descriptor();
        assert_eq!(descriptor.name, "mcp__test-server__read_file");
        assert!(descriptor.concurrency_safe);
    }

    #[test]
    fn test_map_content_blocks_text_only() {
        let blocks = vec![
            McpContentBlock::Text {
                text: "hello".to_string(),
            },
            McpContentBlock::Text {
                text: "world".to_string(),
            },
        ];
        let result = map_content_blocks(&blocks);
        assert_eq!(result, json!(["hello", "world"]));
    }

    #[test]
    fn test_map_content_blocks_empty() {
        let result = map_content_blocks(&[]);
        assert!(result.is_null());
    }

    #[test]
    fn test_map_content_blocks_single_text() {
        let blocks = vec![McpContentBlock::Text {
            text: "hello".to_string(),
        }];
        let result = map_content_blocks(&blocks);
        assert_eq!(result, json!("hello"));
    }

    #[test]
    fn test_annotations_default() {
        let ann = McpToolAnnotations::default();
        assert!(!ann.read_only_hint);
        assert!(!ann.destructive_hint);
        assert!(!ann.open_world_hint);
    }
}
