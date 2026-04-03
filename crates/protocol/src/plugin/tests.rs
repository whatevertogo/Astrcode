use serde_json::json;

use super::{
    CancelMessage, CapabilityDescriptor, CapabilityKind, DescriptorBuildError, ErrorPayload,
    EventMessage, EventPhase, FilterDescriptor, HandlerDescriptor, InitializeMessage,
    InitializeResultData, InvocationContext, PeerDescriptor, PeerRole, PermissionHint,
    PluginMessage, ProfileDescriptor, ResultMessage, SideEffectLevel, StabilityLevel,
    TriggerDescriptor, WorkspaceRef, PROTOCOL_VERSION,
};

fn sample_peer() -> PeerDescriptor {
    PeerDescriptor {
        id: "peer-1".to_string(),
        name: "sample".to_string(),
        role: PeerRole::Worker,
        version: "0.1.0".to_string(),
        supported_profiles: vec!["coding".to_string()],
        metadata: json!({ "region": "local" }),
    }
}

fn sample_capability() -> CapabilityDescriptor {
    CapabilityDescriptor::builder("tool.echo", CapabilityKind::tool())
        .description("Echo the input")
        .schema(json!({ "type": "object" }), json!({ "type": "object" }))
        .streaming(true)
        .profile("coding")
        .tag("test")
        .permissions(vec![PermissionHint {
            name: "filesystem.read".to_string(),
            rationale: Some("reads fixtures".to_string()),
        }])
        .side_effect(SideEffectLevel::Local)
        .stability(StabilityLevel::Stable)
        .build()
        .expect("sample capability should build")
}

#[test]
fn plugin_messages_roundtrip_as_v4_json() {
    let init = PluginMessage::Initialize(InitializeMessage {
        id: "init-1".to_string(),
        protocol_version: PROTOCOL_VERSION.to_string(),
        supported_protocol_versions: vec![PROTOCOL_VERSION.to_string()],
        peer: sample_peer(),
        capabilities: vec![sample_capability()],
        handlers: vec![HandlerDescriptor {
            id: "handler-1".to_string(),
            trigger: TriggerDescriptor {
                kind: "command".to_string(),
                value: "/echo".to_string(),
                metadata: json!({}),
            },
            input_schema: json!({ "type": "object" }),
            profiles: vec!["coding".to_string()],
            filters: vec![FilterDescriptor {
                field: "profile".to_string(),
                op: "eq".to_string(),
                value: "coding".to_string(),
            }],
            permissions: vec![],
        }],
        profiles: vec![ProfileDescriptor {
            name: "coding".to_string(),
            version: "1".to_string(),
            description: "Coding workflow".to_string(),
            context_schema: json!({ "type": "object" }),
            metadata: json!({}),
        }],
        metadata: json!({ "bootstrap": true }),
    });

    let invoke = PluginMessage::Invoke(super::InvokeMessage {
        id: "req-1".to_string(),
        capability: "tool.echo".to_string(),
        input: json!({ "message": "hi" }),
        context: InvocationContext {
            request_id: "req-1".to_string(),
            trace_id: Some("trace-1".to_string()),
            session_id: Some("session-1".to_string()),
            caller: None,
            workspace: Some(WorkspaceRef {
                working_dir: Some("/tmp/project".to_string()),
                repo_root: Some("/tmp/project".to_string()),
                branch: Some("main".to_string()),
                metadata: json!({}),
            }),
            deadline_ms: Some(5_000),
            budget: None,
            profile: "coding".to_string(),
            profile_context: json!({
                "workingDir": "/tmp/project",
                "repoRoot": "/tmp/project",
                "openFiles": ["/tmp/project/src/main.rs"],
                "activeFile": "/tmp/project/src/main.rs",
                "selection": { "startLine": 1, "endLine": 3 },
                "approvalMode": "on-request"
            }),
            metadata: json!({}),
        },
        stream: true,
    });

    let result = PluginMessage::Result(ResultMessage {
        id: "init-1".to_string(),
        kind: Some("initialize".to_string()),
        success: true,
        output: serde_json::to_value(InitializeResultData {
            protocol_version: PROTOCOL_VERSION.to_string(),
            peer: sample_peer(),
            capabilities: vec![sample_capability()],
            handlers: vec![],
            profiles: vec![],
            skills: vec![],
            metadata: json!({}),
        })
        .expect("serialize initialize result"),
        error: None,
        metadata: json!({ "acceptedVersion": PROTOCOL_VERSION }),
    });

    let event = PluginMessage::Event(EventMessage {
        id: "req-1".to_string(),
        phase: EventPhase::Delta,
        event: "artifact.patch".to_string(),
        payload: json!({ "path": "src/main.rs", "patch": "@@ ..." }),
        seq: 2,
        error: None,
    });

    let cancel = PluginMessage::Cancel(CancelMessage {
        id: "req-1".to_string(),
        reason: Some("user interrupted".to_string()),
    });

    for message in [init, invoke, result, event, cancel] {
        let json = serde_json::to_string(&message).expect("serialize message");
        let decoded: PluginMessage = serde_json::from_str(&json).expect("deserialize message");
        assert_eq!(decoded, message);
    }
}

