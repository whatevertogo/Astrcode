use astrcode_core::{
    AgentEventContext, AgentLifecycleStatus, AgentMailboxEnvelope, AgentMode, AgentProfile,
    AgentTurnOutcome, EventLogWriter, MailboxQueuedPayload, PluginHealth, PluginState, Result,
    StorageEvent, StorageEventPayload, Tool, ToolCapabilityMetadata, ToolContext, ToolDefinition,
    ToolExecutionResult, UserMessageOrigin, plugin::PluginEntry,
};
use astrcode_protocol::{
    capability::{CapabilityDescriptor, CapabilityKind, SideEffectLevel, StabilityLevel},
    http::{
        AgentExecuteResponseDto, AgentProfileDto, ConfigReloadResponse, PromptAcceptedResponse,
        PromptRequest, SessionHistoryResponseDto, SubRunStatusDto, SubRunStatusSourceDto,
        SubRunStorageModeDto,
    },
};
use astrcode_runtime::{Config, ModelConfig, Profile, RuntimeConfig, config, save_config};
use astrcode_runtime_registry::{CapabilityRouter, ToolCapabilityInvoker};
use astrcode_storage::session::EventLog;
use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use chrono::Utc;
use serde_json::json;
use tower::ServiceExt;

use crate::{
    AUTH_HEADER_NAME,
    routes::build_api_router,
    test_support::{append_session_events, test_state, test_state_with_capabilities},
};

struct DemoReadTool;

// Why: Quickstart 场景 B/C 依赖一份可复用的层级子执行样本来验证
// history/events 的 scope 过滤与 legacy 降级边界。
fn seed_shared_subrun_session(session_id: &str, working_dir: &std::path::Path) {
    let mut log =
        EventLog::create(session_id, working_dir).expect("session file should be created");
    let root = AgentEventContext::root_execution("root-agent", "primary");
    let sub_a = AgentEventContext::sub_run(
        "agent-a",
        "turn-root",
        "review",
        "sub-a",
        None,
        astrcode_core::SubRunStorageMode::IndependentSession,
        None,
    );
    let sub_b = AgentEventContext::sub_run(
        "agent-b",
        "turn-a",
        "review",
        "sub-b",
        Some("sub-a".to_string()),
        astrcode_core::SubRunStorageMode::IndependentSession,
        None,
    );
    let sub_c = AgentEventContext::sub_run(
        "agent-c",
        "turn-b",
        "review",
        "sub-c",
        Some("sub-b".to_string()),
        astrcode_core::SubRunStorageMode::IndependentSession,
        None,
    );

    for event in [
        StorageEvent {
            turn_id: None,
            agent: AgentEventContext::default(),
            payload: StorageEventPayload::SessionStart {
                session_id: session_id.to_string(),
                timestamp: Utc::now(),
                working_dir: working_dir.display().to_string(),
                parent_session_id: None,
                parent_storage_seq: None,
            },
        },
        StorageEvent {
            turn_id: Some("turn-root".to_string()),
            agent: root,
            payload: StorageEventPayload::UserMessage {
                content: "root".to_string(),
                origin: UserMessageOrigin::User,
                timestamp: Utc::now(),
            },
        },
        StorageEvent {
            turn_id: Some("turn-root".to_string()),
            agent: sub_a.clone(),
            payload: StorageEventPayload::SubRunStarted {
                tool_call_id: None,
                resolved_overrides: astrcode_core::ResolvedSubagentContextOverrides::default(),
                resolved_limits: astrcode_core::ResolvedExecutionLimitsSnapshot::default(),
                timestamp: Some(Utc::now()),
            },
        },
        StorageEvent {
            turn_id: Some("turn-a".to_string()),
            agent: sub_a.clone(),
            payload: StorageEventPayload::UserMessage {
                content: "sub-a".to_string(),
                origin: UserMessageOrigin::User,
                timestamp: Utc::now(),
            },
        },
        StorageEvent {
            turn_id: Some("turn-a".to_string()),
            agent: sub_b.clone(),
            payload: StorageEventPayload::SubRunStarted {
                tool_call_id: None,
                resolved_overrides: astrcode_core::ResolvedSubagentContextOverrides::default(),
                resolved_limits: astrcode_core::ResolvedExecutionLimitsSnapshot::default(),
                timestamp: Some(Utc::now()),
            },
        },
        StorageEvent {
            turn_id: Some("turn-b".to_string()),
            agent: sub_b.clone(),
            payload: StorageEventPayload::UserMessage {
                content: "sub-b".to_string(),
                origin: UserMessageOrigin::User,
                timestamp: Utc::now(),
            },
        },
        StorageEvent {
            turn_id: Some("turn-b".to_string()),
            agent: sub_c,
            payload: StorageEventPayload::SubRunStarted {
                tool_call_id: None,
                resolved_overrides: astrcode_core::ResolvedSubagentContextOverrides::default(),
                resolved_limits: astrcode_core::ResolvedExecutionLimitsSnapshot::default(),
                timestamp: Some(Utc::now()),
            },
        },
    ] {
        log.append(&event).expect("event should append");
    }
}

