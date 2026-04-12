//! # MCP Prompt 模板桥接
//!
//! 将 `prompts/list` 暴露的模板转换为可调用 capability，
//! 避免把模板目录错误地常驻注入 system prompt。

use std::{sync::Arc, time::Instant};

use astrcode_core::{
    CapabilityContext, CapabilityExecutionResult, CapabilityInvoker, CapabilityKind,
    CapabilitySpec, Result,
};
use async_trait::async_trait;
use log::warn;
use serde_json::{Map, Value, json};
use tokio::sync::Mutex;

use super::build_mcp_tool_name;
use crate::protocol::{
    McpClient,
    types::{McpContentBlock, McpPromptInfo},
};

/// MCP prompt 模板调用桥接。
pub struct McpPromptBridge {
    server_name: String,
    prompt_name: String,
    fully_qualified_name: String,
    description: String,
    input_schema: Value,
    client: Arc<Mutex<McpClient>>,
}

impl McpPromptBridge {
    pub fn new(
        server_name: impl Into<String>,
        prompt_info: &McpPromptInfo,
        client: Arc<Mutex<McpClient>>,
    ) -> Self {
        let server_name = server_name.into();
        let prompt_name = prompt_info.name.clone();
        Self {
            fully_qualified_name: build_mcp_tool_name(&server_name, &prompt_name),
            server_name,
            prompt_name,
            description: prompt_info.description.clone().unwrap_or_default(),
            input_schema: prompt_arguments_to_schema(prompt_info),
            client,
        }
    }
}

fn prompt_arguments_to_schema(prompt_info: &McpPromptInfo) -> Value {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for argument in &prompt_info.arguments {
        properties.insert(
            argument.name.clone(),
            json!({
                "type": "string",
                "description": argument.description.clone().unwrap_or_default(),
            }),
        );
        if argument.required {
            required.push(argument.name.clone());
        }
    }

    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

#[cfg(test)]
fn map_prompt_blocks(blocks: &[McpContentBlock]) -> Value {
    if blocks.len() == 1 {
        return map_prompt_block(&blocks[0]);
    }
    Value::Array(blocks.iter().map(map_prompt_block).collect())
}

fn map_prompt_block(block: &McpContentBlock) -> Value {
    match block {
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
            "blob": resource.blob,
        }),
    }
}

#[async_trait]
impl CapabilityInvoker for McpPromptBridge {
    fn capability_spec(&self) -> CapabilitySpec {
        CapabilitySpec::builder(self.fully_qualified_name.clone(), CapabilityKind::Tool)
            .description(if self.description.is_empty() {
                format!(
                    "调用 MCP prompt 模板 {}/{}",
                    self.server_name, self.prompt_name
                )
            } else {
                self.description.clone()
            })
            .schema(self.input_schema.clone(), json!({"type": "object"}))
            .concurrency_safe(true)
            .tags(["source:mcp", "mcp:prompt"])
            .build()
            .expect("mcp prompt capability spec must build")
    }

    async fn invoke(
        &self,
        payload: Value,
        _ctx: &CapabilityContext,
    ) -> Result<CapabilityExecutionResult> {
        let start = Instant::now();
        let arguments = if payload.is_null() {
            None
        } else {
            Some(payload)
        };
        let client = self.client.lock().await;
        match client.get_prompt(&self.prompt_name, arguments).await {
            Ok(result) => {
                let messages = result
                    .messages
                    .into_iter()
                    .map(|message| {
                        json!({
                            "role": message.role,
                            "content": map_prompt_block(&message.content),
                        })
                    })
                    .collect::<Vec<_>>();
                let mut exec_result = CapabilityExecutionResult::ok(
                    &self.fully_qualified_name,
                    json!({
                        "description": result.description,
                        "messages": messages,
                    }),
                );
                exec_result.duration_ms = start.elapsed().as_millis() as u64;
                Ok(exec_result)
            },
            Err(error) => {
                warn!(
                    "MCP prompt invoke failed: {} on {}: {}",
                    self.prompt_name, self.server_name, error
                );
                let mut exec_result = CapabilityExecutionResult::failure(
                    &self.fully_qualified_name,
                    error.to_string(),
                    json!(null),
                );
                exec_result.duration_ms = start.elapsed().as_millis() as u64;
                Ok(exec_result)
            },
        }
    }
}

impl McpPromptBridge {
    pub fn capability_name(&self) -> &str {
        &self.fully_qualified_name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        protocol::types::McpPromptArgument, transport::mock::testsupport::create_connected_mock,
    };

    #[tokio::test]
    async fn builds_prompt_capability_spec_from_arguments() {
        let (transport, _) = create_connected_mock().await;
        let client = Arc::new(Mutex::new(
            McpClient::connect(transport).await.expect("client"),
        ));
        let bridge = McpPromptBridge::new(
            "srv",
            &McpPromptInfo {
                name: "review".to_string(),
                description: Some("Review code".to_string()),
                arguments: vec![McpPromptArgument {
                    name: "path".to_string(),
                    required: true,
                    description: Some("Path".to_string()),
                }],
            },
            client,
        );

        let spec = bridge.capability_spec();
        assert_eq!(spec.name.as_str(), "mcp__srv__review");
        assert!(spec.tags.iter().any(|tag| tag == "mcp:prompt"));
    }

    #[test]
    fn maps_prompt_arguments_into_json_schema() {
        let schema = prompt_arguments_to_schema(&McpPromptInfo {
            name: "review".to_string(),
            description: None,
            arguments: vec![McpPromptArgument {
                name: "path".to_string(),
                required: true,
                description: Some("Path".to_string()),
            }],
        });
        assert_eq!(schema["required"], json!(["path"]));
        assert_eq!(schema["properties"]["path"]["type"], json!("string"));
    }

    #[test]
    fn maps_mixed_prompt_blocks() {
        let mapped = map_prompt_blocks(&[
            McpContentBlock::Text {
                text: "hello".to_string(),
            },
            McpContentBlock::Image {
                data: "data".to_string(),
                mime_type: "image/png".to_string(),
            },
        ]);
        assert!(mapped.is_array());
    }
}
