use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use astrcode_core::{
    AgentEventContext, AgentMode, AgentProfile, AgentStatus, AstrError, CancelToken,
    ChildSessionLineageKind, ChildSessionNotificationKind, ExecutionOwner, InvocationKind, Phase,
    ResolvedExecutionLimitsSnapshot, ResolvedSubagentContextOverrides, SessionTurnAcquireResult,
    SpawnAgentParams, StorageEvent, SubRunDescriptor, SubRunFailure, SubRunFailureCode,
    SubRunHandle, SubRunHandoff, SubRunOutcome, SubRunResult, SubRunStorageMode,
    SubagentContextOverrides, Tool, ToolCapabilityMetadata, ToolContext, ToolDefinition,
    ToolEventSink, ToolExecutionResult, UserMessageOrigin, test_support::TestEnvGuard,
};
use astrcode_runtime_agent_loop::{AgentLoop, ProviderFactory};
use astrcode_runtime_agent_tool::{SpawnAgentTool, SubAgentExecutor};
use astrcode_runtime_config::DEFAULT_MAX_CONCURRENT_AGENTS;
use astrcode_runtime_execution::{
    AgentExecutionRequest, build_child_agent_state, derive_child_execution_owner,
    resolve_context_snapshot, resolve_profile_tool_names, resolve_subagent_overrides,
};
use astrcode_runtime_session::prepare_session_execution;
use async_trait::async_trait;
use serde_json::json;

use super::DeferredSubAgentExecutor;
use crate::{
    llm::{EventSink, LlmOutput, LlmProvider, LlmRequest, ModelLimits},
    service::RuntimeService,
    test_support::{capabilities_from_tools, empty_capabilities},
};

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

struct StaticProvider {
    output: LlmOutput,
}

#[async_trait]
impl LlmProvider for StaticProvider {
    fn model_limits(&self) -> ModelLimits {
        ModelLimits {
            context_window: 200_000,
            max_output_tokens: 4_096,
        }
    }

    async fn generate(
        &self,
        _request: LlmRequest,
        _sink: Option<EventSink>,
    ) -> astrcode_core::Result<LlmOutput> {
        Ok(self.output.clone())
    }
}

struct StaticProviderFactory {
    provider: Arc<dyn LlmProvider>,
}