fn seed_finished_subrun_session(session_id: &str, working_dir: &std::path::Path) {
    let mut log =
        EventLog::create(session_id, working_dir).expect("session file should be created");
    let sub = AgentEventContext::sub_run(
        "agent-durable",
        "turn-root",
        "review",
        "sub-durable",
        None,
        astrcode_core::SubRunStorageMode::IndependentSession,
        Some("child-durable".to_string()),
    );

    for event in [
        StorageEvent {
            turn_id: None,
            agent: AgentEventContext::default(),
            payload: StorageEventPayload::SessionStart {
                session_id: session_id.to_string(),
                timestamp: Utc::now(),
                working_dir: working_dir.display().to_string(),
                parent_session_id: None,
                parent_storage_seq: None,
            },
        },
        StorageEvent {
            turn_id: Some("turn-root".to_string()),
            agent: sub.clone(),
            payload: StorageEventPayload::SubRunStarted {
                tool_call_id: Some("call-durable".to_string()),
                resolved_overrides: astrcode_core::ResolvedSubagentContextOverrides::default(),
                resolved_limits: astrcode_core::ResolvedExecutionLimitsSnapshot::default(),
                timestamp: Some(Utc::now()),
            },
        },
        StorageEvent {
            turn_id: Some("turn-root".to_string()),
            agent: sub,
            payload: StorageEventPayload::SubRunFinished {
                tool_call_id: Some("call-durable".to_string()),
                result: astrcode_core::SubRunResult {
                    lifecycle: astrcode_core::AgentLifecycleStatus::Terminated,
                    last_turn_outcome: Some(astrcode_core::AgentTurnOutcome::Completed),
                    handoff: Some(astrcode_core::SubRunHandoff {
                        summary: "done".to_string(),
                        findings: vec!["ok".to_string()],
                        artifacts: Vec::new(),
                    }),
                    failure: None,
                },
                step_count: 2,
                estimated_tokens: 120,
                timestamp: Some(Utc::now()),
            },
        },
    ] {
        log.append(&event).expect("event should append");
    }
}

fn seed_legacy_subrun_session(session_id: &str, working_dir: &std::path::Path) {
    let mut log =
        EventLog::create(session_id, working_dir).expect("session file should be created");
    let sub = AgentEventContext::sub_run(
        "agent-legacy",
        "turn-legacy",
        "review",
        "sub-legacy",
        None,
        astrcode_core::SubRunStorageMode::IndependentSession,
        None,
    );

    for event in [
        StorageEvent {
            turn_id: None,
            agent: AgentEventContext::default(),
            payload: StorageEventPayload::SessionStart {
                session_id: session_id.to_string(),
                timestamp: Utc::now(),
                working_dir: working_dir.display().to_string(),
                parent_session_id: None,
                parent_storage_seq: None,
            },
        },
        StorageEvent {
            turn_id: Some("turn-legacy".to_string()),
            agent: sub.clone(),
            payload: StorageEventPayload::SubRunStarted {
                tool_call_id: Some("call-legacy".to_string()),
                resolved_overrides: astrcode_core::ResolvedSubagentContextOverrides::default(),
                resolved_limits: astrcode_core::ResolvedExecutionLimitsSnapshot::default(),
                timestamp: Some(Utc::now()),
            },
        },
        StorageEvent {
            turn_id: Some("turn-legacy".to_string()),
            agent: sub,
            payload: StorageEventPayload::SubRunFinished {
                tool_call_id: Some("call-legacy".to_string()),
                result: astrcode_core::SubRunResult {
                    lifecycle: astrcode_core::AgentLifecycleStatus::Terminated,
                    last_turn_outcome: Some(astrcode_core::AgentTurnOutcome::Completed),
                    handoff: None,
                    failure: None,
                },
                step_count: 1,
                estimated_tokens: 24,
                timestamp: Some(Utc::now()),
            },
        },
    ] {
        log.append(&event).expect("event should append");
    }
}

