//! 工具到能力调用器的桥接。
//!
//! `ToolCapabilityInvoker` 将 `Tool` trait 适配为 `CapabilityInvoker`，
//! 是生产路径中工具注册的唯一入口。

use std::sync::Arc;

use astrcode_core::{
    AgentEventContext, AstrError, CancelToken, CapabilityContext, CapabilityExecutionResult,
    CapabilityInvoker, CapabilitySpec, ExecutionOwner, Result, SessionId, Tool, ToolContext,
    ToolEventSink, ToolOutputDelta,
};
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;

const DEFAULT_TOOL_CAPABILITY_PROFILE: &str = "coding";

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
            Ok(result) => {
                let common = result.common();
                Ok(CapabilityExecutionResult::from_common(
                    result.tool_name,
                    result.ok,
                    Value::String(result.output),
                    result.child_ref,
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

pub(crate) fn capability_context_from_tool_context(
    ctx: &ToolContext,
    request_id: Option<String>,
) -> CapabilityContext {
    ToolBridgeContext::from_tool_context(ctx).into_capability_context(request_id)
}

fn tool_context_from_capability_context(ctx: &CapabilityContext) -> ToolContext {
    ToolBridgeContext::from_capability_context(ctx).into_tool_context()
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
            execution_owner: ctx.execution_owner.clone(),
            tool_output_sender: ctx.tool_output_sender.clone(),
            event_sink: ctx.event_sink.clone(),
        }
    }

    fn into_capability_context(self, request_id: Option<String>) -> CapabilityContext {
        let profile_context = default_tool_capability_profile_context(&self.working_dir);

        CapabilityContext {
            request_id,
            trace_id: None,
            session_id: self.session_id,
            working_dir: self.working_dir,
            cancel: self.cancel,
            turn_id: self.turn_id,
            agent: self.agent,
            current_mode_id: self.current_mode_id,
            execution_owner: self.execution_owner,
            profile: default_tool_capability_profile().to_string(),
            profile_context,
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

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, sync::Arc};

    use astrcode_core::{
        AgentLifecycleStatus, CapabilityInvoker, ChildExecutionIdentity, ChildSessionLineageKind,
        ExecutionOwner, InvocationKind, ParentExecutionRef, Tool, ToolContext, ToolDefinition,
        ToolExecutionResult,
    };
    use async_trait::async_trait;
    use serde_json::{Value, json};

    use super::{
        ToolCapabilityInvoker, capability_context_from_tool_context,
        default_tool_capability_profile, default_tool_capability_profile_context,
        tool_context_from_capability_context,
    };

    #[test]
    fn capability_bridge_preserves_tool_context_fields() {
        let tool_ctx = ToolContext::new(
            "session-1".into(),
            PathBuf::from("/repo"),
            astrcode_core::CancelToken::new(),
        )
        .with_turn_id("turn-1")
        .with_tool_call_id("call-1")
        .with_agent_context(astrcode_core::AgentEventContext::root_execution(
            "agent-root",
            "planner",
        ))
        .with_execution_owner(ExecutionOwner::root(
            "session-1",
            "turn-root",
            InvocationKind::RootExecution,
        ));

        let capability_ctx =
            capability_context_from_tool_context(&tool_ctx, Some("request-1".to_string()));

        assert_eq!(capability_ctx.session_id.as_str(), "session-1");
        assert_eq!(capability_ctx.working_dir, PathBuf::from("/repo"));
        assert_eq!(capability_ctx.turn_id.as_deref(), Some("turn-1"));
        assert_eq!(capability_ctx.request_id.as_deref(), Some("request-1"));
        assert_eq!(capability_ctx.agent.agent_id.as_deref(), Some("agent-root"));
        assert_eq!(
            capability_ctx
                .execution_owner
                .as_ref()
                .map(|owner| owner.root_turn_id.as_str()),
            Some("turn-root")
        );
        assert_eq!(capability_ctx.profile, default_tool_capability_profile());
        assert_eq!(
            capability_ctx.profile_context,
            default_tool_capability_profile_context(&PathBuf::from("/repo"))
        );
    }

    #[test]
    fn tool_bridge_restores_request_id_as_tool_call_id() {
        let tool_ctx = ToolContext::new(
            "session-2".into(),
            PathBuf::from("/workspace"),
            astrcode_core::CancelToken::new(),
        )
        .with_turn_id("turn-2")
        .with_agent_context(astrcode_core::AgentEventContext::root_execution(
            "agent-2", "reviewer",
        ));

        let capability_ctx =
            capability_context_from_tool_context(&tool_ctx, Some("request-2".to_string()));
        let bridged_tool_ctx = tool_context_from_capability_context(&capability_ctx);

        assert_eq!(bridged_tool_ctx.session_id(), "session-2");
        assert_eq!(bridged_tool_ctx.working_dir(), PathBuf::from("/workspace"));
        assert_eq!(bridged_tool_ctx.turn_id(), Some("turn-2"));
        assert_eq!(bridged_tool_ctx.tool_call_id(), Some("request-2"));
        assert_eq!(
            bridged_tool_ctx.agent_context().agent_id.as_deref(),
            Some("agent-2")
        );
    }

    struct ChildRefTool;

    #[async_trait]
    impl Tool for ChildRefTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "spawn".to_string(),
                description: "returns child ref".to_string(),
                parameters: json!({"type": "object"}),
            }
        }

        fn capability_spec(
            &self,
        ) -> std::result::Result<
            astrcode_core::CapabilitySpec,
            astrcode_core::CapabilitySpecBuildError,
        > {
            astrcode_core::CapabilitySpec::builder("spawn", astrcode_core::CapabilityKind::Tool)
                .description("returns child ref")
                .schema(json!({"type": "object"}), json!({"type": "string"}))
                .build()
        }

        async fn execute(
            &self,
            tool_call_id: String,
            _input: Value,
            _ctx: &ToolContext,
        ) -> astrcode_core::Result<ToolExecutionResult> {
            Ok(ToolExecutionResult {
                tool_call_id,
                tool_name: "spawn".to_string(),
                ok: true,
                output: "spawn accepted".to_string(),
                error: None,
                metadata: Some(json!({ "schema": "subRunResult" })),
                child_ref: Some(astrcode_core::ChildAgentRef {
                    identity: ChildExecutionIdentity {
                        agent_id: "agent-child".into(),
                        session_id: "session-parent".into(),
                        sub_run_id: "subrun-1".into(),
                    },
                    parent: ParentExecutionRef {
                        parent_agent_id: Some("agent-parent".into()),
                        parent_sub_run_id: Some("subrun-parent".into()),
                    },
                    lineage_kind: ChildSessionLineageKind::Spawn,
                    status: AgentLifecycleStatus::Running,
                    open_session_id: "session-child".into(),
                }),
                duration_ms: 0,
                truncated: false,
            })
        }
    }

    #[tokio::test]
    async fn tool_capability_invoker_preserves_child_ref_in_capability_result() {
        let invoker =
            ToolCapabilityInvoker::new(Arc::new(ChildRefTool)).expect("tool invoker should build");
        let tool_ctx = ToolContext::new(
            "session-3".into(),
            PathBuf::from("/workspace"),
            astrcode_core::CancelToken::new(),
        )
        .with_tool_call_id("call-3");
        let capability_ctx =
            capability_context_from_tool_context(&tool_ctx, Some("call-3".to_string()));

        let result = invoker
            .invoke(json!({}), &capability_ctx)
            .await
            .expect("invocation should succeed");

        assert_eq!(
            result
                .child_ref
                .as_ref()
                .map(|child_ref| child_ref.agent_id().as_str()),
            Some("agent-child")
        );
        assert_eq!(
            result
                .child_ref
                .as_ref()
                .map(|child_ref| child_ref.open_session_id.as_str()),
            Some("session-child")
        );
    }
}
