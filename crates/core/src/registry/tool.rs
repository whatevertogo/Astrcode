use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::{
    AstrError, CapabilityContext, CapabilityDescriptor, CapabilityExecutionResult,
    CapabilityInvoker, Result, Tool, ToolCallRequest, ToolContext, ToolExecutionResult,
};

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

    pub fn definitions(&self) -> Vec<crate::ToolDefinition> {
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
                truncated: false,
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
                truncated: false,
            },
        }
    }

    /// Converts the frozen tool registry into generic capability invokers while preserving
    /// registration order.
    pub fn into_capability_invokers(mut self) -> Result<Vec<Arc<dyn CapabilityInvoker>>> {
        self.order
            .into_iter()
            .filter_map(|name| self.tools.remove(&name))
            .map(ToolCapabilityInvoker::boxed)
            .collect()
    }
}

pub struct ToolCapabilityInvoker {
    tool: Arc<dyn Tool>,
    descriptor: CapabilityDescriptor,
}

impl ToolCapabilityInvoker {
    pub fn new(tool: Arc<dyn Tool>) -> Result<Self> {
        let fallback_name = tool.definition().name;
        let descriptor = tool.capability_descriptor().map_err(|error| {
            AstrError::Validation(format!(
                "invalid tool descriptor '{}': {}",
                display_tool_label(&fallback_name),
                error
            ))
        })?;
        descriptor.validate().map_err(|error| {
            AstrError::Validation(format!(
                "invalid tool descriptor '{}': {}",
                display_tool_label(&descriptor.name),
                error
            ))
        })?;
        Ok(Self { tool, descriptor })
    }

    pub fn boxed(tool: Box<dyn Tool>) -> Result<Arc<dyn CapabilityInvoker>> {
        Ok(Arc::new(Self::new(Arc::from(tool))?))
    }
}

#[async_trait]
impl CapabilityInvoker for ToolCapabilityInvoker {
    fn descriptor(&self) -> CapabilityDescriptor {
        self.descriptor.clone()
    }

    async fn invoke(
        &self,
        payload: Value,
        ctx: &CapabilityContext,
    ) -> Result<CapabilityExecutionResult> {
        let result = self
            .tool
            .execute(
                ctx.request_id
                    .clone()
                    .unwrap_or_else(|| "capability-call".to_string()),
                payload,
                &ToolContext {
                    session_id: ctx.session_id.clone(),
                    working_dir: ctx.working_dir.clone(),
                    cancel: ctx.cancel.clone(),
                    max_output_size: crate::DEFAULT_MAX_OUTPUT_SIZE,
                },
            )
            .await;

        match result {
            Ok(result) => Ok(CapabilityExecutionResult {
                capability_name: result.tool_name,
                success: result.ok,
                output: Value::String(result.output),
                error: result.error,
                metadata: result.metadata,
                duration_ms: result.duration_ms,
                truncated: result.truncated,
            }),
            Err(error) => Ok(CapabilityExecutionResult::failure(
                self.descriptor.name.clone(),
                error.to_string(),
                Value::Null,
            )),
        }
    }
}

