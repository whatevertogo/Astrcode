//! 工具到能力调用器的桥接。
//!
//! `ToolCapabilityInvoker` 将 `Tool` trait 适配为 `CapabilityInvoker`，
//! 是生产路径中工具注册的唯一入口。

use std::sync::Arc;

use astrcode_core::{
    AstrError, CapabilityExecutionResult, CapabilityInvoker, CapabilitySpec, Result, Tool,
    ToolContext,
};
use async_trait::async_trait;
use serde_json::Value;

pub struct ToolCapabilityInvoker {
    tool: Arc<dyn Tool>,
    capability_spec: CapabilitySpec,
}

impl ToolCapabilityInvoker {
    pub fn new(tool: Arc<dyn Tool>) -> Result<Self> {
        let capability_spec = tool.capability_spec().map_err(|error| {
            let fallback_name = tool.definition().name;
            AstrError::Validation(format!(
                "invalid tool capability spec '{}': {}",
                display_tool_label(&fallback_name),
                error
            ))
        })?;
        capability_spec.validate().map_err(|error| {
            AstrError::Validation(format!(
                "invalid tool capability spec '{}': {}",
                display_tool_label(capability_spec.name.as_str()),
                error
            ))
        })?;
        Ok(Self {
            tool,
            capability_spec,
        })
    }

    pub fn boxed(tool: Box<dyn Tool>) -> Result<Arc<dyn CapabilityInvoker>> {
        Ok(Arc::new(Self::new(Arc::from(tool))?))
    }
}

#[async_trait]
impl CapabilityInvoker for ToolCapabilityInvoker {
    fn capability_spec(&self) -> CapabilitySpec {
        self.capability_spec.clone()
    }

    async fn invoke(
        &self,
        payload: Value,
        ctx: &astrcode_core::CapabilityContext,
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
                self.capability_spec.name.to_string(),
                error.to_string(),
                Value::Null,
            )),
        }
    }
}

pub(crate) fn capability_context_from_tool_context(
    ctx: &ToolContext,
    request_id: Option<String>,
) -> astrcode_core::CapabilityContext {
    let working_dir = ctx.working_dir().to_path_buf();
    let working_dir_str = working_dir.to_string_lossy().into_owned();

    astrcode_core::CapabilityContext {
        request_id,
        trace_id: None,
        session_id: ctx.session_id().into(),
        working_dir,
        cancel: ctx.cancel().clone(),
        turn_id: ctx.turn_id().map(ToString::to_string),
        agent: ctx.agent_context().clone(),
        execution_owner: ctx.execution_owner().cloned(),
        profile: "coding".to_string(),
        profile_context: serde_json::json!({
            "workingDir": working_dir_str,
            "repoRoot": working_dir_str,
            "approvalMode": "inherit"
        }),
        metadata: Value::Null,
        tool_output_sender: ctx.tool_output_sender(),
        event_sink: ctx.event_sink(),
    }
}

fn tool_context_from_capability_context(ctx: &astrcode_core::CapabilityContext) -> ToolContext {
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
