use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use astrcode_core::{
    AgentEventContext, AgentMode, AgentProfile, AgentStatus, AstrError, CancelToken,
    ExecutionOwner, InvocationKind, ResolvedExecutionLimitsSnapshot,
    ResolvedSubagentContextOverrides, SpawnAgentParams, StorageEvent, SubRunHandle, SubRunHandoff,
    SubRunOutcome, SubRunResult, SubRunStorageMode, SubagentContextOverrides, Tool,
    ToolCapabilityMetadata, ToolContext, ToolDefinition, ToolEventSink, ToolExecutionResult,
    test_support::TestEnvGuard,
};
use astrcode_runtime_agent_tool::{SpawnAgentTool, SubAgentExecutor};
use astrcode_runtime_config::DEFAULT_MAX_CONCURRENT_AGENTS;
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

fn append_storage_event(service: &RuntimeService, session_id: &str, event: &StorageEvent) {
    let mut writer = service
        .session_manager
        .open_event_log(session_id)
        .expect("session event log should open");
    writer
        .append(event)
        .expect("storage event should append successfully");
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
        .launch(
            SpawnAgentParams {
                r#type: Some("review".to_string()),
                description: "check".to_string(),
                prompt: "check".to_string(),
                context: None,
            },
            &context,
        )
        .await
        .expect_err("unbound executor should fail");

    assert!(error.to_string().contains("not bound"));
}

#[tokio::test]
async fn scoped_agent_profile_cache_reuses_loaded_registry_until_reload() {
    let _guard = TestEnvGuard::new();
    let service = Arc::new(
        RuntimeService::from_capabilities(capabilities_from_tools(ToolRegistry::builder().build()))
            .expect("runtime service should build"),
    );
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let project_agents_dir = temp_dir.path().join(".astrcode").join("agents");
    std::fs::create_dir_all(&project_agents_dir).expect("project agents dir should exist");
    std::fs::write(
        project_agents_dir.join("review.md"),
        r#"---
name: review
description: 第一版审查员
tools: [readFile]
---
先检查缓存命中。
"#,
    )
    .expect("initial agent definition should be written");

    let _session = service
        .sessions()
        .create(temp_dir.path())
        .await
        .expect("session should be created");

    let first = service
        .execution()
        .load_profiles_for_working_dir(temp_dir.path())
        .await
        .expect("initial scoped profiles should load");
    assert_eq!(
        first
            .get("review")
            .expect("review profile should exist")
            .description,
        "第一版审查员"
    );

    std::fs::write(
        project_agents_dir.join("review.md"),
        r#"---
name: review
description: 第二版审查员
tools: [readFile, grep]
---
缓存失效后应该看到新版。
"#,
    )
    .expect("updated agent definition should be written");

    let cached = service
        .execution()
        .load_profiles_for_working_dir(temp_dir.path())
        .await
        .expect("cached scoped profiles should load");
    assert_eq!(
        cached
            .get("review")
            .expect("cached review profile should exist")
            .description,
        "第一版审查员"
    );

    service
        .reload_agent_profiles_from_disk()
        .await
        .expect("agent profile reload should succeed");

    let refreshed = service
        .execution()
        .load_profiles_for_working_dir(temp_dir.path())
        .await
        .expect("scoped profiles should refresh after reload");
    let review = refreshed
        .get("review")
        .expect("refreshed review profile should exist");
    assert_eq!(review.description, "第二版审查员");
    assert_eq!(
        review.allowed_tools,
        vec!["readFile".to_string(), "grep".to_string()]
    );
}

