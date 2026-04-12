//! # 外部工具搜索工具
//!
//! 为模型按需展开 MCP / plugin 工具 schema，避免外部工具在 system prompt 中
//! 全量铺开。

use std::sync::Arc;

use astrcode_core::{
    Result, Tool, ToolCapabilityMetadata, ToolContext, ToolDefinition, ToolExecutionResult,
    ToolPromptMetadata,
};
use astrcode_protocol::capability::SideEffectLevel;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use crate::external_tool_catalog::ExternalToolCatalog;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToolSearchArgs {
    #[serde(default)]
    query: String,
    #[serde(default)]
    limit: Option<usize>,
}

pub(crate) struct ToolSearchTool {
    catalog: Arc<ExternalToolCatalog>,
}

impl ToolSearchTool {
    pub(crate) fn new(catalog: Arc<ExternalToolCatalog>) -> Self {
        Self { catalog }
    }
}

#[async_trait]
impl Tool for ToolSearchTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "tool_search".to_string(),
            description: "Search MCP and plugin tools, returning their full schema on demand."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Substring query matched against tool name, description, and tags"
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 50,
                        "description": "Maximum results to return (default 10)"
                    }
                },
                "additionalProperties": false
            }),
        }
    }

    fn capability_metadata(&self) -> ToolCapabilityMetadata {
        ToolCapabilityMetadata::builtin()
            .side_effect(SideEffectLevel::None)
            .concurrency_safe(true)
            .prompt(ToolPromptMetadata::new(
                "Search external MCP/plugin tools and fetch their full schema only when needed.",
                "Use `tool_search` before calling an MCP or plugin tool when the prompt summary \
                 only gives you a name/description and you still need the concrete JSON schema.",
            ))
    }

    async fn execute(
        &self,
        tool_call_id: String,
        input: serde_json::Value,
        _ctx: &ToolContext,
    ) -> Result<ToolExecutionResult> {
        let args = serde_json::from_value::<ToolSearchArgs>(input).unwrap_or(ToolSearchArgs {
            query: String::new(),
            limit: None,
        });
        let limit = args.limit.unwrap_or(10).clamp(1, 50);
        let results = self.catalog.search(&args.query, limit);
        let payload = results
            .into_iter()
            .map(|descriptor| {
                let source = descriptor
                    .tags
                    .iter()
                    .find_map(|tag| tag.strip_prefix("source:"))
                    .unwrap_or("external")
                    .to_string();
                json!({
                    "name": descriptor.name,
                    "description": descriptor.description,
                    "source": source,
                    "tags": descriptor.tags,
                    "inputSchema": descriptor.input_schema,
                })
            })
            .collect::<Vec<_>>();

        Ok(ToolExecutionResult {
            tool_call_id,
            tool_name: "tool_search".to_string(),
            ok: true,
            output: serde_json::to_string(&payload)
                .expect("tool_search result serialization should not fail"),
            error: None,
            metadata: Some(json!({
                "returned": payload.len(),
                "query": args.query,
            })),
            duration_ms: 0,
            truncated: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{CancelToken, ToolContext};
    use astrcode_protocol::capability::{CapabilityDescriptor, CapabilityKind};
    use serde_json::json;

    use super::*;

    fn tool_context() -> ToolContext {
        ToolContext::new(
            "session".to_string(),
            std::env::temp_dir(),
            CancelToken::new(),
        )
    }

    fn descriptor(name: &str, tag: &str) -> CapabilityDescriptor {
        CapabilityDescriptor::builder(name, CapabilityKind::tool())
            .description("demo tool")
            .schema(json!({"type": "object"}), json!({"type": "object"}))
            .tag(tag)
            .build()
            .expect("descriptor should build")
    }

    #[tokio::test]
    async fn returns_full_schema_for_external_tools() {
        let catalog = Arc::new(ExternalToolCatalog::default());
        catalog.replace_from_descriptors(&[descriptor("mcp__demo__search", "source:mcp")]);
        let tool = ToolSearchTool::new(catalog);

        let result = tool
            .execute(
                "call-1".to_string(),
                json!({"query": "demo"}),
                &tool_context(),
            )
            .await
            .expect("tool_search should succeed");

        assert!(result.ok);
        assert!(result.output.contains("mcp__demo__search"));
        assert!(result.output.contains("inputSchema"));
    }
}