#[test]
fn initialize_result_uses_result_kind_payload() {
    let result = ResultMessage {
        id: "init-1".to_string(),
        kind: Some("initialize".to_string()),
        success: true,
        output: serde_json::to_value(InitializeResultData {
            protocol_version: PROTOCOL_VERSION.to_string(),
            peer: sample_peer(),
            capabilities: vec![sample_capability()],
            handlers: vec![],
            profiles: vec![],
            skills: vec![],
            metadata: json!({ "mode": "stdio" }),
        })
        .expect("serialize initialize result"),
        error: None,
        metadata: json!({}),
    };

    let decoded: InitializeResultData = result.parse_output().expect("parse output");
    assert_eq!(decoded.protocol_version, PROTOCOL_VERSION);
    assert_eq!(decoded.peer.role, PeerRole::Worker);
    assert_eq!(decoded.capabilities[0].name, "tool.echo");
}

#[test]
fn invocation_context_supports_coding_profile_shape() {
    let context = InvocationContext {
        request_id: "req-1".to_string(),
        trace_id: None,
        session_id: Some("session-1".to_string()),
        caller: None,
        workspace: Some(WorkspaceRef {
            working_dir: Some("/repo".to_string()),
            repo_root: Some("/repo".to_string()),
            branch: None,
            metadata: json!({}),
        }),
        deadline_ms: None,
        budget: None,
        profile: "coding".to_string(),
        profile_context: json!({
            "workingDir": "/repo",
            "repoRoot": "/repo",
            "openFiles": ["/repo/src/lib.rs"],
            "activeFile": "/repo/src/lib.rs",
            "selection": {
                "startLine": 10,
                "startColumn": 1,
                "endLine": 12,
                "endColumn": 4
            },
            "approvalMode": "never"
        }),
        metadata: json!({}),
    };

    let value = serde_json::to_value(&context).expect("serialize context");
    assert_eq!(value["profile"], "coding");
    assert_eq!(value["profileContext"]["activeFile"], "/repo/src/lib.rs");
    assert_eq!(value["profileContext"]["approvalMode"], "never");
}

#[test]
fn result_message_preserves_error_payload_details() {
    let message = ResultMessage {
        id: "req-1".to_string(),
        kind: None,
        success: false,
        output: json!(null),
        error: Some(ErrorPayload {
            code: "permission_denied".to_string(),
            message: "filesystem.write requires approval".to_string(),
            details: json!({ "permission": "filesystem.write" }),
            retriable: false,
        }),
        metadata: json!({ "source": "policy" }),
    };

    let encoded = serde_json::to_value(&message).expect("serialize result");
    assert_eq!(
        encoded["error"]["details"]["permission"],
        "filesystem.write"
    );
    assert_eq!(encoded["metadata"]["source"], "policy");
}

#[test]
fn capability_builder_rejects_invalid_fields() {
    let error = CapabilityDescriptor::builder("tool.echo", CapabilityKind::tool())
        .description("Echo the input")
        .schema(json!({ "type": "object" }), json!("not-a-schema"))
        .profile("coding")
        .build()
        .expect_err("invalid output schema should fail");

    assert_eq!(error, DescriptorBuildError::InvalidSchema("output_schema"));
}

#[test]
fn capability_builder_accepts_custom_kind_strings() {
    let descriptor = CapabilityDescriptor::builder("workspace.index", "lsp.indexer")
        .description("Indexes workspace symbols")
        .schema(json!({ "type": "object" }), json!({ "type": "object" }))
        .build()
        .expect("custom kind should build");

    assert_eq!(descriptor.kind.as_str(), "lsp.indexer");
    assert_eq!(
        serde_json::to_value(&descriptor).expect("serialize descriptor")["kind"],
        "lsp.indexer"
    );
}

#[test]
fn capability_builder_rejects_blank_custom_kind() {
    let error = CapabilityDescriptor::builder("workspace.index", CapabilityKind::new("  "))
        .description("Indexes workspace symbols")
        .schema(json!({ "type": "object" }), json!({ "type": "object" }))
        .build()
        .expect_err("blank kind should fail");

    assert_eq!(error, DescriptorBuildError::EmptyField("kind"));
}

#[test]
fn capability_kind_deserialization_trims_whitespace() {
    let kind: CapabilityKind =
        serde_json::from_value(json!("  lsp.indexer  ")).expect("kind should deserialize");

    assert_eq!(kind.as_str(), "lsp.indexer");
}

#[test]
fn capability_validate_rejects_direct_blank_kind() {
    let descriptor = CapabilityDescriptor {
        name: "workspace.index".to_string(),
        kind: CapabilityKind::new("  "),
        description: "Indexes workspace symbols".to_string(),
        input_schema: json!({ "type": "object" }),
        output_schema: json!({ "type": "object" }),
        streaming: false,
        concurrency_safe: false,
        compact_clearable: false,
        profiles: vec![],
        tags: vec![],
        permissions: vec![],
        side_effect: SideEffectLevel::None,
        stability: StabilityLevel::Stable,
        metadata: json!({}),
    };

    assert_eq!(
        descriptor
            .validate()
            .expect_err("direct descriptor validation should reject blank kind"),
        DescriptorBuildError::EmptyField("kind")
    );
}