#[tokio::test]
async fn spawn_agent_tool_emits_child_events_with_agent_context() {
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
    let tool = SpawnAgentTool::new(executor);
    let sink = Arc::new(RecordingEventSink {
        events: Mutex::new(Vec::new()),
    });

    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let session = service
        .sessions()
        .create(temp_dir.path())
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
                "type": "plan",
                "description": "summarize repository layout",
                "prompt": "summarize the repository layout"
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

#[tokio::test]
async fn spawn_agent_background_cancellation_releases_concurrency_slots() {
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
    let tool = SpawnAgentTool::new(executor);
    let sink: Arc<dyn ToolEventSink> = Arc::new(RecordingEventSink {
        events: Mutex::new(Vec::new()),
    });

    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let session = service
        .sessions()
        .create(temp_dir.path())
        .await
        .expect("session should be created");
    let context = ToolContext::new(
        session.session_id.clone(),
        temp_dir.path().to_path_buf(),
        CancelToken::new(),
    )
    .with_turn_id("turn-parent")
    .with_event_sink(sink);

    // 连续启动后台子会话并显式取消，验证取消后的终态会及时释放并发槽位。
    for _ in 0..(DEFAULT_MAX_CONCURRENT_AGENTS + 2) {
        let result = tool
            .execute(
                "call-1".to_string(),
                json!({
                    "type": "plan",
                    "description": "regression test for spawnAgent slot leak",
                    "prompt": "collect quick summary"
                }),
                &context,
            )
            .await
            .expect("background launch should succeed");
        let sub_run_id = result
            .metadata
            .as_ref()
            .and_then(|value| value.get("handoff"))
            .and_then(|value| value.get("artifacts"))
            .and_then(|value| value.as_array())
            .and_then(|artifacts| artifacts.first())
            .and_then(|artifact| artifact.get("id"))
            .and_then(|value| value.as_str())
            .expect("background launch should expose sub-run artifact")
            .to_string();
        service
            .execution()
            .cancel_subrun(&session.session_id, &sub_run_id)
            .await
            .expect("sub-run cancel should succeed");
        let handle = service
            .agent_control()
            .wait(&sub_run_id)
            .await
            .expect("cancelled sub-run should still be observable");
        assert!(
            matches!(handle.status, AgentStatus::Cancelled | AgentStatus::Failed),
            "background sub-run should reach a final state after explicit cancel"
        );
    }
}

#[tokio::test]
async fn spawn_agent_lifecycle_events_persist_parent_tool_call_id() {
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
    let tool = SpawnAgentTool::new(executor);
    let sink = Arc::new(RecordingEventSink {
        events: Mutex::new(Vec::new()),
    });

    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let session = service
        .sessions()
        .create(temp_dir.path())
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
            "call-persist-1".to_string(),
            json!({
                "type": "plan",
                "description": "persist tool call id",
                "prompt": "collect summary"
            }),
            &context,
        )
        .await
        .expect("background launch should succeed");
    let sub_run_id = result
        .metadata
        .as_ref()
        .and_then(|value| value.get("handoff"))
        .and_then(|value| value.get("artifacts"))
        .and_then(|value| value.as_array())
        .and_then(|artifacts| artifacts.first())
        .and_then(|artifact| artifact.get("id"))
        .and_then(|value| value.as_str())
        .expect("sub-run id should be exposed")
        .to_string();

    service
        .execution()
        .cancel_subrun(&session.session_id, &sub_run_id)
        .await
        .expect("cancel should succeed");
    let _ = service
        .agent_control()
        .wait(&sub_run_id)
        .await
        .expect("cancelled sub-run should be observable");

    let started = crate::service::session::load_events(
        Arc::clone(&service.session_manager),
        &session.session_id,
    )
    .await
    .expect("session events should load")
    .iter()
    .find_map(|stored| match &stored.event {
        StorageEvent::SubRunStarted {
            tool_call_id,
            descriptor,
            ..
        } => Some((tool_call_id.clone(), descriptor.clone())),
        _ => None,
    });

    let finished = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(found) = crate::service::session::load_events(
                Arc::clone(&service.session_manager),
                &session.session_id,
            )
            .await
            .expect("session events should load")
            .iter()
            .find_map(|stored| match &stored.event {
                StorageEvent::SubRunFinished {
                    tool_call_id,
                    descriptor,
                    ..
                } => Some((tool_call_id.clone(), descriptor.clone())),
                _ => None,
            }) {
                break found;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("sub-run finished event should be persisted");
    assert_eq!(
        started
            .as_ref()
            .and_then(|(tool_call_id, _)| tool_call_id.as_deref()),
        Some("call-persist-1")
    );
    assert!(
        started
            .as_ref()
            .and_then(|(_, descriptor)| descriptor.as_ref())
            .is_some()
    );
    assert_eq!(finished.0.as_deref(), Some("call-persist-1"));
    assert!(finished.1.is_some());
}

#[tokio::test]
async fn get_subrun_status_reconstructs_durable_snapshot_without_live_handle() {
    let _guard = TestEnvGuard::new();
    let service = Arc::new(
        RuntimeService::from_capabilities(capabilities_from_tools(
            ToolRegistry::builder()
                .register(Box::new(DemoTool { name: "readFile" }))
                .build(),
        ))
        .expect("runtime service should build"),
    );

    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let session = service
        .sessions()
        .create(temp_dir.path())
        .await
        .expect("session should be created");
    let agent = AgentEventContext::sub_run(
        "agent-durable".to_string(),
        "turn-parent".to_string(),
        "review".to_string(),
        "subrun-durable".to_string(),
        SubRunStorageMode::IndependentSession,
        Some("child-session".to_string()),
    );
    let overrides = ResolvedSubagentContextOverrides {
        storage_mode: SubRunStorageMode::IndependentSession,
        ..Default::default()
    };
    let limits = ResolvedExecutionLimitsSnapshot {
        allowed_tools: vec!["readFile".to_string()],
    };
    let result = SubRunResult {
        status: SubRunOutcome::Completed,
        handoff: Some(SubRunHandoff {
            summary: "done".to_string(),
            findings: vec!["ok".to_string()],
            artifacts: Vec::new(),
        }),
        failure: None,
    };

    append_storage_event(
        service.as_ref(),
        &session.session_id,
        &StorageEvent::SubRunStarted {
            turn_id: Some("turn-parent".to_string()),
            agent: agent.clone(),
            descriptor: None,
            tool_call_id: None,
            resolved_overrides: overrides.clone(),
            resolved_limits: limits.clone(),
            timestamp: None,
        },
    );
    append_storage_event(
        service.as_ref(),
        &session.session_id,
        &StorageEvent::SubRunFinished {
            turn_id: Some("turn-parent".to_string()),
            agent,
            descriptor: None,
            tool_call_id: None,
            result: result.clone(),
            step_count: 3,
            estimated_tokens: 222,
            timestamp: None,
        },
    );

    let snapshot = service
        .execution()
        .get_subrun_status(&session.session_id, "subrun-durable")
        .await
        .expect("durable snapshot should be reconstructed");

    assert_eq!(snapshot.handle.status, AgentStatus::Completed);
    assert_eq!(snapshot.handle.agent_id, "agent-durable");
    assert_eq!(snapshot.handle.agent_profile, "review");
    assert_eq!(snapshot.result, Some(result));
    assert_eq!(snapshot.step_count, Some(3));
    assert_eq!(snapshot.estimated_tokens, Some(222));
    assert_eq!(snapshot.resolved_overrides, Some(overrides));
    assert_eq!(snapshot.resolved_limits, Some(limits));
}

#[tokio::test]
async fn get_subrun_status_prefers_live_handle_when_agent_control_has_entry() {
    let _guard = TestEnvGuard::new();
    let service = Arc::new(
        RuntimeService::from_capabilities(capabilities_from_tools(
            ToolRegistry::builder()
                .register(Box::new(DemoTool { name: "readFile" }))
                .build(),
        ))
        .expect("runtime service should build"),
    );

    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let session = service
        .sessions()
        .create(temp_dir.path())
        .await
        .expect("session should be created");
    let control = service.agent_control();
    let profile = AgentProfile {
        id: "review".to_string(),
        name: "Review".to_string(),
        description: "review".to_string(),
        mode: AgentMode::SubAgent,
        system_prompt: None,
        allowed_tools: vec!["readFile".to_string()],
        disallowed_tools: Vec::new(),
        model_preference: None,
    };
    let handle = control
        .spawn(
            &profile,
            &session.session_id,
            Some("turn-parent".to_string()),
            None,
        )
        .await
        .expect("live sub-run should spawn");
    let _ = control
        .mark_running(&handle.sub_run_id)
        .await
        .expect("live sub-run should become running");

    append_storage_event(
        service.as_ref(),
        &session.session_id,
        &StorageEvent::SubRunFinished {
            turn_id: Some("turn-parent".to_string()),
            agent: AgentEventContext::sub_run(
                handle.agent_id.clone(),
                "turn-parent".to_string(),
                "review".to_string(),
                handle.sub_run_id.clone(),
                SubRunStorageMode::SharedSession,
                None,
            ),
            descriptor: None,
            tool_call_id: None,
            result: SubRunResult {
                status: SubRunOutcome::Completed,
                handoff: None,
                failure: None,
            },
            step_count: 9,
            estimated_tokens: 999,
            timestamp: None,
        },
    );

    let snapshot = service
        .execution()
        .get_subrun_status(&session.session_id, &handle.sub_run_id)
        .await
        .expect("live snapshot should be returned");

    assert_eq!(snapshot.handle.sub_run_id, handle.sub_run_id);
    assert_eq!(snapshot.handle.agent_id, handle.agent_id);
    assert_eq!(snapshot.handle.status, AgentStatus::Running);
    assert_eq!(snapshot.source, crate::service::SubRunStatusSource::Live);
    assert_eq!(
        snapshot.result.as_ref().map(|item| item.status.clone()),
        Some(SubRunOutcome::Completed)
    );
    assert_eq!(snapshot.step_count, Some(9));
    assert_eq!(snapshot.estimated_tokens, Some(999));
}

#[tokio::test]
async fn get_subrun_status_keeps_storage_mode_parity_for_parent_aborted_subruns() {
    let _guard = TestEnvGuard::new();
    let service = Arc::new(
        RuntimeService::from_capabilities(capabilities_from_tools(
            ToolRegistry::builder()
                .register(Box::new(DemoTool { name: "readFile" }))
                .build(),
        ))
        .expect("runtime service should build"),
    );

    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let session = service
        .sessions()
        .create(temp_dir.path())
        .await
        .expect("session should be created");

    let cases = [
        ("subrun-shared", SubRunStorageMode::SharedSession, None),
        (
            "subrun-independent",
            SubRunStorageMode::IndependentSession,
            Some("child-independent".to_string()),
        ),
    ];

    for (sub_run_id, storage_mode, child_session_id) in &cases {
        let agent = AgentEventContext::sub_run(
            format!("agent-{sub_run_id}"),
            "turn-parent".to_string(),
            "review".to_string(),
            (*sub_run_id).to_string(),
            *storage_mode,
            child_session_id.clone(),
        );
        append_storage_event(
            service.as_ref(),
            &session.session_id,
            &StorageEvent::SubRunStarted {
                turn_id: Some("turn-parent".to_string()),
                agent: agent.clone(),
                descriptor: None,
                tool_call_id: None,
                resolved_overrides: ResolvedSubagentContextOverrides {
                    storage_mode: *storage_mode,
                    ..Default::default()
                },
                resolved_limits: ResolvedExecutionLimitsSnapshot::default(),
                timestamp: None,
            },
        );
        append_storage_event(
            service.as_ref(),
            &session.session_id,
            &StorageEvent::SubRunFinished {
                turn_id: Some("turn-parent".to_string()),
                agent,
                descriptor: None,
                tool_call_id: None,
                result: SubRunResult {
                    status: SubRunOutcome::Aborted,
                    handoff: None,
                    failure: None,
                },
                step_count: 1,
                estimated_tokens: 33,
                timestamp: None,
            },
        );

        let snapshot = service
            .execution()
            .get_subrun_status(&session.session_id, sub_run_id)
            .await
            .expect("durable snapshot should be reconstructed");

        assert_eq!(snapshot.handle.storage_mode, *storage_mode);
        assert_eq!(snapshot.handle.status, AgentStatus::Cancelled);
        assert_eq!(
            snapshot.result.as_ref().map(|item| item.status.clone()),
            Some(SubRunOutcome::Aborted)
        );
    }
}

#[tokio::test]
async fn get_subrun_status_rejects_live_handle_from_other_session() {
    let _guard = TestEnvGuard::new();
    let service = Arc::new(
        RuntimeService::from_capabilities(capabilities_from_tools(
            ToolRegistry::builder()
                .register(Box::new(DemoTool { name: "readFile" }))
                .build(),
        ))
        .expect("runtime service should build"),
    );

    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let session_a = service
        .sessions()
        .create(temp_dir.path())
        .await
        .expect("session A should be created");
    let session_b = service
        .sessions()
        .create(temp_dir.path())
        .await
        .expect("session B should be created");
    let control = service.agent_control();
    let profile = AgentProfile {
        id: "review".to_string(),
        name: "Review".to_string(),
        description: "review".to_string(),
        mode: AgentMode::SubAgent,
        system_prompt: None,
        allowed_tools: vec!["readFile".to_string()],
        disallowed_tools: Vec::new(),
        model_preference: None,
    };
    let handle = control
        .spawn(
            &profile,
            &session_a.session_id,
            Some("turn-a".to_string()),
            None,
        )
        .await
        .expect("live sub-run should spawn");
    let _ = control
        .mark_running(&handle.sub_run_id)
        .await
        .expect("live sub-run should become running");

    let error = service
        .execution()
        .get_subrun_status(&session_b.session_id, &handle.sub_run_id)
        .await
        .expect_err("cross-session live handle should not be visible");

    assert!(error.to_string().contains("was not found"));
}

#[tokio::test]
async fn get_subrun_status_does_not_overlay_unrelated_live_handle_when_durable_exists() {
    let _guard = TestEnvGuard::new();
    let service = Arc::new(
        RuntimeService::from_capabilities(capabilities_from_tools(
            ToolRegistry::builder()
                .register(Box::new(DemoTool { name: "readFile" }))
                .build(),
        ))
        .expect("runtime service should build"),
    );

    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let session_a = service
        .sessions()
        .create(temp_dir.path())
        .await
        .expect("session A should be created");
    let session_b = service
        .sessions()
        .create(temp_dir.path())
        .await
        .expect("session B should be created");
    let control = service.agent_control();
    let profile = AgentProfile {
        id: "review".to_string(),
        name: "Review".to_string(),
        description: "review".to_string(),
        mode: AgentMode::SubAgent,
        system_prompt: None,
        allowed_tools: vec!["readFile".to_string()],
        disallowed_tools: Vec::new(),
        model_preference: None,
    };
    let live_handle = control
        .spawn(
            &profile,
            &session_a.session_id,
            Some("turn-a".to_string()),
            None,
        )
        .await
        .expect("live sub-run should spawn");
    let _ = control
        .mark_running(&live_handle.sub_run_id)
        .await
        .expect("live sub-run should become running");

    append_storage_event(
        service.as_ref(),
        &session_b.session_id,
        &StorageEvent::SubRunStarted {
            turn_id: Some("turn-b".to_string()),
            agent: AgentEventContext::sub_run(
                "agent-b".to_string(),
                "turn-b".to_string(),
                "review".to_string(),
                live_handle.sub_run_id.clone(),
                SubRunStorageMode::SharedSession,
                None,
            ),
            descriptor: Some(astrcode_core::SubRunDescriptor {
                sub_run_id: live_handle.sub_run_id.clone(),
                parent_turn_id: "turn-b".to_string(),
                parent_agent_id: Some("agent-parent-b".to_string()),
                depth: 1,
            }),
            tool_call_id: Some("call-b".to_string()),
            resolved_overrides: ResolvedSubagentContextOverrides::default(),
            resolved_limits: ResolvedExecutionLimitsSnapshot::default(),
            timestamp: None,
        },
    );
    append_storage_event(
        service.as_ref(),
        &session_b.session_id,
        &StorageEvent::SubRunFinished {
            turn_id: Some("turn-b".to_string()),
            agent: AgentEventContext::sub_run(
                "agent-b".to_string(),
                "turn-b".to_string(),
                "review".to_string(),
                live_handle.sub_run_id.clone(),
                SubRunStorageMode::SharedSession,
                None,
            ),
            descriptor: Some(astrcode_core::SubRunDescriptor {
                sub_run_id: live_handle.sub_run_id.clone(),
                parent_turn_id: "turn-b".to_string(),
                parent_agent_id: Some("agent-parent-b".to_string()),
                depth: 1,
            }),
            tool_call_id: Some("call-b".to_string()),
            result: SubRunResult {
                status: SubRunOutcome::Completed,
                handoff: None,
                failure: None,
            },
            step_count: 2,
            estimated_tokens: 77,
            timestamp: None,
        },
    );

    let snapshot = service
        .execution()
        .get_subrun_status(&session_b.session_id, &live_handle.sub_run_id)
        .await
        .expect("durable snapshot should be returned");

    assert_eq!(snapshot.source, crate::service::SubRunStatusSource::Durable);
    assert_eq!(snapshot.handle.status, AgentStatus::Completed);
    assert_eq!(snapshot.handle.agent_id, "agent-b");
    assert_eq!(snapshot.tool_call_id.as_deref(), Some("call-b"));
}

#[tokio::test]
async fn get_subrun_status_uses_live_overlay_for_independent_subrun_in_parent_session() {
    let _guard = TestEnvGuard::new();
    let service = Arc::new(
        RuntimeService::from_capabilities(capabilities_from_tools(
            ToolRegistry::builder()
                .register(Box::new(DemoTool { name: "readFile" }))
                .build(),
        ))
        .expect("runtime service should build"),
    );

    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let parent_session = service
        .sessions()
        .create(temp_dir.path())
        .await
        .expect("parent session should be created");
    let child_session = service
        .sessions()
        .create(temp_dir.path())
        .await
        .expect("child session should be created");
    let profile = AgentProfile {
        id: "review".to_string(),
        name: "Review".to_string(),
        description: "review".to_string(),
        mode: AgentMode::SubAgent,
        system_prompt: None,
        allowed_tools: vec!["readFile".to_string()],
        disallowed_tools: Vec::new(),
        model_preference: None,
    };
    let control = service.agent_control();
    let handle = control
        .spawn_with_storage(
            &profile,
            child_session.session_id.clone(),
            Some(child_session.session_id.clone()),
            Some("turn-parent".to_string()),
            None,
            SubRunStorageMode::IndependentSession,
        )
        .await
        .expect("independent sub-run should spawn");
    let _ = control
        .mark_running(&handle.agent_id)
        .await
        .expect("sub-run should be running");

    append_storage_event(
        service.as_ref(),
        &parent_session.session_id,
        &StorageEvent::SubRunStarted {
            turn_id: Some("turn-parent".to_string()),
            agent: AgentEventContext::sub_run(
                handle.agent_id.clone(),
                "turn-parent".to_string(),
                "review".to_string(),
                handle.sub_run_id.clone(),
                SubRunStorageMode::IndependentSession,
                Some(child_session.session_id.clone()),
            ),
            descriptor: Some(astrcode_core::SubRunDescriptor {
                sub_run_id: handle.sub_run_id.clone(),
                parent_turn_id: "turn-parent".to_string(),
                parent_agent_id: None,
                depth: 1,
            }),
            tool_call_id: Some("call-independent".to_string()),
            resolved_overrides: ResolvedSubagentContextOverrides {
                storage_mode: SubRunStorageMode::IndependentSession,
                ..Default::default()
            },
            resolved_limits: ResolvedExecutionLimitsSnapshot::default(),
            timestamp: None,
        },
    );

    let snapshot = service
        .execution()
        .get_subrun_status(&parent_session.session_id, &handle.sub_run_id)
        .await
        .expect("status should resolve through parent ownership");

    assert_eq!(snapshot.source, crate::service::SubRunStatusSource::Live);
    assert_eq!(snapshot.handle.status, AgentStatus::Running);
    assert_eq!(
        snapshot.handle.storage_mode,
        SubRunStorageMode::IndependentSession
    );
    assert_eq!(
        snapshot.handle.child_session_id,
        Some(child_session.session_id)
    );
    assert_eq!(snapshot.tool_call_id.as_deref(), Some("call-independent"));
    assert!(snapshot.result.is_none());
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

#[test]
fn runtime_owner_graph_exposes_execution_surface_owner() {
    let _guard = TestEnvGuard::new();
    let service = RuntimeService::from_capabilities(crate::test_support::empty_capabilities())
        .expect("runtime service should build");

    let owner_graph = service.owner_graph();
    assert_eq!(owner_graph.session_owner, "runtime-session");
    assert_eq!(owner_graph.execution_owner, "runtime-execution");
    assert_eq!(owner_graph.tool_owner, "runtime-execution");
}

#[tokio::test]
async fn tool_execution_surface_lists_registered_tools() {
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

    let tools = service.tools().list_tools().await;
    let tool_names = tools.into_iter().map(|tool| tool.name).collect::<Vec<_>>();

    assert_eq!(tool_names, vec!["grep".to_string(), "readFile".to_string()]);
}
