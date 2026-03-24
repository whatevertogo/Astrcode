use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    AstrError, CancelToken, CapabilityDescriptor, CapabilityKind, Result, ToolCallRequest,
    ToolContext, ToolDefinition, ToolExecutionResult, ToolRegistry,
};

#[derive(Clone, Debug)]
pub struct CapabilityContext {
    pub request_id: Option<String>,
    pub trace_id: Option<String>,
    pub session_id: String,
    pub working_dir: PathBuf,
    pub cancel: CancelToken,
    pub profile: String,
    pub profile_context: Value,
    pub metadata: Value,
}

impl CapabilityContext {
    pub fn from_tool_context(ctx: &ToolContext, request_id: impl Into<Option<String>>) -> Self {
        let working_dir = ctx.working_dir.to_string_lossy().into_owned();
        Self {
            request_id: request_id.into(),
            trace_id: None,
            session_id: ctx.session_id.clone(),
            working_dir: ctx.working_dir.clone(),
            cancel: ctx.cancel.clone(),
            profile: "coding".to_string(),
            profile_context: json!({
                "workingDir": working_dir,
                "repoRoot": working_dir,
                "approvalMode": "inherit"
            }),
            metadata: Value::Null,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityExecutionResult {
    pub capability_name: String,
    pub success: bool,
    pub output: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    pub duration_ms: u128,
    #[serde(default)]
    pub truncated: bool,
}

impl CapabilityExecutionResult {
    pub fn ok(capability_name: impl Into<String>, output: Value) -> Self {
        Self {
            capability_name: capability_name.into(),
            success: true,
            output,
            error: None,
            metadata: None,
            duration_ms: 0,
            truncated: false,
        }
    }

    pub fn failure(
        capability_name: impl Into<String>,
        error: impl Into<String>,
        output: Value,
    ) -> Self {
        Self {
            capability_name: capability_name.into(),
            success: false,
            output,
            error: Some(error.into()),
            metadata: None,
            duration_ms: 0,
            truncated: false,
        }
    }

    pub fn output_text(&self) -> String {
        match &self.output {
            Value::Null => String::new(),
            Value::String(text) => text.clone(),
            other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
        }
    }

    pub fn into_tool_execution_result(self, tool_call_id: String) -> ToolExecutionResult {
        let output = self.output_text();
        ToolExecutionResult {
            tool_call_id,
            tool_name: self.capability_name,
            ok: self.success,
            output,
            error: self.error,
            metadata: self.metadata,
            duration_ms: self.duration_ms,
            truncated: self.truncated,
        }
    }
}

#[async_trait]
pub trait CapabilityInvoker: Send + Sync {
    fn descriptor(&self) -> CapabilityDescriptor;

    async fn invoke(
        &self,
        payload: Value,
        ctx: &CapabilityContext,
    ) -> Result<CapabilityExecutionResult>;
}

pub struct CapabilityRouterBuilder {
    invokers: Vec<Arc<dyn CapabilityInvoker>>,
}

impl CapabilityRouterBuilder {
    pub fn new() -> Self {
        Self {
            invokers: Vec::new(),
        }
    }

    pub fn register_invoker(mut self, invoker: Arc<dyn CapabilityInvoker>) -> Self {
        self.invokers.push(invoker);
        self
    }

    pub fn register_tool_registry(mut self, registry: ToolRegistry) -> Self {
        let registry = Arc::new(registry);
        for definition in registry.definitions() {
            self.invokers
                .push(Arc::new(ToolRegistryCapabilityInvoker::new(
                    Arc::clone(&registry),
                    definition,
                )));
        }
        self
    }

    pub fn build(self) -> Result<CapabilityRouter> {
        let mut invokers_by_name = HashMap::new();
        let mut order = Vec::new();

        for invoker in self.invokers {
            let descriptor = invoker.descriptor();
            if invokers_by_name
                .insert(descriptor.name.clone(), Arc::clone(&invoker))
                .is_some()
            {
                return Err(AstrError::Validation(format!(
                    "duplicate capability '{}' registered",
                    descriptor.name
                )));
            }
            order.push(descriptor.name);
        }

        Ok(CapabilityRouter {
            invokers_by_name,
            order,
        })
    }
}

pub struct CapabilityRouter {
    invokers_by_name: HashMap<String, Arc<dyn CapabilityInvoker>>,
    order: Vec<String>,
}

impl CapabilityRouter {
    pub fn builder() -> CapabilityRouterBuilder {
        CapabilityRouterBuilder::new()
    }

    pub fn from_tool_registry(registry: ToolRegistry) -> Self {
        CapabilityRouter::builder()
            .register_tool_registry(registry)
            .build()
            .expect("tool registry cannot produce duplicate capability names")
    }

    pub fn descriptors(&self) -> Vec<CapabilityDescriptor> {
        self.order
            .iter()
            .filter_map(|name| self.invokers_by_name.get(name))
            .map(|invoker| invoker.descriptor())
            .collect()
    }

    pub fn capability_names(&self) -> Vec<String> {
        self.order.clone()
    }

    pub fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.descriptors()
            .into_iter()
            .filter(|descriptor| descriptor.kind == CapabilityKind::Tool)
            .map(|descriptor| ToolDefinition {
                name: descriptor.name,
                description: descriptor.description,
                parameters: descriptor.input_schema,
            })
            .collect()
    }

    pub fn tool_names(&self) -> Vec<String> {
        self.tool_definitions()
            .into_iter()
            .map(|definition| definition.name)
            .collect()
    }

    pub async fn invoke(
        &self,
        name: &str,
        payload: Value,
        ctx: &CapabilityContext,
    ) -> Result<CapabilityExecutionResult> {
        let invoker = self
            .invokers_by_name
            .get(name)
            .ok_or_else(|| AstrError::Validation(format!("unknown capability '{name}'")))?;
        invoker.invoke(payload, ctx).await
    }

    pub async fn execute_tool(
        &self,
        call: &ToolCallRequest,
        ctx: &ToolContext,
    ) -> ToolExecutionResult {
        let Some(invoker) = self.invokers_by_name.get(&call.name) else {
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

        let descriptor = invoker.descriptor();
        if descriptor.kind != CapabilityKind::Tool {
            return ToolExecutionResult {
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                ok: false,
                output: String::new(),
                error: Some(format!("capability '{}' is not tool-callable", call.name)),
                metadata: None,
                duration_ms: 0,
                truncated: false,
            };
        }

        match invoker
            .invoke(
                call.args.clone(),
                &CapabilityContext::from_tool_context(ctx, Some(call.id.clone())),
            )
            .await
        {
            Ok(result) => result.into_tool_execution_result(call.id.clone()),
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
}

struct ToolRegistryCapabilityInvoker {
    registry: Arc<ToolRegistry>,
    definition: ToolDefinition,
}

impl ToolRegistryCapabilityInvoker {
    fn new(registry: Arc<ToolRegistry>, definition: ToolDefinition) -> Self {
        Self {
            registry,
            definition,
        }
    }
}

#[async_trait]
impl CapabilityInvoker for ToolRegistryCapabilityInvoker {
    fn descriptor(&self) -> CapabilityDescriptor {
        CapabilityDescriptor {
            name: self.definition.name.clone(),
            kind: CapabilityKind::Tool,
            description: self.definition.description.clone(),
            input_schema: self.definition.parameters.clone(),
            output_schema: json!({ "type": "string" }),
            streaming: false,
            profiles: vec!["coding".to_string()],
            tags: vec!["builtin".to_string()],
            permissions: Vec::new(),
            side_effect: crate::SideEffectLevel::Workspace,
            stability: crate::StabilityLevel::Stable,
        }
    }

    async fn invoke(
        &self,
        payload: Value,
        ctx: &CapabilityContext,
    ) -> Result<CapabilityExecutionResult> {
        let result = self
            .registry
            .execute(
                &ToolCallRequest {
                    id: ctx
                        .request_id
                        .clone()
                        .unwrap_or_else(|| "capability-call".to_string()),
                    name: self.definition.name.clone(),
                    args: payload,
                },
                &ToolContext {
                    session_id: ctx.session_id.clone(),
                    working_dir: ctx.working_dir.clone(),
                    cancel: ctx.cancel.clone(),
                    max_output_size: crate::DEFAULT_MAX_OUTPUT_SIZE,
                },
            )
            .await;

        Ok(CapabilityExecutionResult {
            capability_name: result.tool_name,
            success: result.ok,
            output: Value::String(result.output),
            error: result.error,
            metadata: result.metadata,
            duration_ms: result.duration_ms,
            truncated: result.truncated,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use serde_json::{json, Value};

    use super::{CapabilityExecutionResult, CapabilityInvoker, CapabilityRouter, ToolRegistry};
    use crate::{
        CancelToken, CapabilityContext, CapabilityDescriptor, CapabilityKind, Result,
        SideEffectLevel, StabilityLevel, Tool, ToolCallRequest, ToolContext, ToolDefinition,
        ToolExecutionResult,
    };

    struct FakeTool;

    #[async_trait]
    impl Tool for FakeTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "fake".to_string(),
                description: "fake".to_string(),
                parameters: json!({ "type": "object" }),
            }
        }

        async fn execute(
            &self,
            tool_call_id: String,
            _input: Value,
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

    struct EchoCapability;

    #[async_trait]
    impl CapabilityInvoker for EchoCapability {
        fn descriptor(&self) -> CapabilityDescriptor {
            CapabilityDescriptor {
                name: "plugin.echo".to_string(),
                kind: CapabilityKind::Tool,
                description: "echo".to_string(),
                input_schema: json!({ "type": "object" }),
                output_schema: json!({ "type": "object" }),
                streaming: false,
                profiles: vec!["coding".to_string()],
                tags: vec![],
                permissions: vec![],
                side_effect: SideEffectLevel::None,
                stability: StabilityLevel::Stable,
            }
        }

        async fn invoke(
            &self,
            payload: Value,
            _ctx: &CapabilityContext,
        ) -> Result<CapabilityExecutionResult> {
            Ok(CapabilityExecutionResult::ok("plugin.echo", payload))
        }
    }

    fn tool_context() -> ToolContext {
        ToolContext {
            session_id: "session-1".to_string(),
            working_dir: std::env::temp_dir(),
            cancel: CancelToken::new(),
            max_output_size: crate::DEFAULT_MAX_OUTPUT_SIZE,
        }
    }

    #[tokio::test]
    async fn from_tool_registry_exposes_tool_definitions_and_executes_tools() {
        let router = CapabilityRouter::from_tool_registry(
            ToolRegistry::builder().register(Box::new(FakeTool)).build(),
        );
        assert_eq!(router.tool_names(), vec!["fake".to_string()]);

        let result = router
            .execute_tool(
                &ToolCallRequest {
                    id: "call-1".to_string(),
                    name: "fake".to_string(),
                    args: json!({}),
                },
                &tool_context(),
            )
            .await;
        assert!(result.ok);
        assert_eq!(result.output, "ok");
    }

    #[test]
    fn builder_rejects_duplicate_capabilities() {
        let result = CapabilityRouter::builder()
            .register_invoker(Arc::new(EchoCapability))
            .register_invoker(Arc::new(EchoCapability))
            .build();
        let error = match result {
            Ok(_) => panic!("duplicate capabilities should fail"),
            Err(error) => error,
        };
        assert!(matches!(error, crate::AstrError::Validation(_)));
    }
}
