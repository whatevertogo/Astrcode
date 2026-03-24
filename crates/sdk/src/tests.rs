use std::pin::Pin;

use astrcode_protocol::plugin::{
    CapabilityDescriptor, CapabilityKind, InvocationContext, WorkspaceRef,
};
use serde_json::json;

use crate::{
    PluginContext, PolicyDecision, PolicyHook, StreamWriter, ToolHandler, ToolRegistration,
};

#[derive(Default)]
struct SampleTool;

impl ToolHandler for SampleTool {
    fn descriptor(&self) -> CapabilityDescriptor {
        CapabilityDescriptor {
            name: "tool.sample".to_string(),
            kind: CapabilityKind::Tool,
            description: "Sample tool".to_string(),
            input_schema: json!({ "type": "object" }),
            output_schema: json!({ "type": "object" }),
            streaming: true,
            profiles: vec!["coding".to_string()],
            tags: vec!["sample".to_string()],
            permissions: vec![],
            side_effect: Default::default(),
            stability: Default::default(),
        }
    }

    fn execute(
        &self,
        input: serde_json::Value,
        _context: PluginContext,
        stream: StreamWriter,
    ) -> Pin<Box<dyn std::future::Future<Output = crate::tool::ToolResult<serde_json::Value>> + Send>>
    {
        Box::pin(async move {
            stream.message_delta("running")?;
            Ok(input)
        })
    }
}

struct SamplePolicy;

impl PolicyHook for SamplePolicy {
    fn before_invoke(
        &self,
        capability: &CapabilityDescriptor,
        _context: &PluginContext,
    ) -> PolicyDecision {
        if capability.name == "tool.sample" {
            PolicyDecision::allow()
        } else {
            PolicyDecision::deny("unsupported capability")
        }
    }
}

#[test]
fn tool_registration_uses_descriptor_first_model() {
    let registration = ToolRegistration::new(Box::new(SampleTool));
    assert_eq!(registration.descriptor().name, "tool.sample");
    assert!(registration.descriptor().streaming);
}

#[test]
fn coding_profile_helper_extracts_profile_context() {
    let context = PluginContext::from(InvocationContext {
        request_id: "req-1".to_string(),
        trace_id: Some("trace-1".to_string()),
        session_id: Some("session-1".to_string()),
        caller: None,
        workspace: Some(WorkspaceRef {
            working_dir: Some("/repo".to_string()),
            repo_root: Some("/repo".to_string()),
            branch: Some("main".to_string()),
            metadata: json!({}),
        }),
        deadline_ms: None,
        budget: None,
        profile: "coding".to_string(),
        profile_context: json!({
            "workingDir": "/repo",
            "repoRoot": "/repo",
            "openFiles": ["/repo/src/main.rs"],
            "activeFile": "/repo/src/main.rs",
            "selection": { "startLine": 1, "endLine": 2 },
            "approvalMode": "on-request"
        }),
        metadata: json!({}),
    });

    let coding = context.coding_profile().expect("coding profile");
    assert_eq!(coding.active_file.as_deref(), Some("/repo/src/main.rs"));
    assert_eq!(coding.approval_mode.as_deref(), Some("on-request"));
}

#[test]
fn stream_writer_records_standard_event_shapes() {
    let stream = StreamWriter::default();
    stream.message_delta("hello").expect("message delta");
    stream
        .artifact_patch("src/main.rs", "@@ -1 +1 @@")
        .expect("artifact patch");
    stream
        .diagnostic("warning", "unused variable")
        .expect("diagnostic");

    let records = stream.records();
    assert_eq!(records[0].event, "message.delta");
    assert_eq!(records[1].event, "artifact.patch");
    assert_eq!(records[2].event, "diagnostic");
}

#[test]
fn policy_hook_returns_structured_decision() {
    let policy = SamplePolicy;
    let decision = policy.before_invoke(&SampleTool.descriptor(), &PluginContext::default());
    assert!(decision.allowed);
    assert!(decision.reason.is_none());
}