fn seed_parent_abort_storage_mode_parity_session(session_id: &str, working_dir: &std::path::Path) {
    let mut log =
        EventLog::create(session_id, working_dir).expect("session file should be created");
    let shared = AgentEventContext::sub_run(
        "agent-shared",
        "turn-parent",
        "review",
        "sub-shared",
        None,
        astrcode_core::SubRunStorageMode::IndependentSession,
        None,
    );
    let independent = AgentEventContext::sub_run(
        "agent-independent",
        "turn-parent",
        "review",
        "sub-independent",
        None,
        astrcode_core::SubRunStorageMode::IndependentSession,
        Some("child-independent".to_string()),
    );

    for event in [
        StorageEvent {
            turn_id: None,
            agent: AgentEventContext::default(),
            payload: StorageEventPayload::SessionStart {
                session_id: session_id.to_string(),
                timestamp: Utc::now(),
                working_dir: working_dir.display().to_string(),
                parent_session_id: None,
                parent_storage_seq: None,
            },
        },
        StorageEvent {
            turn_id: Some("turn-parent".to_string()),
            agent: shared.clone(),
            payload: StorageEventPayload::SubRunStarted {
                tool_call_id: Some("call-shared".to_string()),
                resolved_overrides: astrcode_core::ResolvedSubagentContextOverrides::default(),
                resolved_limits: astrcode_core::ResolvedExecutionLimitsSnapshot::default(),
                timestamp: Some(Utc::now()),
            },
        },
        StorageEvent {
            turn_id: Some("turn-parent".to_string()),
            agent: shared,
            payload: StorageEventPayload::SubRunFinished {
                tool_call_id: Some("call-shared".to_string()),
                result: astrcode_core::SubRunResult {
                    lifecycle: astrcode_core::AgentLifecycleStatus::Terminated,
                    last_turn_outcome: Some(astrcode_core::AgentTurnOutcome::Cancelled),
                    handoff: None,
                    failure: None,
                },
                step_count: 1,
                estimated_tokens: 10,
                timestamp: Some(Utc::now()),
            },
        },
        StorageEvent {
            turn_id: Some("turn-parent".to_string()),
            agent: independent.clone(),
            payload: StorageEventPayload::SubRunStarted {
                tool_call_id: Some("call-independent".to_string()),
                resolved_overrides: astrcode_core::ResolvedSubagentContextOverrides::default(),
                resolved_limits: astrcode_core::ResolvedExecutionLimitsSnapshot::default(),
                timestamp: Some(Utc::now()),
            },
        },
        StorageEvent {
            turn_id: Some("turn-parent".to_string()),
            agent: independent,
            payload: StorageEventPayload::SubRunFinished {
                tool_call_id: Some("call-independent".to_string()),
                result: astrcode_core::SubRunResult {
                    lifecycle: astrcode_core::AgentLifecycleStatus::Terminated,
                    last_turn_outcome: Some(astrcode_core::AgentTurnOutcome::Cancelled),
                    handoff: None,
                    failure: None,
                },
                step_count: 2,
                estimated_tokens: 20,
                timestamp: Some(Utc::now()),
            },
        },
    ] {
        log.append(&event).expect("event should append");
    }
}

#[async_trait]
impl Tool for DemoReadTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "readFile".to_string(),
            description: "Demo readable tool for root execution route tests.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
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
    ) -> Result<ToolExecutionResult> {
        Ok(ToolExecutionResult {
            tool_call_id,
            tool_name: "readFile".to_string(),
            ok: true,
            output: "ok".to_string(),
            error: None,
            metadata: None,
            duration_ms: 0,
            truncated: false,
        })
    }
}

/// Demo grep tool for plan agent tests.
/// Plan agent requires both readFile and grep tools.
struct DemoGrepTool;

#[async_trait]
impl Tool for DemoGrepTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "grep".to_string(),
            description: "Demo grep tool for root execution route tests.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
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
    ) -> Result<ToolExecutionResult> {
        Ok(ToolExecutionResult {
            tool_call_id,
            tool_name: "grep".to_string(),
            ok: true,
            output: "ok".to_string(),
            error: None,
            metadata: None,
            duration_ms: 0,
            truncated: false,
        })
    }
}

