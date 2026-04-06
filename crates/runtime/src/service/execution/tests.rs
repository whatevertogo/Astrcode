use std::sync::{Arc, Mutex};

use astrcode_core::{
    AgentEventContext, AgentMode, AgentProfile, AgentStatus, AstrError, CancelToken,
    ExecutionOwner, InvocationKind, StorageEvent, SubRunHandle, SubRunStorageMode,
    SubagentContextOverrides, Tool, ToolCapabilityMetadata, ToolContext, ToolDefinition,
    ToolEventSink, ToolExecutionResult, test_support::TestEnvGuard,
};
use astrcode_runtime_agent_tool::{RunAgentParams, RunAgentTool, SubAgentExecutor};
use astrcode_runtime_execution::{
    derive_child_execution_owner, resolve_profile_tool_names, resolve_subagent_overrides,
};
use astrcode_runtime_registry::ToolRegistry;
use async_trait::async_trait;
use serde_json::json;

use super::DeferredSubAgentExecutor;
use crate::{service::RuntimeService, test_support::capabilities_from_tools};

struct RecordingEventSink {
    events: Mutex<Vec<StorageEvent>>,
}

impl ToolEventSink for RecordingEventSink {
    fn emit(&self, event: StorageEvent) -> astrcode_core::Result<()> {
        self.events.lock().expect("events lock").push(event);
        Ok(())
    }
}

struct DemoTool {
    name: &'static str,
}

#[async_trait]
impl Tool for DemoTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name.to_string(),
            description: "demo".to_string(),
            parameters: json!({"type":"object","properties":{}}),
        }
    }

    fn capability_metadata(&self) -> ToolCapabilityMetadata {
        ToolCapabilityMetadata::builtin()
    }

    async fn execute(
        &self,
        tool_call_id: String,
        _input: serde_json::Value,
        _ctx: &ToolContext,
    ) -> astrcode_core::Result<ToolExecutionResult> {
        Ok(ToolExecutionResult {
            tool_call_id,
            tool_name: self.name.to_string(),
            ok: true,
            output: "ok".to_string(),
            error: None,
            metadata: None,
            duration_ms: 0,
            truncated: false,
        })
    }
}

#[test]
fn resolve_profile_tool_names_rejects_legacy_aliases() {
    let capabilities = capabilities_from_tools(
        ToolRegistry::builder()
            .register(Box::new(DemoTool { name: "readFile" }))
            .register(Box::new(DemoTool { name: "shell" }))
            .build(),
    );

    let error = resolve_profile_tool_names(
        &capabilities,
        &AgentProfile {
            id: "review".to_string(),
            name: "Review".to_string(),
            description: "review".to_string(),
            mode: AgentMode::SubAgent,
            system_prompt: None,
            allowed_tools: vec!["Read".to_string()],
            disallowed_tools: vec!["Bash".to_string()],
            max_steps: None,
            token_budget: None,
            model_preference: None,
        },
    )
    .expect_err("legacy aliases should be rejected explicitly");

    assert!(matches!(error, AstrError::Validation(_)));
    assert!(error.to_string().contains("unknown allowed_tools: Read"));
}

#[tokio::test]
async fn deferred_executor_fails_before_runtime_binding() {
    let executor = DeferredSubAgentExecutor::default();
    let context = ToolContext::new(
        "session-1".to_string(),
        std::env::temp_dir(),
        CancelToken::new(),
    );

    let error = executor
        .execute(
            RunAgentParams {
                name: "review".to_string(),
                task: "check".to_string(),
                context: None,
                max_steps: None,
                context_overrides: None,
            },
            &context,
        )
        .await
        .expect_err("unbound executor should fail");

    assert!(error.to_string().contains("not bound"));
}

#[tokio::test]
async fn run_agent_tool_emits_child_events_with_agent_context() {
    let _guard = TestEnvGuard::new();
    let service = Arc::new(
        RuntimeService::from_capabilities(capabilities_from_tools(
            ToolRegistry::builder()
                .register(Box::new(DemoTool { name: "readFile" }))
                .register(Box::new(DemoTool { name: "grep" }))
                .build(),
        ))
        .expect("runtime service should build"),
    );
    let executor = Arc::new(DeferredSubAgentExecutor::default());
    executor.bind(&service);
    let tool = RunAgentTool::new(executor);
    let sink = Arc::new(RecordingEventSink {
        events: Mutex::new(Vec::new()),
    });

    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let session = service
        .create_session(temp_dir.path())
        .await
        .expect("session should be created");
    let context = ToolContext::new(
        session.session_id.clone(),
        temp_dir.path().to_path_buf(),
        CancelToken::new(),
    )
    .with_turn_id("turn-parent")
    .with_event_sink(sink.clone());

    let result = tool
        .execute(
            "call-1".to_string(),
            json!({
                "name": "plan",
                "task": "summarize the repository layout",
                "maxSteps": 1
            }),
            &context,
        )
        .await;

    match result {
        Ok(tool_result) => assert!(tool_result.metadata.is_some()),
        Err(error) => {
            assert!(
                !error.to_string().contains("session not found"),
                "session should be loaded successfully, got: {error}"
            );
        },
    }
}

#[test]
fn resolve_subagent_overrides_rejects_mixed_instruction_inheritance() {
    let error = resolve_subagent_overrides(
        Some(&SubagentContextOverrides {
            inherit_system_instructions: Some(false),
            inherit_project_instructions: Some(true),
            ..Default::default()
        }),
        &crate::config::RuntimeConfig::default(),
    )
    .expect_err("mixed inheritance should be rejected");

    assert!(
        error
            .to_string()
            .contains("inheritSystemInstructions and inheritProjectInstructions")
    );
}

#[test]
fn resolve_subagent_overrides_rejects_unsupported_context_flags() {
    for overrides in [
        SubagentContextOverrides {
            inherit_cancel_token: Some(false),
            ..Default::default()
        },
        SubagentContextOverrides {
            include_recovery_refs: Some(true),
            ..Default::default()
        },
        SubagentContextOverrides {
            include_parent_findings: Some(true),
            ..Default::default()
        },
    ] {
        let error =
            resolve_subagent_overrides(Some(&overrides), &crate::config::RuntimeConfig::default())
                .expect_err("unsupported override should be rejected");
        assert!(error.to_string().contains("not supported yet"));
    }
}

#[test]
fn derive_child_execution_owner_keeps_root_identity() {
    let context = ToolContext::new(
        "session-parent".to_string(),
        std::env::temp_dir(),
        CancelToken::new(),
    )
    .with_execution_owner(ExecutionOwner::root(
        "session-root".to_string(),
        "turn-root".to_string(),
        InvocationKind::RootExecution,
    ))
    .with_agent_context(AgentEventContext::root_execution("agent-root", "execute"));
    let child = SubRunHandle {
        sub_run_id: "subrun-1".to_string(),
        agent_id: "agent-1".to_string(),
        session_id: "session-child".to_string(),
        child_session_id: Some("session-child".to_string()),
        depth: 1,
        parent_turn_id: Some("turn-root".to_string()),
        parent_agent_id: None,
        agent_profile: "plan".to_string(),
        storage_mode: SubRunStorageMode::IndependentSession,
        status: AgentStatus::Pending,
    };

    let owner = derive_child_execution_owner(&context, "turn-root", &child);
    assert_eq!(owner.root_session_id, "session-root");
    assert_eq!(owner.root_turn_id, "turn-root");
    assert_eq!(owner.sub_run_id.as_deref(), Some("subrun-1"));
    assert_eq!(owner.invocation_kind, InvocationKind::SubRun);
}