impl ProviderFactory for StaticProviderFactory {
    fn build_for_working_dir(
        &self,
        _working_dir: Option<PathBuf>,
    ) -> astrcode_core::Result<Arc<dyn LlmProvider>> {
        Ok(Arc::clone(&self.provider))
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

async fn install_provider_loop(service: &Arc<RuntimeService>, provider: Arc<dyn LlmProvider>) {
    let loop_ = AgentLoop::from_capabilities(
        Arc::new(StaticProviderFactory { provider }),
        empty_capabilities(),
    );
    *service.loop_.write().await = Arc::new(loop_);
}

async fn install_test_loop(service: &Arc<RuntimeService>, output: LlmOutput) {
    install_provider_loop(service, Arc::new(StaticProvider { output })).await;
}

async fn occupy_parent_turn(service: &Arc<RuntimeService>, session_id: &str, turn_id: &str) {
    let session_state = service
        .ensure_session_loaded(session_id)
        .await
        .expect("session state should load");
    let turn_lease = match service
        .session_manager
        .try_acquire_turn(session_id, turn_id)
        .expect("turn lease acquisition should succeed")
    {
        SessionTurnAcquireResult::Acquired(turn_lease) => turn_lease,
        SessionTurnAcquireResult::Busy(_) => {
            panic!("fresh session should not already be busy")
        },
    };
    prepare_session_execution(
        &session_state,
        session_id,
        turn_id,
        CancelToken::new(),
        turn_lease,
        None,
    )
    .expect("parent turn should be marked busy");
}

fn write_test_agent_profile(working_dir: &std::path::Path, profile_id: &str) {
    let project_agents_dir = working_dir.join(".astrcode").join("agents");
    std::fs::create_dir_all(&project_agents_dir).expect("project agents dir should exist");
    std::fs::write(
        project_agents_dir.join(format!("{profile_id}.md")),
        format!(
            r#"---
name: {profile_id}
description: Resume regression profile
tools: [readFile]
---
恢复时继续沿用 durable 历史。
"#
        ),
    )
    .expect("agent definition should be written");
}

fn extract_spawned_sub_run_id(result: &ToolExecutionResult) -> String {
    result
        .metadata
        .as_ref()
        .and_then(|value| value.get("handoff"))
        .and_then(|value| value.get("artifacts"))
        .and_then(|value| value.as_array())
        .and_then(|artifacts| {
            artifacts
                .iter()
                .find(|artifact| artifact.get("kind") == Some(&json!("subRun")))
                .or_else(|| artifacts.first())
        })
        .and_then(|artifact| artifact.get("id"))
        .and_then(|value| value.as_str())
        .expect("spawnAgent launch should expose sub-run artifact")
        .to_string()
}

#[test]
fn resolve_profile_tool_names_rejects_legacy_aliases() {
    let capabilities = capabilities_from_tools(vec![
        Box::new(DemoTool { name: "readFile" }),
        Box::new(DemoTool { name: "shell" }),
    ]);

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
        RuntimeService::from_capabilities(empty_capabilities())
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
async fn spawn_agent_background_cancellation_releases_concurrency_slots() {
    let _guard = TestEnvGuard::new();
    let service = Arc::new(
        RuntimeService::from_capabilities(capabilities_from_tools(vec![
            Box::new(DemoTool { name: "readFile" }),
            Box::new(DemoTool { name: "grep" }),
        ]))
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
    occupy_parent_turn(&service, &session.session_id, "turn-parent").await;
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
        let sub_run_id = extract_spawned_sub_run_id(&result);
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
            handle.status.is_final(),
            "background sub-run should eventually release its concurrency slot via any final state"
        );
    }
}

#[tokio::test]
async fn spawn_agent_lifecycle_events_persist_parent_tool_call_id() {
    let _guard = TestEnvGuard::new();
    let service = Arc::new(
        RuntimeService::from_capabilities(capabilities_from_tools(vec![
            Box::new(DemoTool { name: "readFile" }),
            Box::new(DemoTool { name: "grep" }),
        ]))
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
    occupy_parent_turn(&service, &session.session_id, "turn-parent").await;
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
    let sub_run_id = extract_spawned_sub_run_id(&result);

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
        RuntimeService::from_capabilities(capabilities_from_tools(vec![
            Box::new(DemoTool { name: "readFile" }),
            Box::new(DemoTool { name: "grep" }),
        ]))
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
    let descriptor = SubRunDescriptor {
        sub_run_id: "subrun-durable".to_string(),
        parent_turn_id: "turn-parent".to_string(),
        parent_agent_id: None,
        depth: 0,
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
            descriptor: Some(descriptor.clone()),
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
            descriptor: Some(descriptor),
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
        RuntimeService::from_capabilities(capabilities_from_tools(vec![
            Box::new(DemoTool { name: "readFile" }),
            Box::new(DemoTool { name: "grep" }),
        ]))
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
async fn get_subrun_status_keeps_storage_mode_parity_for_parent_aborted_independent_subruns() {
    let _guard = TestEnvGuard::new();
    let service = Arc::new(
        RuntimeService::from_capabilities(capabilities_from_tools(vec![
            Box::new(DemoTool { name: "readFile" }),
            Box::new(DemoTool { name: "grep" }),
        ]))
        .expect("runtime service should build"),
    );

    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let session = service
        .sessions()
        .create(temp_dir.path())
        .await
        .expect("session should be created");

    let sub_run_id = "subrun-independent";
    let descriptor = SubRunDescriptor {
        sub_run_id: sub_run_id.to_string(),
        parent_turn_id: "turn-parent".to_string(),
        parent_agent_id: None,
        depth: 0,
    };
    let agent = AgentEventContext::sub_run(
        format!("agent-{sub_run_id}"),
        "turn-parent".to_string(),
        "review".to_string(),
        sub_run_id.to_string(),
        SubRunStorageMode::IndependentSession,
        Some("child-independent".to_string()),
    );
    append_storage_event(
        service.as_ref(),
        &session.session_id,
        &StorageEvent::SubRunStarted {
            turn_id: Some("turn-parent".to_string()),
            agent: agent.clone(),
            descriptor: Some(descriptor.clone()),
            tool_call_id: None,
            resolved_overrides: ResolvedSubagentContextOverrides {
                storage_mode: SubRunStorageMode::IndependentSession,
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
            descriptor: Some(descriptor),
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

    assert_eq!(
        snapshot.handle.storage_mode,
        SubRunStorageMode::IndependentSession
    );
    assert_eq!(snapshot.handle.status, AgentStatus::Cancelled);
    assert_eq!(
        snapshot.result.as_ref().map(|item| item.status.clone()),
        Some(SubRunOutcome::Aborted)
    );
}

#[tokio::test]
async fn get_subrun_status_rejects_live_handle_from_other_session() {
    let _guard = TestEnvGuard::new();
    let service = Arc::new(
        RuntimeService::from_capabilities(capabilities_from_tools(vec![
            Box::new(DemoTool { name: "readFile" }),
            Box::new(DemoTool { name: "grep" }),
        ]))
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
        RuntimeService::from_capabilities(capabilities_from_tools(vec![Box::new(DemoTool {
            name: "readFile",
        })]))
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
        RuntimeService::from_capabilities(capabilities_from_tools(vec![Box::new(DemoTool {
            name: "readFile",
        })]))
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

#[tokio::test]
async fn get_subrun_status_rejects_legacy_shared_history_snapshots() {
    let _guard = TestEnvGuard::new();
    let service = Arc::new(
        RuntimeService::from_capabilities(empty_capabilities())
            .expect("runtime service should build"),
    );

    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let session = service
        .sessions()
        .create(temp_dir.path())
        .await
        .expect("session should be created");

    let legacy_agent = AgentEventContext::sub_run(
        "agent-legacy".to_string(),
        "turn-legacy".to_string(),
        "review".to_string(),
        "subrun-legacy".to_string(),
        SubRunStorageMode::SharedSession,
        None,
    );
    append_storage_event(
        service.as_ref(),
        &session.session_id,
        &StorageEvent::SubRunStarted {
            turn_id: Some("turn-legacy".to_string()),
            agent: legacy_agent.clone(),
            descriptor: None,
            tool_call_id: Some("call-legacy".to_string()),
            resolved_overrides: ResolvedSubagentContextOverrides::default(),
            resolved_limits: ResolvedExecutionLimitsSnapshot::default(),
            timestamp: None,
        },
    );
    append_storage_event(
        service.as_ref(),
        &session.session_id,
        &StorageEvent::SubRunFinished {
            turn_id: Some("turn-legacy".to_string()),
            agent: legacy_agent,
            descriptor: None,
            tool_call_id: Some("call-legacy".to_string()),
            result: SubRunResult {
                status: SubRunOutcome::Completed,
                handoff: None,
                failure: None,
            },
            step_count: 1,
            estimated_tokens: 21,
            timestamp: None,
        },
    );

    let error = service
        .execution()
        .get_subrun_status(&session.session_id, "subrun-legacy")
        .await
        .expect_err("legacy shared-history subrun should be rejected");

    assert!(matches!(error, crate::service::ServiceError::Conflict(_)));
    assert!(
        error
            .to_string()
            .contains("unsupported_legacy_shared_history"),
        "unexpected legacy rejection error: {error}"
    );
}

#[tokio::test]
async fn resume_child_session_replays_existing_child_history_and_mints_new_subrun_id() {
    let _guard = TestEnvGuard::new();
    let service = Arc::new(
        RuntimeService::from_capabilities(capabilities_from_tools(vec![Box::new(DemoTool {
            name: "readFile",
        })]))
        .expect("runtime service should build"),
    );

    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    write_test_agent_profile(temp_dir.path(), "review");

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
    let child_state = service
        .ensure_session_loaded(&child_session.session_id)
        .await
        .expect("child session state should load");
    let child_sink = astrcode_runtime_session::SessionStateEventSink::new(Arc::clone(&child_state))
        .expect("child event sink should build");
    child_sink
        .emit(StorageEvent::UserMessage {
            turn_id: Some("turn-child-old".to_string()),
            agent: AgentEventContext::root_execution("agent-child", "review"),
            content: "先完成第一轮分析".to_string(),
            origin: UserMessageOrigin::User,
            timestamp: chrono::Utc::now(),
        })
        .expect("old child user message should persist");
    child_sink
        .emit(StorageEvent::AssistantFinal {
            turn_id: Some("turn-child-old".to_string()),
            agent: AgentEventContext::root_execution("agent-child", "review"),
            content: "已经整理出第一版结论".to_string(),
            reasoning_content: None,
            reasoning_signature: None,
            timestamp: Some(chrono::Utc::now()),
        })
        .expect("old child assistant message should persist");
    child_sink
        .emit(StorageEvent::TurnDone {
            turn_id: Some("turn-child-old".to_string()),
            agent: AgentEventContext::root_execution("agent-child", "review"),
            timestamp: chrono::Utc::now(),
            reason: Some("completed".to_string()),
        })
        .expect("old child turn done should persist");

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
    let original_handle = service
        .agent_control()
        .spawn_with_storage(
            &profile,
            child_session.session_id.clone(),
            Some(child_session.session_id.clone()),
            Some("turn-parent".to_string()),
            None,
            SubRunStorageMode::IndependentSession,
        )
        .await
        .expect("child execution should spawn");
    let _ = service
        .agent_control()
        .mark_running(&original_handle.agent_id)
        .await
        .expect("child execution should become running");
    let _ = service
        .agent_control()
        .mark_completed(&original_handle.agent_id)
        .await
        .expect("child execution should complete");

    let parent_state = service
        .ensure_session_loaded(&parent_session.session_id)
        .await
        .expect("parent session state should load");
    parent_state
        .upsert_child_session_node(astrcode_runtime_execution::build_child_session_node(
            &original_handle,
            &parent_session.session_id,
            "turn-parent",
            Some("call-resume".to_string()),
        ))
        .expect("child node should be persisted");

    let context = ToolContext::new(
        parent_session.session_id.clone(),
        temp_dir.path().to_path_buf(),
        CancelToken::new(),
    )
    .with_turn_id("turn-parent")
    .with_tool_call_id("call-resume");

    let (resumed_handle, running_result) = service
        .execution()
        .resume_child_session(
            &original_handle.agent_id,
            Some("继续补完遗漏检查".to_string()),
            &context,
        )
        .await
        .expect("resume should succeed");

    assert_eq!(running_result.status, SubRunOutcome::Running);
    assert_eq!(resumed_handle.agent_id, original_handle.agent_id);
    assert_ne!(resumed_handle.sub_run_id, original_handle.sub_run_id);
    assert_eq!(
        resumed_handle.child_session_id.as_deref(),
        Some(child_session.session_id.as_str())
    );

    let live_handle = service
        .agent_control()
        .get(&original_handle.agent_id)
        .await
        .expect("latest execution should be accessible via stable agent id");
    assert_eq!(live_handle.sub_run_id, resumed_handle.sub_run_id);
    let historical = service
        .agent_control()
        .get(&original_handle.sub_run_id)
        .await
        .expect("historical execution should remain queryable by old sub-run id");
    assert_eq!(historical.status, AgentStatus::Completed);

    let child_snapshot = service
        .ensure_session_loaded(&child_session.session_id)
        .await
        .expect("child session should stay loaded")
        .snapshot_projected_state()
        .expect("child snapshot should rebuild");
    assert_eq!(child_snapshot.session_id, child_session.session_id);
    assert!(
        child_snapshot.messages.iter().any(|message| {
            matches!(
                message,
                astrcode_core::LlmMessage::Assistant { content, .. }
                    if content == "已经整理出第一版结论"
            )
        }),
        "replayed history should be preserved before the new resume message"
    );
    assert!(
        child_snapshot.messages.iter().any(|message| {
            matches!(
                message,
                astrcode_core::LlmMessage::User { content, .. }
                    if content == "继续补完遗漏检查"
            )
        }),
        "resume message should be appended on top of the replayed history"
    );

    let resumed_node = parent_state
        .child_session_node(&resumed_handle.sub_run_id)
        .expect("child node lookup should succeed")
        .expect("resumed child node should exist");
    assert_eq!(resumed_node.child_session_id, child_session.session_id);
    assert_eq!(resumed_node.lineage_kind, ChildSessionLineageKind::Resume);

    let parent_events = crate::service::session::load_events(
        Arc::clone(&service.session_manager),
        &parent_session.session_id,
    )
    .await
    .expect("parent session events should load");
    assert!(parent_events.iter().any(|stored| {
        matches!(
            &stored.event,
            StorageEvent::SubRunStarted { descriptor: Some(descriptor), .. }
                if descriptor.sub_run_id == resumed_handle.sub_run_id
        )
    }));
    assert!(parent_events.iter().any(|stored| {
        matches!(
            &stored.event,
            StorageEvent::ChildSessionNotification { notification, .. }
                if notification.kind == ChildSessionNotificationKind::Resumed
                    && notification.child_ref.sub_run_id == resumed_handle.sub_run_id
                    && notification.open_session_id == child_session.session_id
        )
    }));
}

#[tokio::test]
async fn resume_child_session_rejects_lineage_mismatch_before_minting_new_execution() {
    let _guard = TestEnvGuard::new();
    let service = Arc::new(
        RuntimeService::from_capabilities(capabilities_from_tools(vec![Box::new(DemoTool {
            name: "readFile",
        })]))
        .expect("runtime service should build"),
    );

    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    write_test_agent_profile(temp_dir.path(), "review");

    let parent_session = service
        .sessions()
        .create(temp_dir.path())
        .await
        .expect("parent session should be created");
    let other_parent_session = service
        .sessions()
        .create(temp_dir.path())
        .await
        .expect("other parent session should be created");
    let child_session = service
        .sessions()
        .create(temp_dir.path())
        .await
        .expect("child session should be created");

    append_storage_event(
        service.as_ref(),
        &child_session.session_id,
        &StorageEvent::UserMessage {
            turn_id: Some("turn-child-old".to_string()),
            agent: AgentEventContext::root_execution("agent-child", "review"),
            content: "旧历史".to_string(),
            origin: UserMessageOrigin::User,
            timestamp: chrono::Utc::now(),
        },
    );

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
    let original_handle = service
        .agent_control()
        .spawn_with_storage(
            &profile,
            child_session.session_id.clone(),
            Some(child_session.session_id.clone()),
            Some("turn-parent".to_string()),
            None,
            SubRunStorageMode::IndependentSession,
        )
        .await
        .expect("child execution should spawn");
    let _ = service
        .agent_control()
        .mark_running(&original_handle.agent_id)
        .await
        .expect("child execution should become running");
    let _ = service
        .agent_control()
        .mark_completed(&original_handle.agent_id)
        .await
        .expect("child execution should complete");

    let parent_state = service
        .ensure_session_loaded(&parent_session.session_id)
        .await
        .expect("parent session state should load");
    parent_state
        .upsert_child_session_node(astrcode_runtime_execution::build_child_session_node(
            &original_handle,
            &other_parent_session.session_id,
            "turn-parent",
            Some("call-resume".to_string()),
        ))
        .expect("mismatched child node should be persisted");

    let context = ToolContext::new(
        parent_session.session_id.clone(),
        temp_dir.path().to_path_buf(),
        CancelToken::new(),
    )
    .with_turn_id("turn-parent")
    .with_tool_call_id("call-resume");

    let error = service
        .execution()
        .resume_child_session(
            &original_handle.agent_id,
            Some("继续执行".to_string()),
            &context,
        )
        .await
        .expect_err("lineage mismatch should reject resume");

    assert!(matches!(error, crate::service::ServiceError::Conflict(_)));
    assert!(
        error
            .to_string()
            .contains("lineage_mismatch_parent_session"),
        "unexpected error: {error}"
    );

    let latest_handle = service
        .agent_control()
        .get(&original_handle.agent_id)
        .await
        .expect("agent should still resolve to the previous execution");
    assert_eq!(latest_handle.sub_run_id, original_handle.sub_run_id);
    assert_eq!(latest_handle.status, AgentStatus::Completed);

    let diagnostics = service.observability.snapshot().execution_diagnostics;
    assert_eq!(diagnostics.lineage_mismatch_parent_session, 1);

    let parent_events = crate::service::session::load_events(
        Arc::clone(&service.session_manager),
        &parent_session.session_id,
    )
    .await
    .expect("parent session events should load");
    assert!(parent_events.iter().any(|stored| {
        matches!(
            &stored.event,
            StorageEvent::Error { message, .. }
                if message.contains("lineage_mismatch_parent_session")
        )
    }));
}

#[tokio::test]
async fn resume_child_session_rejects_empty_child_history_as_unsafe_resume() {
    let _guard = TestEnvGuard::new();
    let service = Arc::new(
        RuntimeService::from_capabilities(capabilities_from_tools(vec![Box::new(DemoTool {
            name: "readFile",
        })]))
        .expect("runtime service should build"),
    );

    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    write_test_agent_profile(temp_dir.path(), "review");

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
    let original_handle = service
        .agent_control()
        .spawn_with_storage(
            &profile,
            child_session.session_id.clone(),
            Some(child_session.session_id.clone()),
            Some("turn-parent".to_string()),
            None,
            SubRunStorageMode::IndependentSession,
        )
        .await
        .expect("child execution should spawn");
    let _ = service
        .agent_control()
        .mark_running(&original_handle.agent_id)
        .await
        .expect("child execution should become running");
    let _ = service
        .agent_control()
        .mark_completed(&original_handle.agent_id)
        .await
        .expect("child execution should complete");

    let parent_state = service
        .ensure_session_loaded(&parent_session.session_id)
        .await
        .expect("parent session state should load");
    parent_state
        .upsert_child_session_node(astrcode_runtime_execution::build_child_session_node(
            &original_handle,
            &parent_session.session_id,
            "turn-parent",
            Some("call-resume".to_string()),
        ))
        .expect("child node should be persisted");

    let context = ToolContext::new(
        parent_session.session_id.clone(),
        temp_dir.path().to_path_buf(),
        CancelToken::new(),
    )
    .with_turn_id("turn-parent")
    .with_tool_call_id("call-resume");

    let error = service
        .execution()
        .resume_child_session(
            &original_handle.agent_id,
            Some("继续执行".to_string()),
            &context,
        )
        .await
        .expect_err("empty child history should be rejected");

    assert!(matches!(error, crate::service::ServiceError::Conflict(_)));
    assert!(
        error.to_string().contains("unsafe_resume_rejected"),
        "unexpected error: {error}"
    );
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
fn child_task_payload_stays_task_only_while_parent_context_moves_to_inherited_blocks() {
    let parent_state = astrcode_core::AgentState {
        session_id: "session-parent".to_string(),
        working_dir: std::env::temp_dir(),
        messages: vec![
            astrcode_core::LlmMessage::User {
                content: astrcode_core::format_compact_summary("父会话摘要").to_string(),
                origin: astrcode_core::UserMessageOrigin::CompactSummary,
            },
            astrcode_core::LlmMessage::User {
                content: "检查 auth 缓存".to_string(),
                origin: astrcode_core::UserMessageOrigin::User,
            },
            astrcode_core::LlmMessage::Assistant {
                content: "先看登录链路".to_string(),
                tool_calls: Vec::new(),
                reasoning: None,
            },
        ],
        phase: astrcode_core::Phase::Idle,
        turn_count: 3,
    };
    let request = AgentExecutionRequest {
        subagent_type: Some("review".to_string()),
        description: "检查缓存".to_string(),
        prompt: "排查 auth 模块".to_string(),
        context: Some("重点看命中率".to_string()),
        context_overrides: None,
    };
    let overrides = ResolvedSubagentContextOverrides {
        include_compact_summary: true,
        include_recent_tail: true,
        ..ResolvedSubagentContextOverrides::default()
    };

    let snapshot = resolve_context_snapshot(&request, Some(&parent_state), &overrides);
    let child_state = build_child_agent_state(
        "session-child",
        std::env::temp_dir(),
        &snapshot.task_payload,
    );

    assert_eq!(
        snapshot.inherited_compact_summary.as_deref(),
        Some("父会话摘要")
    );
    assert_eq!(
        snapshot.inherited_recent_tail,
        vec!["- user: 检查 auth 缓存", "- assistant: 先看登录链路"]
    );
    assert!(matches!(
        &child_state.messages[0],
        astrcode_core::LlmMessage::User { content, .. }
            if content.contains("# Task\n排查 auth 模块")
                && content.contains("# Context\n重点看命中率")
                && !content.contains("父会话摘要")
                && !content.contains("检查 auth 缓存")
    ));
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
async fn parent_turn_completion_does_not_cancel_running_child_session() {
    let _guard = TestEnvGuard::new();
    let service = Arc::new(
        RuntimeService::from_capabilities(empty_capabilities())
            .expect("runtime service should build"),
    );

    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let session = service
        .sessions()
        .create(temp_dir.path())
        .await
        .expect("session should be created");
    let session_state = service
        .ensure_session_loaded(&session.session_id)
        .await
        .expect("session state should load");

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
    let child = service
        .agent_control()
        .spawn(
            &profile,
            &session.session_id,
            Some("turn-parent".to_string()),
            None,
        )
        .await
        .expect("child sub-run should spawn");
    let _ = service
        .agent_control()
        .mark_running(&child.agent_id)
        .await
        .expect("child should transition to running");
    let child_cancel = service
        .agent_control()
        .cancel_token(&child.agent_id)
        .await
        .expect("child cancel token should exist");

    session_state
        .running
        .store(true, std::sync::atomic::Ordering::SeqCst);
    {
        let mut active_turn = session_state
            .active_turn_id
            .lock()
            .expect("active turn lock");
        *active_turn = Some("turn-parent".to_string());
    }

    crate::service::turn::complete_session_execution(
        &session_state,
        Phase::Idle,
        &service.agent_control(),
    )
    .await;

    let refreshed = service
        .agent_control()
        .get(&child.agent_id)
        .await
        .expect("child should remain in registry");
    assert_eq!(refreshed.status, AgentStatus::Running);
    assert!(!child_cancel.is_cancelled());
}

#[tokio::test]
async fn spawn_agent_terminal_delivery_notification_is_emitted_once() {
    let _guard = TestEnvGuard::new();
    let service = Arc::new(
        RuntimeService::from_capabilities(capabilities_from_tools(vec![
            Box::new(DemoTool { name: "readFile" }),
            Box::new(DemoTool { name: "grep" }),
        ]))
        .expect("runtime service should build"),
    );
    install_test_loop(
        &service,
        LlmOutput {
            content: "terminal delivery".to_string(),
            ..LlmOutput::default()
        },
    )
    .await;
    let executor = Arc::new(DeferredSubAgentExecutor::default());
    executor.bind(&service);
    let tool = SpawnAgentTool::new(executor);

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
    .with_turn_id("turn-parent");

    let launched = tool
        .execute(
            "call-terminal-once".to_string(),
            json!({
                "type": "plan",
                "description": "verify terminal delivery once",
                "prompt": "return quickly"
            }),
            &context,
        )
        .await
        .expect("spawnAgent launch should succeed");

    let sub_run_id = extract_spawned_sub_run_id(&launched);

    let _ = service
        .agent_control()
        .wait(&sub_run_id)
        .await
        .expect("child sub-run should reach a terminal state");

    let terminal_count = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let events = crate::service::session::load_events(
                Arc::clone(&service.session_manager),
                &session.session_id,
            )
            .await
            .expect("session events should load");
            let count = events
                .iter()
                .filter(|stored| {
                    matches!(
                        &stored.event,
                        StorageEvent::ChildSessionNotification { notification, .. }
                            if notification.child_ref.sub_run_id == sub_run_id
                                && matches!(
                                    notification.kind,
                                    ChildSessionNotificationKind::Delivered
                                        | ChildSessionNotificationKind::Failed
                                        | ChildSessionNotificationKind::Closed
                                )
                    )
                })
                .count();
            if count > 0 {
                break count;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("terminal notification should appear in time");

    assert_eq!(terminal_count, 1, "terminal delivery must be emitted once");
}

// ─── 状态投影与重激活测试 ──────────────────────────────────

#[test]
fn project_child_terminal_delivery_maps_token_exceeded_to_delivered_completed() {
    // TokenExceeded 语义上视为成功交付：虽然因 token 上限截断，
    // 但已收集到部分结果，应映射为 Delivered/Completed。
    let result = SubRunResult {
        status: SubRunOutcome::TokenExceeded,
        handoff: Some(SubRunHandoff {
            summary: "达到 token 上限，已收集部分结果".to_string(),
            findings: vec!["中间发现1".to_string()],
            artifacts: vec![],
        }),
        failure: None,
    };
    let projection = super::status::project_child_terminal_delivery(&result);
    assert_eq!(projection.kind, ChildSessionNotificationKind::Delivered);
    assert_eq!(projection.status, AgentStatus::Completed);
    assert_eq!(projection.summary, "达到 token 上限，已收集部分结果");
    assert_eq!(
        projection.final_reply_excerpt.as_deref(),
        Some("达到 token 上限，已收集部分结果")
    );
}

#[test]
fn project_child_terminal_delivery_maps_token_exceeded_without_handoff() {
    // TokenExceeded + 无 handoff 时，summary 应走默认回退。
    let result = SubRunResult {
        status: SubRunOutcome::TokenExceeded,
        handoff: None,
        failure: None,
    };
    let projection = super::status::project_child_terminal_delivery(&result);
    assert_eq!(projection.kind, ChildSessionNotificationKind::Delivered);
    assert_eq!(projection.status, AgentStatus::Completed);
    assert_eq!(projection.summary, "子 Agent 已完成，但没有返回可读总结。");
    assert!(projection.final_reply_excerpt.is_none());
}

#[test]
fn project_child_terminal_delivery_uses_failure_display_message_as_summary_fallback() {
    // handoff 为空但 failure 有 display_message 时，summary 应使用 display_message。
    let result = SubRunResult {
        status: SubRunOutcome::Failed,
        handoff: None,
        failure: Some(SubRunFailure {
            code: SubRunFailureCode::Internal,
            display_message: "模型调用失败：配额不足".to_string(),
            technical_message: "insufficient_quota".to_string(),
            retryable: false,
        }),
    };
    let projection = super::status::project_child_terminal_delivery(&result);
    assert_eq!(projection.kind, ChildSessionNotificationKind::Failed);
    assert_eq!(projection.status, AgentStatus::Failed);
    assert_eq!(projection.summary, "模型调用失败：配额不足");
    assert!(projection.final_reply_excerpt.is_none());
}

#[test]
fn project_child_terminal_delivery_returns_default_summary_when_no_handoff_or_failure() {
    // 既无 handoff 也无 failure 时，使用状态对应的中文默认文案。
    let result = SubRunResult {
        status: SubRunOutcome::Completed,
        handoff: None,
        failure: None,
    };
    let projection = super::status::project_child_terminal_delivery(&result);
    assert_eq!(projection.kind, ChildSessionNotificationKind::Delivered);
    assert_eq!(projection.status, AgentStatus::Completed);
    assert_eq!(projection.summary, "子 Agent 已完成，但没有返回可读总结。");
    assert!(projection.final_reply_excerpt.is_none());
}

#[tokio::test]
async fn reactivate_parent_buffers_delivery_while_parent_turn_is_busy() {
    let _guard = TestEnvGuard::new();
    let service = Arc::new(
        RuntimeService::from_capabilities(empty_capabilities())
            .expect("runtime service should build"),
    );
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let session = service
        .sessions()
        .create(temp_dir.path())
        .await
        .expect("session should be created");
    let session_state = service
        .ensure_session_loaded(&session.session_id)
        .await
        .expect("session state should load");

    let turn_lease = match service
        .session_manager
        .try_acquire_turn(&session.session_id, "turn-busy")
        .expect("turn lease acquisition should succeed")
    {
        astrcode_core::SessionTurnAcquireResult::Acquired(turn_lease) => turn_lease,
        astrcode_core::SessionTurnAcquireResult::Busy(_) => {
            panic!("fresh session should not already be busy")
        },
    };
    prepare_session_execution(
        &session_state,
        &session.session_id,
        "turn-busy",
        CancelToken::new(),
        turn_lease,
        None,
    )
    .expect("busy turn should be prepared");

    let notification = make_test_notification("child-reactivate-busy", "turn-child");
    service
        .execution()
        .reactivate_parent_agent_if_idle(&session.session_id, "turn-parent", &notification)
        .await;

    assert_eq!(
        service
            .agent_control
            .pending_parent_delivery_count(&session.session_id)
            .await,
        1
    );
    assert!(
        !session_contains_reactivation_prompt(&service, &session.session_id).await,
        "busy-parent buffering must not persist mechanism user messages"
    );
}

#[tokio::test]
async fn reactivate_parent_drains_buffer_after_busy_parent_turn_completes() {
    let _guard = TestEnvGuard::new();
    let service = Arc::new(
        RuntimeService::from_capabilities(empty_capabilities())
            .expect("runtime service should build"),
    );
    // 需要 LLM loop 才能让 wake turn 成功执行并消费 delivery
    install_test_loop(
        &service,
        LlmOutput {
            content: "drain consumed".to_string(),
            ..LlmOutput::default()
        },
    )
    .await;
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let session = service
        .sessions()
        .create(temp_dir.path())
        .await
        .expect("session should be created");
    let session_state = service
        .ensure_session_loaded(&session.session_id)
        .await
        .expect("session state should load");

    let turn_lease = match service
        .session_manager
        .try_acquire_turn(&session.session_id, "turn-busy")
        .expect("turn lease acquisition should succeed")
    {
        astrcode_core::SessionTurnAcquireResult::Acquired(turn_lease) => turn_lease,
        astrcode_core::SessionTurnAcquireResult::Busy(_) => {
            panic!("fresh session should not already be busy")
        },
    };
    prepare_session_execution(
        &session_state,
        &session.session_id,
        "turn-busy",
        CancelToken::new(),
        turn_lease,
        None,
    )
    .expect("busy turn should be prepared");

    let notification = make_test_notification("child-reactivate-drain", "turn-child");
    service
        .execution()
        .reactivate_parent_agent_if_idle(&session.session_id, "turn-parent", &notification)
        .await;
    assert_eq!(
        service
            .agent_control
            .pending_parent_delivery_count(&session.session_id)
            .await,
        1
    );

    crate::service::turn::complete_session_execution(
        &session_state,
        Phase::Idle,
        &service.agent_control(),
    )
    .await;
    service
        .execution()
        .try_start_parent_delivery_turn(&session.session_id)
        .await
        .expect("wake turn scheduling should succeed");

    tokio::time::timeout(Duration::from_secs(5), async {
        while service
            .agent_control
            .pending_parent_delivery_count(&session.session_id)
            .await
            > 0
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("buffered delivery should be consumed by wake turn");
    assert!(
        !session_contains_reactivation_prompt(&service, &session.session_id).await,
        "wake turn must consume runtime-only delivery input"
    );
}

#[tokio::test]
async fn reactivate_parent_idle_wake_turn_uses_runtime_only_delivery_input() {
    let _guard = TestEnvGuard::new();
    let service = Arc::new(
        RuntimeService::from_capabilities(empty_capabilities())
            .expect("runtime service should build"),
    );
    // 需要 LLM loop 才能让 wake turn 成功执行并消费 delivery
    install_test_loop(
        &service,
        LlmOutput {
            content: "delivery consumed".to_string(),
            ..LlmOutput::default()
        },
    )
    .await;
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let session = service
        .sessions()
        .create(temp_dir.path())
        .await
        .expect("session should be created");

    let notification = make_test_notification("child-reactivate-origin", "turn-child");
    service
        .execution()
        .reactivate_parent_agent_if_idle(&session.session_id, "turn-parent", &notification)
        .await;

    tokio::time::timeout(Duration::from_secs(5), async {
        while service
            .agent_control
            .pending_parent_delivery_count(&session.session_id)
            .await
            > 0
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("idle wake turn should consume queued delivery");

    assert!(
        !session_contains_reactivation_prompt(&service, &session.session_id).await,
        "idle wake turn must not persist reactivationPrompt user messages"
    );
}

#[tokio::test]
async fn reactivate_parent_failed_wake_turn_requeues_delivery_for_retry() {
    let _guard = TestEnvGuard::new();
    let service = Arc::new(
        RuntimeService::from_capabilities(empty_capabilities())
            .expect("runtime service should build"),
    );
    install_test_loop(
        &service,
        LlmOutput {
            content: String::new(),
            ..LlmOutput::default()
        },
    )
    .await;
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let session = service
        .sessions()
        .create(temp_dir.path())
        .await
        .expect("session should be created");

    let notification = make_test_notification("child-reactivate-failed", "turn-child");
    service
        .execution()
        .reactivate_parent_agent_if_idle(&session.session_id, "turn-parent", &notification)
        .await;

    tokio::time::timeout(Duration::from_secs(5), async {
        while service.running_session_ids().contains(&session.session_id) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("failed wake turn should release parent session running state");
    assert_eq!(
        service
            .agent_control
            .pending_parent_delivery_count(&session.session_id)
            .await,
        1,
        "failed wake turn must keep the delivery queued for retry"
    );
    assert!(
        !session_contains_reactivation_prompt(&service, &session.session_id).await,
        "failed wake turn must not fall back to durable reactivationPrompt messages"
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        !service.running_session_ids().contains(&session.session_id),
        "failed wake turn must stop draining instead of hot-looping on the same queued delivery"
    );

    install_test_loop(
        &service,
        LlmOutput {
            content: "delivery consumed".to_string(),
            ..LlmOutput::default()
        },
    )
    .await;
    service
        .execution()
        .try_start_parent_delivery_turn(&session.session_id)
        .await
        .expect("retry wake turn should schedule successfully");
    tokio::time::timeout(Duration::from_secs(5), async {
        while service
            .agent_control
            .pending_parent_delivery_count(&session.session_id)
            .await
            > 0
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("successful retry should eventually consume the queued delivery");
}

async fn session_contains_reactivation_prompt(
    service: &Arc<RuntimeService>,
    session_id: &str,
) -> bool {
    crate::service::session::load_events(Arc::clone(&service.session_manager), session_id)
        .await
        .expect("session events should load")
        .into_iter()
        .any(|stored| {
            matches!(
                stored.event,
                StorageEvent::UserMessage {
                    origin: UserMessageOrigin::ReactivationPrompt,
                    ..
                }
            )
        })
}

/// 构造用于重激活测试的 ChildSessionNotification。
fn make_test_notification(
    child_agent_id: &str,
    _child_turn_id: &str,
) -> astrcode_core::ChildSessionNotification {
    astrcode_core::ChildSessionNotification {
        notification_id: format!("notif-{child_agent_id}"),
        child_ref: astrcode_core::ChildAgentRef {
            agent_id: child_agent_id.to_string(),
            session_id: "session-child".to_string(),
            sub_run_id: format!("subrun-{child_agent_id}"),
            parent_agent_id: None,
            lineage_kind: astrcode_core::ChildSessionLineageKind::Spawn,
            status: AgentStatus::Completed,
            openable: true,
            open_session_id: "session-child".to_string(),
        },
        kind: ChildSessionNotificationKind::Delivered,
        summary: "子 agent 完成".to_string(),
        status: AgentStatus::Completed,
        open_session_id: "session-child".to_string(),
        source_tool_call_id: None,
        final_reply_excerpt: None,
    }
}