#[tokio::test]
async fn runtime_status_returns_not_found_when_unauthenticated() {
    let (state, _guard) = test_state(None);
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/runtime/plugins")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn runtime_status_returns_not_found_after_skeleton_removal() {
    let (state, _guard) = test_state(None);
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/runtime/plugins")
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn runtime_plugins_endpoint_returns_not_found_after_skeleton_removal() {
    let (state, _guard) = test_state(None);
    state
        .coordinator
        .plugin_registry()
        .replace_snapshot(vec![PluginEntry {
            manifest: astrcode_core::PluginManifest {
                name: "demo-plugin".to_string(),
                version: "0.1.0".to_string(),
                description: "demo".to_string(),
                plugin_type: vec![astrcode_core::PluginType::Tool],
                capabilities: Vec::new(),
                executable: Some("demo.exe".to_string()),
                args: Vec::new(),
                working_dir: None,
                repository: None,
            },
            state: PluginState::Initialized,
            health: PluginHealth::Healthy,
            failure_count: 0,
            capabilities: vec![CapabilityDescriptor {
                name: "demo.search".to_string(),
                kind: CapabilityKind::tool(),
                description: "search".to_string(),
                input_schema: json!({"type":"object"}),
                output_schema: json!({"type":"object"}),
                streaming: false,
                concurrency_safe: false,
                compact_clearable: false,
                profiles: vec!["coding".to_string()],
                tags: Vec::new(),
                permissions: Vec::new(),
                side_effect: SideEffectLevel::None,
                stability: StabilityLevel::Stable,
                metadata: json!(null),
                max_result_inline_size: None,
            }],
            failure: None,
            warnings: vec![
                "skill 'repo-search' dropped unknown allowed tool 'missing.tool'".to_string(),
            ],
            last_checked_at: None,
        }]);
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/runtime/plugins")
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    // skeleton route 已在 006-prune-dead-code 中删除
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn runtime_reload_endpoint_returns_not_found_after_skeleton_removal() {
    let (state, _guard) = test_state(None);
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/runtime/plugins/reload")
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    // skeleton route 已在 006-prune-dead-code 中删除
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn config_reload_endpoint_refreshes_runtime_config_from_disk() {
    let (state, _guard) = test_state(None);
    save_config(&Config {
        active_profile: "local-openai".to_string(),
        active_model: "gpt-4.1-mini".to_string(),
        runtime: RuntimeConfig {
            max_tool_concurrency: Some(9),
            ..RuntimeConfig::default()
        },
        profiles: vec![Profile {
            name: "local-openai".to_string(),
            provider_kind: config::PROVIDER_KIND_OPENAI.to_string(),
            base_url: "https://example.com/v1".to_string(),
            api_key: Some("literal:sk-test".to_string()),
            models: vec![ModelConfig {
                id: "gpt-4.1-mini".to_string(),
                max_tokens: Some(8096),
                context_limit: Some(128_000),
            }],
        }],
        ..Config::default()
    })
    .expect("config should save");
    let app = build_api_router().with_state(state.clone());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/config/reload")
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: ConfigReloadResponse =
        serde_json::from_slice(&bytes).expect("reload response should deserialize");
    assert_eq!(payload.config.active_profile, "local-openai");
    assert_eq!(payload.config.active_model, "gpt-4.1-mini");
    assert_eq!(
        state.service.config().get_config().await.active_model,
        "gpt-4.1-mini"
    );
    assert_eq!(
        state
            .service
            .loop_surface()
            .current_loop()
            .await
            .max_tool_concurrency(),
        9
    );
}

#[tokio::test]
async fn session_history_endpoint_returns_agent_event_snapshot_and_phase() {
    let (state, _guard) = test_state(None);
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let created = state
        .service
        .sessions()
        .create(temp_dir.path())
        .await
        .expect("session should be created");
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/sessions/{}/history", created.session_id))
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: SessionHistoryResponseDto =
        serde_json::from_slice(&bytes).expect("history response should deserialize");

    assert_eq!(payload.phase, astrcode_protocol::http::PhaseDto::Idle);
    assert_eq!(payload.events.len(), 1);
    assert_eq!(payload.cursor.as_deref(), Some("1.0"));
    assert!(matches!(
        payload.events[0].event,
        astrcode_protocol::http::AgentEventPayload::SessionStarted { .. }
    ));
}

#[tokio::test]
async fn submit_prompt_endpoint_returns_accepted_shape() {
    let (state, _guard) = test_state(None);
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let created = state
        .service
        .sessions()
        .create(temp_dir.path())
        .await
        .expect("session should be created");
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{}/prompts", created.session_id))
                .header(AUTH_HEADER_NAME, "browser-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&PromptRequest {
                        text: "hello".to_string(),
                    })
                    .expect("request should serialize"),
                ))
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let payload: PromptAcceptedResponse = serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should be readable"),
    )
    .expect("accepted payload should deserialize");
    assert_eq!(payload.session_id, created.session_id);
    assert!(!payload.turn_id.is_empty());
}

