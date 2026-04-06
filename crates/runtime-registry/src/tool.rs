//! 工具注册表具体实现（runtime 侧）。

use std::{collections::HashMap, sync::Arc};

use astrcode_core::{
    AstrError, CapabilityContext, CapabilityDescriptor, CapabilityExecutionResult,
    CapabilityInvoker, Result, Tool, ToolCallRequest, ToolContext, ToolDefinition,
    ToolExecutionResult,
};
use async_trait::async_trait;
use serde_json::Value;

pub struct ToolRegistryBuilder {
    tools: HashMap<String, Box<dyn Tool>>,
    order: Vec<String>,
}

impl Default for ToolRegistryBuilder {
    fn default() -> Self {
        Self::new()
    }
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

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.order
            .iter()
            .filter_map(|name| self.tools.get(name))
            .map(|tool| tool.definition())
            .collect()
    }

    pub fn names(&self) -> &[String] {
        &self.order
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
        let descriptor = tool.capability_descriptor().map_err(|error| {
            let fallback_name = tool.definition().name;
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
        let mut tool_ctx = ToolContext::new(
            ctx.session_id.clone(),
            ctx.working_dir.clone(),
            ctx.cancel.clone(),
        );
        if let Some(turn_id) = &ctx.turn_id {
            tool_ctx = tool_ctx.with_turn_id(turn_id.clone());
        }
        tool_ctx = tool_ctx.with_agent_context(ctx.agent.clone());
        if let Some(sender) = ctx.tool_output_sender.clone() {
            tool_ctx = tool_ctx.with_tool_output_sender(sender);
        }
        if let Some(event_sink) = ctx.event_sink.clone() {
            tool_ctx = tool_ctx.with_event_sink(event_sink);
        }
        if let Some(owner) = ctx.execution_owner.clone() {
            tool_ctx = tool_ctx.with_execution_owner(owner);
        }
        let result = self
            .tool
            .execute(
                ctx.request_id
                    .clone()
                    .unwrap_or_else(|| "capability-call".to_string()),
                payload,
                &tool_ctx,
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
