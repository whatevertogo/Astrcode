//! 工具注册表具体实现（runtime 侧）。

use std::{collections::HashMap, sync::Arc};

use astrcode_core::{
    AstrError, CapabilityContext, CapabilityExecutionResult, CapabilityInvoker, Result, Tool,
    ToolContext,
};
use astrcode_protocol::capability::CapabilityDescriptor;
use async_trait::async_trait;
use serde_json::{Value, json};

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

    pub fn into_capability_invokers(mut self) -> Result<Vec<Arc<dyn CapabilityInvoker>>> {
        let tools = self
            .order
            .into_iter()
            .filter_map(|name| self.tools.remove(&name))
            .collect::<Vec<_>>();
        tools_into_capability_invokers(tools)
    }
}

pub fn tools_into_capability_invokers<I>(tools: I) -> Result<Vec<Arc<dyn CapabilityInvoker>>>
where
    I: IntoIterator<Item = Box<dyn Tool>>,
{
    tools
        .into_iter()
        .map(ToolCapabilityInvoker::boxed)
        .collect()
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
        let tool_ctx = tool_context_from_capability_context(ctx);
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

pub(crate) fn capability_context_from_tool_context(
    ctx: &ToolContext,
    request_id: Option<String>,
) -> CapabilityContext {
    // 运行时默认 profile / approval 继承策略属于 runtime 装配决策，
    // 不应该固化在 core DTO 转换里。
    let working_dir = ctx.working_dir().to_path_buf();
    let working_dir_str = working_dir.to_string_lossy().into_owned();

    CapabilityContext {
        request_id,
        trace_id: None,
        session_id: ctx.session_id().to_string(),
        working_dir,
        cancel: ctx.cancel().clone(),
        turn_id: ctx.turn_id().map(ToString::to_string),
        agent: ctx.agent_context().clone(),
        execution_owner: ctx.execution_owner().cloned(),
        profile: "coding".to_string(),
        profile_context: json!({
            "workingDir": working_dir_str,
            "repoRoot": working_dir_str,
            "approvalMode": "inherit"
        }),
        metadata: Value::Null,
        tool_output_sender: ctx.tool_output_sender(),
        event_sink: ctx.event_sink(),
    }
}

fn tool_context_from_capability_context(ctx: &CapabilityContext) -> ToolContext {
    let mut tool_ctx = ToolContext::new(
        ctx.session_id.clone(),
        ctx.working_dir.clone(),
        ctx.cancel.clone(),
    );
    if let Some(turn_id) = &ctx.turn_id {
        tool_ctx = tool_ctx.with_turn_id(turn_id.clone());
    }
    if let Some(tool_call_id) = &ctx.request_id {
        tool_ctx = tool_ctx.with_tool_call_id(tool_call_id.clone());
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
    tool_ctx
}

fn display_tool_label(name: &str) -> &str {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        "<unnamed>"
    } else {
        trimmed
    }
}