#[tokio::test]
async fn interrupt_endpoint_returns_no_content_after_prompt_submission() {
    let (state, _guard) = test_state(None);
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let created = state
        .service
        .sessions()
        .create(temp_dir.path())
        .await
        .expect("session should be created");
    let app = build_api_router().with_state(state);

    let prompt_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{}/prompts", created.session_id))
                .header(AUTH_HEADER_NAME, "browser-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&PromptRequest {
                        text: "hello".to_string(),
                    })
                    .expect("request should serialize"),
                ))
                .expect("request should be valid"),
        )
        .await
        .expect("prompt response should be returned");
    assert_eq!(prompt_response.status(), StatusCode::ACCEPTED);

    let interrupt_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{}/interrupt", created.session_id))
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("interrupt response should be returned");

    assert_eq!(interrupt_response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn session_history_endpoint_filters_subrun_scope_and_cursor() {
    let (state, _guard) = test_state(None);
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    seed_shared_subrun_session("history-filter-session", temp_dir.path());
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri(
                    "/api/sessions/history-filter-session/history?subRunId=sub-a&\
                     scope=directChildren",
                )
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: SessionHistoryResponseDto =
        serde_json::from_slice(&bytes).expect("history response should deserialize");

    let event_names = payload
        .events
        .iter()
        .filter_map(|event| match &event.event {
            astrcode_protocol::http::AgentEventPayload::SubRunStarted { agent, .. } => {
                Some(format!(
                    "subrun:{}",
                    agent.sub_run_id.as_deref().unwrap_or("unknown")
                ))
            },
            astrcode_protocol::http::AgentEventPayload::UserMessage { content, .. } => {
                Some(format!("user:{content}"))
            },
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(
        event_names,
        vec!["subrun:sub-b".to_string(), "user:sub-b".to_string(),]
    );
    assert_eq!(payload.cursor.as_deref(), Some("6.0"));
}

#[tokio::test]
async fn session_history_endpoint_serializes_mailbox_payload_with_sender_snapshots() {
    let (state, _guard) = test_state(None);
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let session_id = "mailbox-history-session";
    append_session_events(
        session_id,
        temp_dir.path(),
        [
            StorageEvent {
                turn_id: None,
                agent: AgentEventContext::default(),
                payload: StorageEventPayload::SessionStart {
                    session_id: session_id.to_string(),
                    timestamp: Utc::now(),
                    working_dir: temp_dir.path().display().to_string(),
                    parent_session_id: None,
                    parent_storage_seq: None,
                },
            },
            StorageEvent {
                turn_id: Some("turn-parent".to_string()),
                agent: AgentEventContext::default(),
                payload: StorageEventPayload::AgentMailboxQueued {
                    payload: MailboxQueuedPayload {
                        envelope: AgentMailboxEnvelope {
                            delivery_id: "delivery-history-1".to_string(),
                            from_agent_id: "agent-child".to_string(),
                            to_agent_id: "agent-parent".to_string(),
                            message: "child summary".to_string(),
                            queued_at: Utc::now(),
                            sender_lifecycle_status: AgentLifecycleStatus::Idle,
                            sender_last_turn_outcome: Some(AgentTurnOutcome::Completed),
                            sender_open_session_id: "session-child".to_string(),
                        },
                    },
                },
            },
        ],
    );
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/sessions/{}/history", session_id))
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: SessionHistoryResponseDto =
        serde_json::from_slice(&bytes).expect("history response should deserialize");

    let mailbox_event = payload
        .events
        .iter()
        .find_map(|event| match &event.event {
            astrcode_protocol::http::AgentEventPayload::AgentMailboxQueued { payload, .. } => {
                Some(payload)
            },
            _ => None,
        })
        .expect("history should include mailbox queued payload");

    assert_eq!(mailbox_event.delivery_id, "delivery-history-1");
    assert_eq!(mailbox_event.from_agent_id, "agent-child");
    assert_eq!(mailbox_event.to_agent_id, "agent-parent");
    assert_eq!(mailbox_event.sender_lifecycle_status, "Idle");
    assert_eq!(
        mailbox_event.sender_last_turn_outcome.as_deref(),
        Some("Completed")
    );
    assert_eq!(mailbox_event.sender_open_session_id, "session-child");
}

#[tokio::test]
async fn session_history_endpoint_rejects_legacy_subtree_scope() {
    let (state, _guard) = test_state(None);
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    seed_legacy_subrun_session("history-legacy-filter-session", temp_dir.path());
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri(
                    "/api/sessions/history-legacy-filter-session/history?subRunId=sub-legacy&\
                     scope=subtree",
                )
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = serde_json::from_slice::<serde_json::Value>(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should be readable"),
    )
    .expect("error payload should deserialize");
    assert_eq!(
        body.get("error").and_then(|value| value.as_str()),
        Some("lineage metadata unavailable for requested scope")
    );
}

#[tokio::test]
async fn session_history_endpoint_returns_404_for_unknown_session() {
    let (state, _guard) = test_state(None);
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/sessions/nonexistent-session-id/history")
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn agents_list_endpoint_returns_current_profiles() {
    let (state, _guard) = test_state(None);
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/agents")
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: Vec<AgentProfileDto> =
        serde_json::from_slice(&bytes).expect("agent list should deserialize");
    assert!(payload.iter().any(|profile| profile.id == "plan"));
}

#[tokio::test]
async fn tools_list_endpoint_returns_not_found_after_skeleton_removal() {
    let (state, _guard) = test_state(None);
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/tools")
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    // skeleton route 已在 006-prune-dead-code 中删除
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn direct_agent_execute_endpoint_accepts_root_execution() {
    // Plan agent requires both readFile and grep tools.
    // Register both to satisfy the agent's tool requirements.
    let invokers: Vec<std::sync::Arc<dyn astrcode_core::CapabilityInvoker>> = [
        Box::new(DemoReadTool) as Box<dyn astrcode_core::Tool>,
        Box::new(DemoGrepTool) as Box<dyn astrcode_core::Tool>,
    ]
    .into_iter()
    .map(|t| ToolCapabilityInvoker::boxed(t).expect("demo tool should wrap"))
    .collect();
    let mut builder = CapabilityRouter::builder();
    for invoker in invokers {
        builder = builder.register_invoker(invoker);
    }
    let capabilities = builder
        .build()
        .expect("tool-derived capability router should build");
    let (state, _guard) = test_state_with_capabilities(capabilities, None);
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/agents/plan/execute")
                .header(AUTH_HEADER_NAME, "browser-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "task": "summarize the workspace",
                        "workingDir": temp_dir.path().display().to_string()
                    })
                    .to_string(),
                ))
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: AgentExecuteResponseDto =
        serde_json::from_slice(&bytes).expect("response should deserialize");
    assert!(payload.accepted);
    assert!(payload.session_id.is_some());
    assert!(payload.turn_id.is_some());
    assert!(payload.agent_id.is_some());
}

#[tokio::test]
async fn direct_agent_execute_endpoint_requires_working_dir() {
    let invokers: Vec<std::sync::Arc<dyn astrcode_core::CapabilityInvoker>> =
        [Box::new(DemoReadTool) as Box<dyn astrcode_core::Tool>]
            .into_iter()
            .map(|t| ToolCapabilityInvoker::boxed(t).expect("demo tool should wrap"))
            .collect();
    let mut builder = CapabilityRouter::builder();
    for invoker in invokers {
        builder = builder.register_invoker(invoker);
    }
    let capabilities = builder
        .build()
        .expect("tool-derived capability router should build");
    let (state, _guard) = test_state_with_capabilities(capabilities, None);
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/agents/plan/execute")
                .header(AUTH_HEADER_NAME, "browser-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "task": "summarize the workspace"
                    })
                    .to_string(),
                ))
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn direct_agent_execute_endpoint_resolves_project_agent_from_request_working_dir() {
    let invokers: Vec<std::sync::Arc<dyn astrcode_core::CapabilityInvoker>> =
        [Box::new(DemoReadTool) as Box<dyn astrcode_core::Tool>]
            .into_iter()
            .map(|t| ToolCapabilityInvoker::boxed(t).expect("demo tool should wrap"))
            .collect();
    let mut builder = CapabilityRouter::builder();
    for invoker in invokers {
        builder = builder.register_invoker(invoker);
    }
    let capabilities = builder
        .build()
        .expect("tool-derived capability router should build");
    let (state, _guard) = test_state_with_capabilities(capabilities, None);
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let agent_dir = temp_dir.path().join(".astrcode").join("agents");
    std::fs::create_dir_all(&agent_dir).expect("agent dir should be created");
    std::fs::write(
        agent_dir.join("scoped-review.md"),
        r#"---
name: scoped-review
description: Working-dir scoped reviewer
tools: ["readFile"]
---
Only available inside this workspace.
"#,
    )
    .expect("scoped agent should be written");
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/agents/scoped-review/execute")
                .header(AUTH_HEADER_NAME, "browser-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "task": "review this repository",
                        "workingDir": temp_dir.path().display().to_string()
                    })
                    .to_string(),
                ))
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::ACCEPTED);
}

