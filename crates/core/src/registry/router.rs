use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    AstrError, CancelToken, CapabilityDescriptor, Result, ToolCallRequest, ToolContext,
    ToolDefinition, ToolExecutionResult,
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
        let working_dir = ctx.working_dir().to_string_lossy().into_owned();
        Self {
            request_id: request_id.into(),
            trace_id: None,
            session_id: ctx.session_id().to_string(),
            working_dir: ctx.working_dir().clone(),
            cancel: ctx.cancel().clone(),
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
    pub duration_ms: u64,
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

impl Default for CapabilityRouterBuilder {
    fn default() -> Self {
        Self::new()
    }
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

    pub fn build(self) -> Result<CapabilityRouter> {
        let mut invokers_by_name = HashMap::new();
        let mut order = Vec::new();
        let mut tool_order = Vec::new();

        for invoker in self.invokers {
            let descriptor = invoker.descriptor();
            descriptor.validate().map_err(|error| {
                AstrError::Validation(format!(
                    "invalid capability descriptor '{}': {}",
                    descriptor.name, error
                ))
            })?;
            if invokers_by_name
                .insert(descriptor.name.clone(), Arc::clone(&invoker))
                .is_some()
            {
                return Err(AstrError::Validation(format!(
                    "duplicate capability '{}' registered",
                    descriptor.name
                )));
            }
            if descriptor.kind.is_tool() {
                tool_order.push(descriptor.name.clone());
            }
            order.push(descriptor.name);
        }

        Ok(CapabilityRouter {
            invokers_by_name,
            order,
            tool_order,
        })
    }
}

pub struct CapabilityRouter {
    invokers_by_name: HashMap<String, Arc<dyn CapabilityInvoker>>,
    order: Vec<String>,
    tool_order: Vec<String>,
}

impl CapabilityRouter {
    pub fn builder() -> CapabilityRouterBuilder {
        CapabilityRouterBuilder::new()
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

    pub fn descriptor(&self, name: &str) -> Option<CapabilityDescriptor> {
        self.invokers_by_name
            .get(name)
            .map(|invoker| invoker.descriptor())
    }

    /// Projects tool-callable capabilities into the LLM-facing tool definition surface.
    ///
    /// This is an adapter view over the generic capability registry, not the core capability
    /// contract itself.
    pub fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.descriptors()
            .into_iter()
            .filter(|descriptor| descriptor.kind.is_tool())
            .map(|descriptor| ToolDefinition {
                name: descriptor.name,
                description: descriptor.description,
                parameters: descriptor.input_schema,
            })
            .collect()
    }

    pub fn tool_names(&self) -> &[String] {
        &self.tool_order
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
        // Tool execution is a projection of the generic capability surface. Keeping the
        // kind-check here confines tool-call semantics to the adapter path instead of requiring
        // the broader capability invoke contract to branch on every kind.
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
        if !descriptor.kind.is_tool() {
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use serde_json::{json, Value};

    use super::{CapabilityExecutionResult, CapabilityInvoker, CapabilityRouter};
    use crate::{
        CancelToken, CapabilityContext, CapabilityDescriptor, CapabilityKind, Result,
        SideEffectLevel, StabilityLevel, Tool, ToolCallRequest, ToolCapabilityInvoker, ToolContext,
        ToolDefinition, ToolExecutionResult,
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
                kind: CapabilityKind::tool(),
                description: "echo".to_string(),
                input_schema: json!({ "type": "object" }),
                output_schema: json!({ "type": "object" }),
                streaming: false,
                profiles: vec!["coding".to_string()],
                tags: vec![],
                permissions: vec![],
                side_effect: SideEffectLevel::None,
                stability: StabilityLevel::Stable,
                metadata: Value::Null,
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
        ToolContext::new(
            "session-1".to_string(),
            std::env::temp_dir(),
            CancelToken::new(),
        )
    }

    #[tokio::test]
    async fn invoker_registered_tools_expose_tool_definitions_and_execute() {
        let router = CapabilityRouter::builder()
            .register_invoker(
                ToolCapabilityInvoker::boxed(Box::new(FakeTool))
                    .expect("tool descriptor should build"),
            )
            .build()
            .expect("router should build");
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
