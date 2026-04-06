use astrcode_core::{
    AgentEventContext, CapabilityDescriptor, CapabilityKind, EventLogWriter, PluginHealth,
    PluginState, Result, SideEffectLevel, StabilityLevel, StorageEvent, Tool,
    ToolCapabilityMetadata, ToolContext, ToolDefinition, ToolExecutionResult, UserMessageOrigin,
    plugin::PluginEntry,
};
use astrcode_protocol::http::{
    AgentExecuteResponseDto, AgentProfileDto, ConfigReloadResponse, RuntimeStatusDto,
    SessionHistoryResponseDto, ToolDescriptorDto, ToolExecuteResponseDto,
};
use astrcode_runtime::{Config, ModelConfig, Profile, RuntimeConfig, config, save_config};
use astrcode_runtime_registry::{CapabilityRouter, ToolRegistry};
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
    test_support::{test_state, test_state_with_capabilities},
};

struct DemoReadTool;

fn seed_shared_subrun_session(session_id: &str, working_dir: &std::path::Path) {
    let mut log =
        EventLog::create(session_id, working_dir).expect("session file should be created");
    let root = AgentEventContext::root_execution("root-agent", "primary");
    let sub_a = AgentEventContext::sub_run(
        "agent-a",
        "turn-root",
        "review",
        "sub-a",
        astrcode_core::SubRunStorageMode::SharedSession,
        None,
    );
    let sub_b = AgentEventContext::sub_run(
        "agent-b",
        "turn-a",
        "review",
        "sub-b",
        astrcode_core::SubRunStorageMode::SharedSession,
        None,
    );
    let sub_c = AgentEventContext::sub_run(
        "agent-c",
        "turn-b",
        "review",
        "sub-c",
        astrcode_core::SubRunStorageMode::SharedSession,
        None,
    );

    for event in [
        StorageEvent::SessionStart {
            session_id: session_id.to_string(),
            timestamp: Utc::now(),
            working_dir: working_dir.display().to_string(),
            parent_session_id: None,
            parent_storage_seq: None,
        },
        StorageEvent::UserMessage {
            turn_id: Some("turn-root".to_string()),
            agent: root,
            content: "root".to_string(),
            origin: UserMessageOrigin::User,
            timestamp: Utc::now(),
        },
        StorageEvent::SubRunStarted {
            turn_id: Some("turn-root".to_string()),
            agent: sub_a.clone(),
            resolved_overrides: astrcode_core::ResolvedSubagentContextOverrides::default(),
            resolved_limits: astrcode_core::ResolvedExecutionLimitsSnapshot::default(),
            timestamp: Some(Utc::now()),
        },
        StorageEvent::UserMessage {
            turn_id: Some("turn-a".to_string()),
            agent: sub_a.clone(),
            content: "sub-a".to_string(),
            origin: UserMessageOrigin::User,
            timestamp: Utc::now(),
        },
        StorageEvent::SubRunStarted {
            turn_id: Some("turn-a".to_string()),
            agent: sub_b.clone(),
            resolved_overrides: astrcode_core::ResolvedSubagentContextOverrides::default(),
            resolved_limits: astrcode_core::ResolvedExecutionLimitsSnapshot::default(),
            timestamp: Some(Utc::now()),
        },
        StorageEvent::UserMessage {
            turn_id: Some("turn-b".to_string()),
            agent: sub_b.clone(),
            content: "sub-b".to_string(),
            origin: UserMessageOrigin::User,
            timestamp: Utc::now(),
        },
        StorageEvent::SubRunStarted {
            turn_id: Some("turn-b".to_string()),
            agent: sub_c,
            resolved_overrides: astrcode_core::ResolvedSubagentContextOverrides::default(),
            resolved_limits: astrcode_core::ResolvedExecutionLimitsSnapshot::default(),
            timestamp: Some(Utc::now()),
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
async fn runtime_status_requires_authentication() {
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

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn runtime_status_exposes_capability_surface() {
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

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn runtime_status_exposes_plugin_warnings() {
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

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: RuntimeStatusDto =
        serde_json::from_slice(&bytes).expect("runtime status should deserialize");
    assert_eq!(payload.plugins.len(), 1);
    assert!(
        payload.plugins[0]
            .warnings
            .iter()
            .any(|warning| warning.contains("missing.tool"))
    );
}

#[tokio::test]
async fn runtime_reload_endpoint_returns_accepted() {
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

    assert_eq!(response.status(), StatusCode::ACCEPTED);
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
        state.service.get_config().await.active_model,
        "gpt-4.1-mini"
    );
    assert_eq!(state.service.current_loop().await.max_tool_concurrency(), 9);
}

#[tokio::test]
async fn session_history_endpoint_returns_agent_event_snapshot_and_phase() {
    let (state, _guard) = test_state(None);
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let created = state
        .service
        .create_session(temp_dir.path())
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
        vec![
            "subrun:sub-a".to_string(),
            "user:sub-a".to_string(),
            "subrun:sub-b".to_string(),
            "user:sub-b".to_string(),
        ]
    );
    assert_eq!(payload.cursor.as_deref(), Some("6.0"));
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
async fn tools_list_endpoint_returns_runtime_tool_surface() {
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

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let _payload: Vec<ToolDescriptorDto> =
        serde_json::from_slice(&bytes).expect("tool list should deserialize");
}

#[tokio::test]
async fn direct_agent_execute_endpoint_accepts_root_execution() {
    // Plan agent requires both readFile and grep tools.
    // Register both to satisfy the agent's tool requirements.
    let invokers = ToolRegistry::builder()
        .register(Box::new(DemoReadTool))
        .register(Box::new(DemoGrepTool))
        .build()
        .into_capability_invokers()
        .expect("demo tool descriptors should build");
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
async fn subrun_status_endpoint_returns_not_found_for_unknown_subrun() {
    let (state, _guard) = test_state(None);
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let created = state
        .service
        .create_session(temp_dir.path())
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
async fn direct_tool_execute_endpoint_returns_not_implemented() {
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

    assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: ToolExecuteResponseDto =
        serde_json::from_slice(&bytes).expect("response should deserialize");
    assert!(!payload.accepted);
}