#[tokio::test]
async fn subrun_status_endpoint_returns_not_found_for_unknown_subrun() {
    let (state, _guard) = test_state(None);
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let created = state
        .service
        .sessions()
        .create(temp_dir.path())
        .await
        .expect("session should be created");
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v1/sessions/{}/subruns/missing-subrun",
                    created.session_id
                ))
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let _payload: serde_json::Value =
        serde_json::from_slice(&bytes).expect("error payload should deserialize");
}

#[tokio::test]
async fn subrun_status_endpoint_returns_contract_fields_for_durable_snapshot() {
    let (state, _guard) = test_state(None);
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    seed_finished_subrun_session("subrun-status-session", temp_dir.path());
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/sessions/subrun-status-session/subruns/sub-durable")
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: SubRunStatusDto =
        serde_json::from_slice(&bytes).expect("status payload should deserialize");

    assert_eq!(payload.sub_run_id, "sub-durable");
    assert_eq!(payload.source, SubRunStatusSourceDto::Durable);
    assert_eq!(
        payload.lifecycle,
        astrcode_protocol::http::AgentLifecycleDto::Terminated
    );
    assert_eq!(payload.step_count, Some(2));
    assert_eq!(payload.estimated_tokens, Some(120));
    assert_eq!(payload.tool_call_id.as_deref(), Some("call-durable"));
}

#[tokio::test]
async fn subrun_status_endpoint_keeps_storage_mode_parity_for_parent_aborted_subruns() {
    let (state, _guard) = test_state(None);
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    seed_parent_abort_storage_mode_parity_session("subrun-abort-parity", temp_dir.path());
    let app = build_api_router().with_state(state);

    for (sub_run_id, expected_mode) in
        [("sub-independent", SubRunStorageModeDto::IndependentSession)]
    {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/v1/sessions/subrun-abort-parity/subruns/{sub_run_id}"
                    ))
                    .header(AUTH_HEADER_NAME, "browser-token")
                    .body(Body::empty())
                    .expect("request should be valid"),
            )
            .await
            .expect("response should be returned");

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should be readable");
        let payload: SubRunStatusDto =
            serde_json::from_slice(&bytes).expect("status payload should deserialize");

        assert_eq!(payload.source, SubRunStatusSourceDto::Durable);
        assert_eq!(
            payload.lifecycle,
            astrcode_protocol::http::AgentLifecycleDto::Terminated
        );
        assert_eq!(payload.storage_mode, expected_mode);
    }
}

