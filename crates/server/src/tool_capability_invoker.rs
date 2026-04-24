//! server-owned Tool -> CapabilityInvoker bridge。
//!
//! Why: `server` 需要把 builtin / agent tools 注册到 capability surface。

use std::sync::Arc;

use astrcode_core::{
    AgentEventContext, AstrError, BoundModeToolContractSnapshot, CancelToken, CapabilityContext,
    CapabilityExecutionResult, CapabilityInvoker, CapabilitySpec, ExecutionOwner, Result,
    SessionId, Tool, ToolContext, ToolEventSink, ToolOutputDelta,
};
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;

const DEFAULT_TOOL_CAPABILITY_PROFILE: &str = "coding";

pub(crate) struct ToolCapabilityInvoker {
    tool: Arc<dyn Tool>,
    capability_spec: CapabilitySpec,
}

impl ToolCapabilityInvoker {
    pub(crate) fn new(tool: Arc<dyn Tool>) -> Result<Self> {
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
}

#[async_trait]
impl CapabilityInvoker for ToolCapabilityInvoker {
    fn capability_spec(&self) -> CapabilitySpec {
        self.capability_spec.clone()
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
            Ok(result) => {
                let common = result.common();
                Ok(CapabilityExecutionResult::from_common(
                    result.tool_name,
                    result.ok,
                    Value::String(result.output),
                    result.continuation,
                    common,
                ))
            },
            Err(error) => Ok(CapabilityExecutionResult::failure(
                self.capability_spec.name.to_string(),
                error.to_string(),
                Value::Null,
            )),
        }
    }
}

#[derive(Clone)]
struct ToolBridgeContext {
    session_id: SessionId,
    working_dir: std::path::PathBuf,
    cancel: CancelToken,
    turn_id: Option<String>,
    request_id: Option<String>,
    agent: AgentEventContext,
    current_mode_id: astrcode_core::ModeId,
    bound_mode_tool_contract: Option<BoundModeToolContractSnapshot>,
    execution_owner: Option<ExecutionOwner>,
    tool_output_sender: Option<UnboundedSender<ToolOutputDelta>>,
    event_sink: Option<Arc<dyn ToolEventSink>>,
}

impl ToolBridgeContext {
    fn from_tool_context(ctx: &ToolContext) -> Self {
        Self {
            session_id: ctx.session_id().into(),
            working_dir: ctx.working_dir().to_path_buf(),
            cancel: ctx.cancel().clone(),
            turn_id: ctx.turn_id().map(ToString::to_string),
            request_id: None,
            agent: ctx.agent_context().clone(),
            current_mode_id: ctx.current_mode_id().clone(),
            bound_mode_tool_contract: ctx.bound_mode_tool_contract().cloned(),
            execution_owner: ctx.execution_owner().cloned(),
            tool_output_sender: ctx.tool_output_sender(),
            event_sink: ctx.event_sink(),
        }
    }

    fn from_capability_context(ctx: &CapabilityContext) -> Self {
        Self {
            session_id: ctx.session_id.clone(),
            working_dir: ctx.working_dir.clone(),
            cancel: ctx.cancel.clone(),
            turn_id: ctx.turn_id.clone(),
            request_id: ctx.request_id.clone(),
            agent: ctx.agent.clone(),
            current_mode_id: ctx.current_mode_id.clone(),
            bound_mode_tool_contract: ctx.bound_mode_tool_contract.clone(),
            execution_owner: ctx.execution_owner.clone(),
            tool_output_sender: ctx.tool_output_sender.clone(),
            event_sink: ctx.event_sink.clone(),
        }
    }

    fn into_capability_context(self, request_id: Option<String>) -> CapabilityContext {
        CapabilityContext {
            request_id,
            trace_id: None,
            session_id: self.session_id,
            working_dir: self.working_dir.clone(),
            cancel: self.cancel,
            turn_id: self.turn_id,
            agent: self.agent,
            current_mode_id: self.current_mode_id,
            bound_mode_tool_contract: self.bound_mode_tool_contract,
            execution_owner: self.execution_owner,
            profile: default_tool_capability_profile().to_string(),
            profile_context: default_tool_capability_profile_context(&self.working_dir),
            metadata: Value::Null,
            tool_output_sender: self.tool_output_sender,
            event_sink: self.event_sink,
        }
    }

    fn into_tool_context(self) -> ToolContext {
        let mut tool_ctx = ToolContext::new(self.session_id, self.working_dir, self.cancel);
        if let Some(turn_id) = self.turn_id {
            tool_ctx = tool_ctx.with_turn_id(turn_id);
        }
        if let Some(tool_call_id) = self.request_id {
            tool_ctx = tool_ctx.with_tool_call_id(tool_call_id);
        }
        tool_ctx = tool_ctx.with_agent_context(self.agent);
        tool_ctx = tool_ctx.with_current_mode_id(self.current_mode_id);
        if let Some(snapshot) = self.bound_mode_tool_contract {
            tool_ctx = tool_ctx.with_bound_mode_tool_contract(snapshot);
        }
        if let Some(sender) = self.tool_output_sender {
            tool_ctx = tool_ctx.with_tool_output_sender(sender);
        }
        if let Some(event_sink) = self.event_sink {
            tool_ctx = tool_ctx.with_event_sink(event_sink);
        }
        if let Some(owner) = self.execution_owner {
            tool_ctx = tool_ctx.with_execution_owner(owner);
        }
        tool_ctx
    }
}

pub(crate) fn tool_context_from_capability_context(ctx: &CapabilityContext) -> ToolContext {
    ToolBridgeContext::from_capability_context(ctx).into_tool_context()
}

pub(crate) fn capability_context_from_tool_context(
    ctx: &ToolContext,
    request_id: Option<String>,
) -> CapabilityContext {
    ToolBridgeContext::from_tool_context(ctx).into_capability_context(request_id)
}

fn display_tool_label(name: &str) -> &str {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        "<unnamed>"
    } else {
        trimmed
    }
}

fn default_tool_capability_profile() -> &'static str {
    DEFAULT_TOOL_CAPABILITY_PROFILE
}

fn default_tool_capability_profile_context(working_dir: &std::path::Path) -> Value {
    let working_dir = working_dir.to_string_lossy().into_owned();
    serde_json::json!({
        "workingDir": working_dir,
        "repoRoot": working_dir,
        "approvalMode": "inherit"
    })
}
