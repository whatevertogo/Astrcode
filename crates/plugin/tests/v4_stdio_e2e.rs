use std::{sync::Arc, time::Duration};

use astrcode_core::{PluginManifest, PluginType, Result};
use astrcode_plugin::{
    CapabilityRouter, Peer, PluginProcess, Supervisor, default_initialize_message, default_profiles,
};
use astrcode_protocol::plugin::{
    EventPhase, InvocationContext, PeerDescriptor, PeerRole, WorkspaceRef,
};
use serde_json::{Value, json};
use tokio::time::{sleep, timeout};

fn fixture_manifest() -> PluginManifest {
    PluginManifest {
        name: "fixture-worker".to_string(),
        version: "0.1.0".to_string(),
        description: "Fixture worker for stdio e2e tests".to_string(),
        plugin_type: vec![PluginType::Tool],
        capabilities: vec![],
        executable: Some(env!("CARGO_BIN_EXE_fixture_worker").to_string()),
        args: Vec::new(),
        working_dir: None,
        repository: None,
    }
}

fn local_peer() -> PeerDescriptor {
    PeerDescriptor {
        id: "astrcode-supervisor".to_string(),
        name: "astrcode-supervisor".to_string(),
        role: PeerRole::Supervisor,
        version: env!("CARGO_PKG_VERSION").to_string(),
        supported_profiles: vec!["coding".to_string()],
        metadata: json!({ "transport": "stdio" }),
    }
}

fn coding_context(request_id: &str) -> InvocationContext {
    InvocationContext {
        request_id: request_id.to_string(),
        trace_id: Some(format!("trace-{request_id}")),
        session_id: Some("session-1".to_string()),
        caller: None,
        workspace: Some(WorkspaceRef {
            working_dir: Some("D:/workspace/project".to_string()),
            repo_root: Some("D:/workspace/project".to_string()),
            branch: Some("main".to_string()),
            metadata: Value::Null,
        }),
        deadline_ms: Some(10_000),
        budget: None,
        profile: "coding".to_string(),
        profile_context: json!({
            "workingDir": "D:/workspace/project",
            "repoRoot": "D:/workspace/project",
            "openFiles": ["D:/workspace/project/src/main.rs"],
            "activeFile": "D:/workspace/project/src/main.rs",
            "selection": {
                "startLine": 1,
                "startColumn": 1,
                "endLine": 2,
                "endColumn": 1
            },
            "approvalMode": "never"
        }),
        metadata: Value::Null,
    }
}

#[tokio::test]
async fn stdio_supervisor_initializes_and_invokes_unary_capability() -> Result<()> {
    let manifest = fixture_manifest();
    let supervisor = Supervisor::start(&manifest, local_peer()).await?;

    assert_eq!(supervisor.remote_initialize().peer.role, PeerRole::Worker);
    assert!(
        supervisor
            .remote_initialize()
            .capabilities
            .iter()
            .any(|capability| capability.name == "tool.echo")
    );

    let response = supervisor
        .invoke(
            "tool.echo",
            json!({ "message": "hello" }),
            coding_context("req-unary"),
        )
        .await?;
    assert!(response.success);
    assert_eq!(response.output["message"], "hello");

    supervisor.shutdown().await
}

#[tokio::test]
async fn stdio_supervisor_streams_started_delta_completed_lifecycle() -> Result<()> {
    let manifest = fixture_manifest();
    let supervisor = Supervisor::start(&manifest, local_peer()).await?;
    let mut stream = supervisor
        .invoke_stream(
            "tool.patch_stream",
            json!({ "path": "src/main.rs" }),
            coding_context("req-stream"),
        )
        .await?;

    let mut phases = Vec::new();
    let mut delta_events = Vec::new();
    while let Some(event) = stream.recv().await {
        phases.push(event.phase.clone());
        if event.phase == EventPhase::Delta {
            delta_events.push(event.event.clone());
        }
        if matches!(event.phase, EventPhase::Completed | EventPhase::Failed) {
            assert_eq!(event.payload["status"], "applied");
            break;
        }
    }

    assert_eq!(phases.first(), Some(&EventPhase::Started));
    assert!(delta_events.iter().all(|event| event == "artifact.patch"));
    assert!(matches!(phases.last(), Some(EventPhase::Completed)));

    supervisor.shutdown().await
}