#[tokio::test]
async fn subrun_status_endpoint_reports_live_for_independent_subrun_owned_by_parent_session() {
    let (state, _guard) = test_state(None);
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let parent_session = state
        .service
        .sessions()
        .create(temp_dir.path())
        .await
        .expect("parent session should be created");
    let child_session = state
        .service
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
    let control = state.service.execution().control();
    let handle = control
        .spawn_with_storage(
            &profile,
            child_session.session_id.clone(),
            Some(child_session.session_id.clone()),
            "turn-parent".to_string(),
            None,
            astrcode_core::SubRunStorageMode::IndependentSession,
        )
        .await
        .expect("independent sub-run should spawn");
    control
        .set_lifecycle(
            &handle.agent_id,
            astrcode_core::AgentLifecycleStatus::Running,
        )
        .await
        .expect("sub-run should be running");

    let mut log = EventLog::open(&parent_session.session_id).expect("parent event log should open");
    log.append(&StorageEvent {
        turn_id: Some("turn-parent".to_string()),
        agent: AgentEventContext::sub_run(
            handle.agent_id.clone(),
            "turn-parent",
            "review",
            handle.sub_run_id.clone(),
            None,
            astrcode_core::SubRunStorageMode::IndependentSession,
            Some(child_session.session_id.clone()),
        ),
        payload: StorageEventPayload::SubRunStarted {
            tool_call_id: Some("call-independent".to_string()),
            resolved_overrides: astrcode_core::ResolvedSubagentContextOverrides {
                storage_mode: astrcode_core::SubRunStorageMode::IndependentSession,
                ..Default::default()
            },
            resolved_limits: astrcode_core::ResolvedExecutionLimitsSnapshot::default(),
            timestamp: Some(Utc::now()),
        },
    })
    .expect("sub-run started should append");

    let app = build_api_router().with_state(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v1/sessions/{}/subruns/{}",
                    parent_session.session_id, handle.sub_run_id
                ))
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: SubRunStatusDto =
        serde_json::from_slice(&bytes).expect("status payload should deserialize");
    assert_eq!(payload.source, SubRunStatusSourceDto::Live);
    assert_eq!(
        payload.lifecycle,
        astrcode_protocol::http::AgentLifecycleDto::Running
    );
    assert_eq!(
        payload.storage_mode,
        SubRunStorageModeDto::IndependentSession
    );
    assert_eq!(payload.child_session_id, Some(child_session.session_id));
    assert_eq!(payload.tool_call_id.as_deref(), Some("call-independent"));
}

#[tokio::test]
async fn direct_tool_execute_endpoint_returns_not_found_after_skeleton_removal() {
    let (state, _guard) = test_state(None);
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/tools/readFile/execute")
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::from("{}"))
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    // skeleton route 已在 006-prune-dead-code 中删除
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ============================================================================
// Scope Filter Contract Tests (T029)
// ============================================================================

#[tokio::test]
async fn history_scope_self_returns_only_target_subrun_events() {
    let (state, guard) = test_state(None);
    let app = build_api_router().with_state(state);

    seed_shared_subrun_session("session-scope-self", guard.path());

    // Why: scope=self 只返回 sub-b 自己的事件，不包含其子执行 sub-c
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/sessions/session-scope-self/history?subRunId=sub-b&scope=self")
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let history: SessionHistoryResponseDto = serde_json::from_slice(&body).unwrap();

    // 应该只包含 sub-b 的 SubRunStarted 和 UserMessage，不包含 sub-c
    let sub_b_events: Vec<_> = history
        .events
        .iter()
        .filter(|e| match &e.event {
            astrcode_protocol::http::AgentEventPayload::SubRunStarted { agent, .. }
            | astrcode_protocol::http::AgentEventPayload::UserMessage { agent, .. } => {
                agent.sub_run_id.as_deref() == Some("sub-b")
            },
            _ => false,
        })
        .collect();

    assert!(
        sub_b_events.len() >= 2,
        "should have at least SubRunStarted and UserMessage for sub-b"
    );

    let has_sub_c = history.events.iter().any(|e| match &e.event {
        astrcode_protocol::http::AgentEventPayload::SubRunStarted { agent, .. }
        | astrcode_protocol::http::AgentEventPayload::UserMessage { agent, .. } => {
            agent.sub_run_id.as_deref() == Some("sub-c")
        },
        _ => false,
    });
    assert!(!has_sub_c, "scope=self should not include child sub-c");
}

#[tokio::test]
async fn history_scope_direct_children_returns_only_immediate_children() {
    let (state, guard) = test_state(None);
    let app = build_api_router().with_state(state);

    seed_shared_subrun_session("session-scope-direct", guard.path());

    // Why: scope=directChildren 应该只返回 sub-a 的直接子执行 sub-b，不包含孙级 sub-c
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(
                    "/api/sessions/session-scope-direct/history?subRunId=sub-a&\
                     scope=directChildren",
                )
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let history: SessionHistoryResponseDto = serde_json::from_slice(&body).unwrap();

    // 应该包含 sub-b 的事件
    let has_sub_b = history.events.iter().any(|e| match &e.event {
        astrcode_protocol::http::AgentEventPayload::SubRunStarted { agent, .. }
        | astrcode_protocol::http::AgentEventPayload::UserMessage { agent, .. } => {
            agent.sub_run_id.as_deref() == Some("sub-b")
        },
        _ => false,
    });
    assert!(has_sub_b, "directChildren should include sub-b");

    // 不应该包含 sub-a 自己的事件
    let has_sub_a = history.events.iter().any(|e| match &e.event {
        astrcode_protocol::http::AgentEventPayload::SubRunStarted { agent, .. }
        | astrcode_protocol::http::AgentEventPayload::UserMessage { agent, .. } => {
            agent.sub_run_id.as_deref() == Some("sub-a")
        },
        _ => false,
    });
    assert!(
        !has_sub_a,
        "directChildren should not include target itself"
    );

    // 不应该包含孙级 sub-c
    let has_sub_c = history.events.iter().any(|e| match &e.event {
        astrcode_protocol::http::AgentEventPayload::SubRunStarted { agent, .. }
        | astrcode_protocol::http::AgentEventPayload::UserMessage { agent, .. } => {
            agent.sub_run_id.as_deref() == Some("sub-c")
        },
        _ => false,
    });
    assert!(
        !has_sub_c,
        "directChildren should not include grandchild sub-c"
    );
}

