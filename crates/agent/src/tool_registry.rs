use std::collections::HashMap;

use astrcode_core::{Tool, ToolCallRequest, ToolContext, ToolExecutionResult};

pub struct ToolRegistryBuilder {
    tools: HashMap<String, Box<dyn Tool>>,
    order: Vec<String>,
}

impl ToolRegistryBuilder {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            order: Vec::new(),
        }
    }

    pub fn register(mut self, tool: Box<dyn Tool>) -> Self {
        let name = tool.definition().name;
        if let Some(index) = self.order.iter().position(|existing| existing == &name) {
            self.order.remove(index);
        }
        self.order.push(name.clone());
        self.tools.insert(name, tool);
        self
    }

    pub fn build(self) -> ToolRegistry {
        ToolRegistry {
            tools: self.tools,
            order: self.order,
        }
    }
}

pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
    order: Vec<String>,
}

impl ToolRegistry {
    pub fn builder() -> ToolRegistryBuilder {
        ToolRegistryBuilder::new()
    }

    pub fn definitions(&self) -> Vec<astrcode_core::ToolDefinition> {
        self.order
            .iter()
            .filter_map(|name| self.tools.get(name))
            .map(|tool| tool.definition())
            .collect()
    }

    pub fn names(&self) -> Vec<String> {
        self.order.clone()
    }

    pub async fn execute(&self, call: &ToolCallRequest, ctx: &ToolContext) -> ToolExecutionResult {
        let Some(tool) = self.tools.get(&call.name) else {
            return ToolExecutionResult {
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                ok: false,
                output: String::new(),
                error: Some(format!("unknown tool '{}'", call.name)),
                metadata: None,
                duration_ms: 0,
            };
        };

        match tool.execute(call.id.clone(), call.args.clone(), ctx).await {
            Ok(result) => result,
            Err(error) => ToolExecutionResult {
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                ok: false,
                output: String::new(),
                error: Some(error.to_string()),
                metadata: None,
                duration_ms: 0,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use serde_json::json;

    use super::{ToolRegistry, ToolRegistryBuilder};
    use astrcode_core::{
        CancelToken, Result, Tool, ToolCallRequest, ToolContext, ToolDefinition, ToolExecutionResult,
    };

    struct FakeTool;

    #[async_trait]
    impl Tool for FakeTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "fake".to_string(),
                description: "fake".to_string(),
                parameters: json!({"type": "object"}),
            }
        }

        async fn execute(
            &self,
            tool_call_id: String,
            _input: serde_json::Value,
            _ctx: &ToolContext,
        ) -> Result<ToolExecutionResult> {
            Ok(ToolExecutionResult {
                tool_call_id,
                tool_name: "fake".to_string(),
                ok: true,
                output: "ok".to_string(),
                error: None,
                metadata: None,
                duration_ms: 0,
            })
        }
    }

    fn test_context() -> ToolContext {
        ToolContext {
            session_id: "session-1".to_string(),
            working_dir: std::env::temp_dir(),
            sandbox_root: std::env::temp_dir(),
            cancel: CancelToken::new(),
        }
    }

    #[tokio::test]
    async fn built_registry_is_read_only_and_executes_registered_tool() {
        let registry = ToolRegistryBuilder::new()
            .register(Box::new(FakeTool))
            .build();
        let result = registry
            .execute(
                &ToolCallRequest {
                    id: "tool-1".to_string(),
                    name: "fake".to_string(),
                    args: json!({}),
                },
                &test_context(),
            )
            .await;

        assert!(result.ok);
    }

    #[test]
    fn builder_preserves_registration_order() {
        let registry = ToolRegistry::builder().register(Box::new(FakeTool)).build();
        assert_eq!(registry.names(), vec!["fake".to_string()]);
    }
}