fn display_tool_label(name: &str) -> &str {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        "<unnamed>"
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use serde_json::json;

    use super::{ToolCapabilityInvoker, ToolRegistry, ToolRegistryBuilder};
    use crate::{
        CancelToken, CapabilityContext, CapabilityDescriptor, CapabilityInvoker, CapabilityKind,
        PermissionHint, Result, SideEffectLevel, StabilityLevel, Tool, ToolCallRequest,
        ToolCapabilityMetadata, ToolContext, ToolDefinition, ToolExecutionResult,
    };

    struct FakeTool;
    struct PolicyAwareTool;
    struct InvalidTool;

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
                truncated: false,
            })
        }
    }

    #[async_trait]
    impl Tool for PolicyAwareTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "policyAware".to_string(),
                description: "policy aware".to_string(),
                parameters: json!({"type": "object"}),
            }
        }

        fn capability_metadata(&self) -> ToolCapabilityMetadata {
            ToolCapabilityMetadata::builtin()
                .tags(["filesystem", "read"])
                .permission_with_rationale("filesystem.read", "reads workspace files")
                .side_effect(SideEffectLevel::None)
                .stability(StabilityLevel::Experimental)
        }

        async fn execute(
            &self,
            tool_call_id: String,
            _input: serde_json::Value,
            _ctx: &ToolContext,
        ) -> Result<ToolExecutionResult> {
            Ok(ToolExecutionResult {
                tool_call_id,
                tool_name: "policyAware".to_string(),
                ok: true,
                output: "ok".to_string(),
                error: None,
                metadata: None,
                duration_ms: 0,
                truncated: false,
            })
        }
    }

    #[async_trait]
    impl Tool for InvalidTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "invalid".to_string(),
                description: "invalid".to_string(),
                parameters: json!({"type": "object"}),
            }
        }

        fn capability_descriptor(
            &self,
        ) -> std::result::Result<CapabilityDescriptor, crate::DescriptorBuildError> {
            Ok(CapabilityDescriptor {
                name: "   ".to_string(),
                kind: CapabilityKind::tool(),
                description: "invalid".to_string(),
                input_schema: json!({"type": "object"}),
                output_schema: json!({"type": "string"}),
                streaming: false,
                profiles: Vec::new(),
                tags: Vec::new(),
                permissions: Vec::<PermissionHint>::new(),
                side_effect: SideEffectLevel::Workspace,
                stability: StabilityLevel::Stable,
            })
        }

        async fn execute(
            &self,
            tool_call_id: String,
            _input: serde_json::Value,
            _ctx: &ToolContext,
        ) -> Result<ToolExecutionResult> {
            Ok(ToolExecutionResult {
                tool_call_id,
                tool_name: "invalid".to_string(),
                ok: true,
                output: "ok".to_string(),
                error: None,
                metadata: None,
                duration_ms: 0,
                truncated: false,
            })
        }
    }

    fn test_context() -> ToolContext {
        ToolContext {
            session_id: "session-1".to_string(),
            working_dir: std::env::temp_dir(),
            cancel: CancelToken::new(),
            max_output_size: crate::DEFAULT_MAX_OUTPUT_SIZE,
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

    #[tokio::test]
    async fn tool_capability_invoker_wraps_tool_execution() {
        let invoker = ToolCapabilityInvoker::new(Arc::new(FakeTool))
            .expect("default tool descriptor should build");
        let result = invoker
            .invoke(
                json!({}),
                &CapabilityContext {
                    request_id: Some("call-1".to_string()),
                    trace_id: None,
                    session_id: "session-1".to_string(),
                    working_dir: std::env::temp_dir(),
                    cancel: CancelToken::new(),
                    profile: "coding".to_string(),
                    profile_context: serde_json::Value::Null,
                    metadata: serde_json::Value::Null,
                },
            )
            .await
            .expect("invocation should succeed");

        assert!(result.success);
        assert_eq!(result.capability_name, "fake");
        assert_eq!(result.output, serde_json::Value::String("ok".to_string()));
    }

    #[test]
    fn into_capability_invokers_preserves_registration_order() {
        let invokers = ToolRegistry::builder()
            .register(Box::new(FakeTool))
            .build()
            .into_capability_invokers()
            .expect("tool descriptors should build");

        assert_eq!(
            invokers
                .into_iter()
                .map(|invoker| invoker.descriptor().name)
                .collect::<Vec<_>>(),
            vec!["fake".to_string()]
        );
    }

    #[test]
    fn tool_capability_invoker_uses_tool_metadata_for_descriptor() {
        let descriptor = ToolCapabilityInvoker::new(Arc::new(PolicyAwareTool))
            .expect("custom tool metadata should build")
            .descriptor();

        assert_eq!(descriptor.name, "policyAware");
        assert_eq!(descriptor.side_effect, SideEffectLevel::None);
        assert_eq!(descriptor.stability, StabilityLevel::Experimental);
        assert_eq!(
            descriptor.permissions,
            vec![PermissionHint {
                name: "filesystem.read".to_string(),
                rationale: Some("reads workspace files".to_string()),
            }]
        );
        assert_eq!(
            descriptor.tags,
            vec![
                "builtin".to_string(),
                "filesystem".to_string(),
                "read".to_string()
            ]
        );
    }

    #[test]
    fn tool_capability_invoker_rejects_invalid_tool_descriptors() {
        let error = match ToolCapabilityInvoker::new(Arc::new(InvalidTool)) {
            Ok(_) => panic!("invalid descriptor should be rejected"),
            Err(error) => error,
        };

        assert!(matches!(error, crate::AstrError::Validation(_)));
    }
}