#[tokio::test]
async fn history_scope_subtree_returns_target_and_all_descendants() {
    let (state, guard) = test_state(None);
    let app = build_api_router().with_state(state);

    seed_shared_subrun_session("session-scope-subtree", guard.path());

    // Why: scope=subtree 应该返回 sub-a 自己 + 所有递归后代（sub-b, sub-c）
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/sessions/session-scope-subtree/history?subRunId=sub-a&scope=subtree")
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let history: SessionHistoryResponseDto = serde_json::from_slice(&body).unwrap();

    // 应该包含 sub-a, sub-b, sub-c
    let has_sub_a = history.events.iter().any(|e| match &e.event {
        astrcode_protocol::http::AgentEventPayload::SubRunStarted { agent, .. }
        | astrcode_protocol::http::AgentEventPayload::UserMessage { agent, .. } => {
            agent.sub_run_id.as_deref() == Some("sub-a")
        },
        _ => false,
    });
    let has_sub_b = history.events.iter().any(|e| match &e.event {
        astrcode_protocol::http::AgentEventPayload::SubRunStarted { agent, .. }
        | astrcode_protocol::http::AgentEventPayload::UserMessage { agent, .. } => {
            agent.sub_run_id.as_deref() == Some("sub-b")
        },
        _ => false,
    });
    let has_sub_c = history.events.iter().any(|e| match &e.event {
        astrcode_protocol::http::AgentEventPayload::SubRunStarted { agent, .. }
        | astrcode_protocol::http::AgentEventPayload::UserMessage { agent, .. } => {
            agent.sub_run_id.as_deref() == Some("sub-c")
        },
        _ => false,
    });

    assert!(has_sub_a, "subtree should include target sub-a");
    assert!(has_sub_b, "subtree should include child sub-b");
    assert!(has_sub_c, "subtree should include grandchild sub-c");
}

#[tokio::test]
async fn history_scope_direct_children_rejects_legacy_without_descriptor() {
    let temp = tempfile::tempdir().unwrap();
    let (state, _guard) = test_state(None);
    let app = build_api_router().with_state(state);

    seed_legacy_subrun_session("session-legacy-scope", temp.path());

    // Why: legacy 历史缺少 descriptor，directChildren 必须拒绝而不是猜测
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(
                    "/api/sessions/session-legacy-scope/history?subRunId=sub-legacy&\
                     scope=directChildren",
                )
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // 应该返回 409 或其他错误状态码，表示 lineage 不可用
    assert!(
        response.status() == StatusCode::CONFLICT || response.status().is_client_error(),
        "directChildren on legacy history should fail, got: {}",
        response.status()
    );
}

#[tokio::test]
async fn history_scope_subtree_rejects_legacy_without_descriptor() {
    let temp = tempfile::tempdir().unwrap();
    let (state, _guard) = test_state(None);
    let app = build_api_router().with_state(state);

    seed_legacy_subrun_session("session-legacy-subtree", temp.path());

    // Why: legacy 历史缺少 descriptor，subtree 必须拒绝而不是做 partial tree
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(
                    "/api/sessions/session-legacy-subtree/history?subRunId=sub-legacy&\
                     scope=subtree",
                )
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(
        response.status() == StatusCode::CONFLICT || response.status().is_client_error(),
        "subtree on legacy history should fail, got: {}",
        response.status()
    );
}

#[tokio::test]
async fn history_scope_self_allows_legacy_without_descriptor() {
    let (state, guard) = test_state(None);
    let app = build_api_router().with_state(state);

    seed_legacy_subrun_session("session-legacy-self", guard.path());

    // Why: scope=self 不依赖 ancestry，legacy 历史仍然允许
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/sessions/session-legacy-self/history?subRunId=sub-legacy&scope=self")
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "scope=self should work on legacy history"
    );
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let history: SessionHistoryResponseDto = serde_json::from_slice(&body).unwrap();

    let has_legacy = history.events.iter().any(|e| match &e.event {
        astrcode_protocol::http::AgentEventPayload::SubRunStarted { agent, .. }
        | astrcode_protocol::http::AgentEventPayload::UserMessage { agent, .. } => {
            agent.sub_run_id.as_deref() == Some("sub-legacy")
        },
        _ => false,
    });
    assert!(has_legacy, "should return legacy subrun events");
}