#[tokio::test]
async fn stdio_supervisor_propagates_cancel_to_streaming_worker() -> Result<()> {
    let manifest = fixture_manifest();
    let supervisor = Supervisor::start(&manifest, local_peer()).await?;
    let mut stream = supervisor
        .invoke_stream(
            "tool.patch_stream",
            json!({ "path": "src/lib.rs" }),
            coding_context("req-cancel"),
        )
        .await?;
    let request_id = stream.request_id().to_string();

    let mut saw_delta = false;
    while let Some(event) = stream.recv().await {
        if event.phase == EventPhase::Delta {
            saw_delta = true;
            supervisor
                .cancel(request_id.clone(), Some("user interrupted".to_string()))
                .await?;
        }
        if event.phase == EventPhase::Failed {
            let error = event
                .error
                .expect("cancel failed event should include error");
            assert_eq!(error.code, "cancelled");
            break;
        }
    }

    assert!(saw_delta);
    supervisor.shutdown().await
}

#[tokio::test]
async fn stdio_peer_closes_pending_request_and_stream_when_process_dies() -> Result<()> {
    let manifest = fixture_manifest();
    let mut process = PluginProcess::start(&manifest).await?;
    let peer = Peer::new(
        process.transport(),
        default_initialize_message(local_peer(), Vec::new(), default_profiles()),
        Arc::new(CapabilityRouter::default()),
    );
    let remote = peer.initialize().await?;
    assert_eq!(remote.peer.role, PeerRole::Worker);

    let unary_peer = peer.clone();
    let unary_task = tokio::spawn(async move {
        unary_peer
            .invoke(astrcode_protocol::plugin::InvokeMessage {
                id: "req-close-unary".to_string(),
                capability: "tool.delayed_echo".to_string(),
                input: json!({ "message": "wait" }),
                context: coding_context("req-close-unary"),
                stream: false,
            })
            .await
    });

    let mut stream = peer
        .invoke_stream(astrcode_protocol::plugin::InvokeMessage {
            id: "req-close-stream".to_string(),
            capability: "tool.patch_stream".to_string(),
            input: json!({ "path": "src/main.rs" }),
            context: coding_context("req-close-stream"),
            stream: true,
        })
        .await?;

    sleep(Duration::from_millis(80)).await;
    process.shutdown().await?;

    let unary = unary_task
        .await
        .expect("join unary")
        .expect("invoke result");
    assert!(!unary.success);
    assert_eq!(
        unary.error.as_ref().expect("transport close error").code,
        "transport_closed"
    );

    let mut terminal = None;
    while let Some(event) = stream.recv().await {
        if matches!(event.phase, EventPhase::Completed | EventPhase::Failed) {
            terminal = Some(event);
            break;
        }
    }
    let terminal = terminal.expect("terminal stream event");
    assert_eq!(terminal.phase, EventPhase::Failed);
    assert_eq!(
        terminal.error.as_ref().expect("stream close error").code,
        "transport_closed"
    );

    Ok(())
}

#[tokio::test]
async fn stdio_peer_abort_closes_pending_request_and_stream() -> Result<()> {
    let manifest = fixture_manifest();
    let mut process = PluginProcess::start(&manifest).await?;
    let peer = Peer::new(
        process.transport(),
        default_initialize_message(local_peer(), Vec::new(), default_profiles()),
        Arc::new(CapabilityRouter::default()),
    );
    let remote = peer.initialize().await?;
    assert_eq!(remote.peer.role, PeerRole::Worker);

    let unary_peer = peer.clone();
    let unary_task = tokio::spawn(async move {
        unary_peer
            .invoke(astrcode_protocol::plugin::InvokeMessage {
                id: "req-abort-unary".to_string(),
                capability: "tool.delayed_echo".to_string(),
                input: json!({ "message": "wait" }),
                context: coding_context("req-abort-unary"),
                stream: false,
            })
            .await
    });

    let mut stream = peer
        .invoke_stream(astrcode_protocol::plugin::InvokeMessage {
            id: "req-abort-stream".to_string(),
            capability: "tool.patch_stream".to_string(),
            input: json!({ "path": "src/main.rs" }),
            context: coding_context("req-abort-stream"),
            stream: true,
        })
        .await?;

    sleep(Duration::from_millis(80)).await;
    peer.abort().await;

    let unary = timeout(Duration::from_secs(2), unary_task)
        .await
        .expect("unary invoke should resolve after peer abort")
        .expect("join unary")
        .expect("invoke result");
    assert!(!unary.success);
    assert_eq!(
        unary.error.as_ref().expect("transport close error").code,
        "transport_closed"
    );

    let terminal = timeout(Duration::from_secs(2), async {
        loop {
            if let Some(event) = stream.recv().await {
                if matches!(event.phase, EventPhase::Completed | EventPhase::Failed) {
                    return event;
                }
            }
        }
    })
    .await
    .expect("stream should terminate after peer abort");
    assert_eq!(terminal.phase, EventPhase::Failed);
    assert_eq!(
        terminal.error.as_ref().expect("stream close error").code,
        "transport_closed"
    );

    process.shutdown().await
}
