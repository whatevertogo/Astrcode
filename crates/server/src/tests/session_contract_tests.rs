use std::{net::TcpListener, sync::Arc, time::Duration};

use astrcode_core::{
    AgentEventContext, EventLogWriter, PluginRegistry, RuntimeCoordinator, RuntimeHandle,
    StorageEvent,
};
use astrcode_protocol::http::{
    AgentEventPayload, ChildSessionLineageKindDto, ChildSessionNotificationKindDto,
    ParentChildSummaryListResponseDto, PromptAcceptedResponse, PromptRequest,
    SessionHistoryResponseDto, SubRunStatusDto, SubRunStatusSourceDto,
};
use astrcode_runtime::{
    Config, ModelConfig, Profile, RuntimeConfig, RuntimeGovernance, RuntimeService,
    config::PROVIDER_KIND_OPENAI, save_config,
};
use astrcode_runtime_registry::CapabilityRouter;
use astrcode_storage::session::EventLog;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use chrono::Utc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    task::JoinHandle,
};
use tower::ServiceExt;

use crate::{
    AUTH_HEADER_NAME, AppState,
    auth::{AuthSessionManager, BootstrapAuth},
    routes::build_api_router,
    test_support::{ServerTestEnvGuard, test_state},
};

// Why: 这些契约测试是 quickstart 验证矩阵中 scope 参数合法性与显式错误语义的
// 稳定保障，防止 server 在重构后回退到隐式容错或启发式行为。

fn seed_subrun_status_contract_session(session_id: &str, working_dir: &std::path::Path) {
    let mut log =
        EventLog::create(session_id, working_dir).expect("session file should be created");
    let descriptor = astrcode_core::SubRunDescriptor {
        sub_run_id: "subrun-contract".to_string(),
        parent_turn_id: "turn-contract".to_string(),
        parent_agent_id: Some("agent-parent".to_string()),
        depth: 1,
    };
    let sub = AgentEventContext::sub_run(
        "agent-contract",
        "turn-contract",
        "review",
        "subrun-contract",
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
        StorageEvent::SubRunStarted {
            turn_id: Some("turn-contract".to_string()),
            agent: sub.clone(),
            descriptor: Some(descriptor.clone()),
            tool_call_id: Some("call-contract".to_string()),
            resolved_overrides: astrcode_core::ResolvedSubagentContextOverrides::default(),
            resolved_limits: astrcode_core::ResolvedExecutionLimitsSnapshot::default(),
            timestamp: Some(Utc::now()),
        },
        StorageEvent::SubRunFinished {
            turn_id: Some("turn-contract".to_string()),
            agent: sub,
            descriptor: Some(descriptor),
            tool_call_id: Some("call-contract".to_string()),
            result: astrcode_core::SubRunResult {
                status: astrcode_core::SubRunOutcome::Completed,
                handoff: None,
                failure: None,
            },
            step_count: 1,
            estimated_tokens: 42,
            timestamp: Some(Utc::now()),
        },
    ] {
        log.append(&event).expect("event should append");
    }
}

fn seed_legacy_subrun_status_contract_session(session_id: &str, working_dir: &std::path::Path) {
    let mut log =
        EventLog::create(session_id, working_dir).expect("session file should be created");
    let sub = AgentEventContext::sub_run(
        "agent-legacy-contract",
        "turn-legacy-contract",
        "review",
        "subrun-legacy-contract",
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
        StorageEvent::SubRunStarted {
            turn_id: Some("turn-legacy-contract".to_string()),
            agent: sub.clone(),
            descriptor: None,
            tool_call_id: Some("call-legacy-contract".to_string()),
            resolved_overrides: astrcode_core::ResolvedSubagentContextOverrides::default(),
            resolved_limits: astrcode_core::ResolvedExecutionLimitsSnapshot::default(),
            timestamp: Some(Utc::now()),
        },
        StorageEvent::SubRunFinished {
            turn_id: Some("turn-legacy-contract".to_string()),
            agent: sub,
            descriptor: None,
            tool_call_id: Some("call-legacy-contract".to_string()),
            result: astrcode_core::SubRunResult {
                status: astrcode_core::SubRunOutcome::Completed,
                handoff: None,
                failure: None,
            },
            step_count: 1,
            estimated_tokens: 21,
            timestamp: Some(Utc::now()),
        },
    ] {
        log.append(&event).expect("event should append");
    }
}

fn configured_state_with_openai_server(base_url: &str) -> (AppState, ServerTestEnvGuard) {
    let guard = ServerTestEnvGuard::new();
    save_config(&Config {
        active_profile: "local-openai".to_string(),
        active_model: "model-a".to_string(),
        runtime: RuntimeConfig::default(),
        profiles: vec![Profile {
            name: "local-openai".to_string(),
            provider_kind: PROVIDER_KIND_OPENAI.to_string(),
            base_url: base_url.to_string(),
            api_key: Some("sk-test".to_string()),
            models: vec![ModelConfig {
                id: "model-a".to_string(),
                max_tokens: Some(8096),
                context_limit: Some(128_000),
            }],
        }],
        ..Config::default()
    })
    .expect("test config should save");

    let capabilities = CapabilityRouter::builder()
        .build()
        .expect("empty capability router should build");
    let service = Arc::new(
        RuntimeService::from_capabilities(capabilities).expect("runtime service should initialize"),
    );
    let runtime: Arc<dyn RuntimeHandle> = service.clone();
    let coordinator = Arc::new(RuntimeCoordinator::new(
        runtime,
        Arc::new(PluginRegistry::default()),
        Vec::new(),
    ));
    let runtime_governance = Arc::new(RuntimeGovernance::from_runtime(
        Arc::clone(&service),
        Arc::clone(&coordinator),
    ));
    let auth_sessions = Arc::new(AuthSessionManager::default());
    auth_sessions.issue_test_token("browser-token");

    (
        AppState {
            service,
            coordinator,
            runtime_governance,
            auth_sessions,
            bootstrap_auth: BootstrapAuth::new(
                "browser-token".to_string(),
                chrono::Utc::now().timestamp_millis() + 60_000,
            ),
            frontend_build: None,
        },
        guard,
    )
}

fn spawn_openai_chat_server(
    content: &str,
    delay: Duration,
    max_requests: usize,
) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
    let addr = listener.local_addr().expect("listener should have addr");
    listener
        .set_nonblocking(true)
        .expect("listener should be nonblocking");
    let listener = tokio::net::TcpListener::from_std(listener).expect("tokio listener");
    let content = content.to_string();

    let handle = tokio::spawn(async move {
        for _ in 0..max_requests {
            let (mut socket, _) = listener.accept().await.expect("accept should work");
            let mut buf = [0_u8; 16_384];
            let bytes_read = socket.read(&mut buf).await.expect("request should read");
            let request = String::from_utf8_lossy(&buf[..bytes_read]);
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            let response_body = if request.contains("\"stream\":true") {
                format!(
                    "data: {}\n\ndata: [DONE]\n\n",
                    serde_json::json!({
                        "choices": [{
                            "delta": { "content": content },
                            "finish_reason": "stop",
                        }]
                    })
                )
            } else {
                serde_json::json!({
                    "choices": [{
                        "message": {
                            "content": content,
                        }
                    }]
                })
                .to_string()
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: {}\r\ncontent-length: {}\r\nconnection: \
                 close\r\n\r\n{}",
                if request.contains("\"stream\":true") {
                    "text/event-stream"
                } else {
                    "application/json"
                },
                response_body.len(),
                response_body
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("response should be written");
            let _ = socket.shutdown().await;
        }
    });

    (format!("http://{}", addr), handle)
}

#[tokio::test]
async fn submit_prompt_contract_returns_accepted_shape() {
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
    .expect("payload should deserialize");
    assert_eq!(payload.session_id, created.session_id);
    assert!(!payload.turn_id.is_empty());
}

#[tokio::test]
async fn compact_session_contract_returns_conflict_for_busy_session() {
    let (base_url, server_handle) =
        spawn_openai_chat_server("slow response", Duration::from_millis(300), 1);
    let (state, _guard) = configured_state_with_openai_server(&base_url);
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
        .expect("response should be returned");
    assert_eq!(prompt_response.status(), StatusCode::ACCEPTED);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{}/compact", created.session_id))
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::CONFLICT);
    server_handle.await.expect("server should finish");
}

#[tokio::test]
async fn interrupt_contract_returns_no_content_for_running_session() {
    let (base_url, server_handle) =
        spawn_openai_chat_server("slow response", Duration::from_millis(300), 1);
    let (state, _guard) = configured_state_with_openai_server(&base_url);
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
        .expect("response should be returned");
    assert_eq!(prompt_response.status(), StatusCode::ACCEPTED);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{}/interrupt", created.session_id))
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let mut server_handle = server_handle;
    if tokio::time::timeout(Duration::from_secs(1), &mut server_handle)
        .await
        .is_err()
    {
        // 中断路径允许后台 turn 在真正发起 LLM 请求前就被取消，因此 mock server
        // 可能永远等不到那次连接。这里主动终止 side server，避免把时序偶然性
        // 变成契约测试的必备前提。
        server_handle.abort();
    } else {
        server_handle
            .await
            .expect("server should finish once the pending request is drained");
    }
}

#[tokio::test]
async fn subrun_status_contract_returns_not_found_for_missing_subrun() {
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
}

#[tokio::test]
async fn subrun_status_contract_returns_expected_payload_shape() {
    let (state, _guard) = test_state(None);
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    seed_subrun_status_contract_session("subrun-contract-session", temp_dir.path());
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/sessions/subrun-contract-session/subruns/subrun-contract")
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::OK);
    let payload: SubRunStatusDto = serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should be readable"),
    )
    .expect("payload should deserialize");
    assert_eq!(payload.sub_run_id, "subrun-contract");
    assert_eq!(payload.source, SubRunStatusSourceDto::Durable);
    assert_eq!(payload.status, "completed");
    assert_eq!(payload.step_count, Some(1));
    assert_eq!(payload.estimated_tokens, Some(42));
    assert!(payload.descriptor.is_some());
    assert_eq!(payload.tool_call_id.as_deref(), Some("call-contract"));
}

#[tokio::test]
async fn subrun_status_contract_downgrades_legacy_durable_payload_shape() {
    let (state, _guard) = test_state(None);
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    seed_legacy_subrun_status_contract_session("subrun-legacy-contract-session", temp_dir.path());
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri(
                    "/api/v1/sessions/subrun-legacy-contract-session/subruns/\
                     subrun-legacy-contract",
                )
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::OK);
    let payload: SubRunStatusDto = serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should be readable"),
    )
    .expect("payload should deserialize");

    assert_eq!(payload.source, SubRunStatusSourceDto::LegacyDurable);
    assert_eq!(payload.status, "completed");
    assert_eq!(payload.depth, 0);
    assert!(payload.descriptor.is_none());
    assert!(payload.tool_call_id.is_none());
    assert!(payload.parent_agent_id.is_none());
}

#[tokio::test]
async fn subrun_cancel_contract_returns_not_found_for_missing_subrun() {
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
                .uri(format!(
                    "/api/v1/sessions/{}/subruns/missing-subrun/cancel",
                    created.session_id
                ))
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn session_routes_reject_invalid_session_id_format() {
    let (state, _guard) = test_state(None);
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/sessions/bad.id/history")
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn session_history_contract_rejects_scope_without_subrun_id() {
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
                    "/api/sessions/{}/history?scope=self",
                    created.session_id
                ))
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn session_events_contract_rejects_scope_without_subrun_id() {
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
                    "/api/sessions/{}/events?scope=self",
                    created.session_id
                ))
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn session_history_contract_rejects_legacy_subtree_scope_with_conflict() {
    let (state, _guard) = test_state(None);
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    seed_legacy_subrun_status_contract_session("legacy-history-contract-session", temp_dir.path());
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri(
                    "/api/sessions/legacy-history-contract-session/history?\
                     subRunId=subrun-legacy-contract&scope=subtree",
                )
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn session_events_contract_rejects_legacy_direct_children_scope_with_conflict() {
    let (state, _guard) = test_state(None);
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    seed_legacy_subrun_status_contract_session("legacy-events-contract-session", temp_dir.path());
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri(
                    "/api/sessions/legacy-events-contract-session/events?\
                     subRunId=subrun-legacy-contract&scope=directChildren",
                )
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn subrun_status_contract_rejects_invalid_session_id_format() {
    let (state, _guard) = test_state(None);
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/sessions/bad.id/subruns/missing-subrun")
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn subrun_cancel_contract_rejects_invalid_subrun_id_format() {
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
                .uri(format!(
                    "/api/v1/sessions/{}/subruns/bad.id/cancel",
                    created.session_id
                ))
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// ============================================================================
// Scope Filter Contract Tests (T029 - Additional Coverage)
// ============================================================================

fn seed_nested_subrun_hierarchy(session_id: &str, working_dir: &std::path::Path) {
    // Why: 创建一个三层嵌套的子执行层级用于测试 scope 过滤语义
    // root -> sub-level1 -> sub-level2 -> sub-level3
    let mut log =
        EventLog::create(session_id, working_dir).expect("session file should be created");

    let root = AgentEventContext::root_execution("agent-root", "primary");
    let sub1 = AgentEventContext::sub_run(
        "agent-level1",
        "turn-root",
        "review",
        "sub-level1",
        astrcode_core::SubRunStorageMode::SharedSession,
        None,
    );
    let sub2 = AgentEventContext::sub_run(
        "agent-level2",
        "turn-level1",
        "review",
        "sub-level2",
        astrcode_core::SubRunStorageMode::SharedSession,
        None,
    );
    let sub3 = AgentEventContext::sub_run(
        "agent-level3",
        "turn-level2",
        "review",
        "sub-level3",
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
            content: "root message".to_string(),
            origin: astrcode_core::UserMessageOrigin::User,
            timestamp: Utc::now(),
        },
        StorageEvent::SubRunStarted {
            turn_id: Some("turn-root".to_string()),
            agent: sub1.clone(),
            descriptor: Some(astrcode_core::SubRunDescriptor {
                sub_run_id: "sub-level1".to_string(),
                parent_turn_id: "turn-root".to_string(),
                parent_agent_id: Some("agent-root".to_string()),
                depth: 1,
            }),
            tool_call_id: Some("call-1".to_string()),
            resolved_overrides: astrcode_core::ResolvedSubagentContextOverrides::default(),
            resolved_limits: astrcode_core::ResolvedExecutionLimitsSnapshot::default(),
            timestamp: Some(Utc::now()),
        },
        StorageEvent::UserMessage {
            turn_id: Some("turn-level1".to_string()),
            agent: sub1,
            content: "level1 message".to_string(),
            origin: astrcode_core::UserMessageOrigin::User,
            timestamp: Utc::now(),
        },
        StorageEvent::SubRunStarted {
            turn_id: Some("turn-level1".to_string()),
            agent: sub2.clone(),
            descriptor: Some(astrcode_core::SubRunDescriptor {
                sub_run_id: "sub-level2".to_string(),
                parent_turn_id: "turn-level1".to_string(),
                parent_agent_id: Some("agent-level1".to_string()),
                depth: 2,
            }),
            tool_call_id: Some("call-2".to_string()),
            resolved_overrides: astrcode_core::ResolvedSubagentContextOverrides::default(),
            resolved_limits: astrcode_core::ResolvedExecutionLimitsSnapshot::default(),
            timestamp: Some(Utc::now()),
        },
        StorageEvent::UserMessage {
            turn_id: Some("turn-level2".to_string()),
            agent: sub2,
            content: "level2 message".to_string(),
            origin: astrcode_core::UserMessageOrigin::User,
            timestamp: Utc::now(),
        },
        StorageEvent::SubRunStarted {
            turn_id: Some("turn-level2".to_string()),
            agent: sub3.clone(),
            descriptor: Some(astrcode_core::SubRunDescriptor {
                sub_run_id: "sub-level3".to_string(),
                parent_turn_id: "turn-level2".to_string(),
                parent_agent_id: Some("agent-level2".to_string()),
                depth: 3,
            }),
            tool_call_id: Some("call-3".to_string()),
            resolved_overrides: astrcode_core::ResolvedSubagentContextOverrides::default(),
            resolved_limits: astrcode_core::ResolvedExecutionLimitsSnapshot::default(),
            timestamp: Some(Utc::now()),
        },
        StorageEvent::UserMessage {
            turn_id: Some("turn-level3".to_string()),
            agent: sub3,
            content: "level3 message".to_string(),
            origin: astrcode_core::UserMessageOrigin::User,
            timestamp: Utc::now(),
        },
    ] {
        log.append(&event).expect("event should append");
    }
}

#[tokio::test]
async fn scope_parameter_without_subrun_id_is_rejected() {
    // Why: scope 参数只有在提供 subRunId 时才有意义
    let (state, _guard) = test_state(None);
    seed_nested_subrun_hierarchy("scope-no-subrun-session", _guard.path());
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/sessions/scope-no-subrun-session/history?scope=directChildren")
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    // 应该返回 400 或其他客户端错误
    assert!(
        response.status().is_client_error(),
        "scope without subRunId should be rejected, got: {}",
        response.status()
    );
}

fn seed_child_delivery_contract_session(session_id: &str, working_dir: &std::path::Path) {
    let mut log =
        EventLog::create(session_id, working_dir).expect("session file should be created");
    let descriptor = astrcode_core::SubRunDescriptor {
        sub_run_id: "subrun-delivery-contract".to_string(),
        parent_turn_id: "turn-delivery-contract".to_string(),
        parent_agent_id: Some("agent-parent".to_string()),
        depth: 1,
    };
    let agent = AgentEventContext::sub_run(
        "agent-child-contract",
        "turn-delivery-contract",
        "review",
        "subrun-delivery-contract",
        astrcode_core::SubRunStorageMode::IndependentSession,
        Some("session-child-contract".to_string()),
    );
    let child_ref = astrcode_core::ChildAgentRef {
        agent_id: "agent-child-contract".to_string(),
        session_id: session_id.to_string(),
        sub_run_id: "subrun-delivery-contract".to_string(),
        parent_agent_id: Some("agent-parent".to_string()),
        lineage_kind: astrcode_core::ChildSessionLineageKind::Spawn,
        status: astrcode_core::AgentStatus::Completed,
        openable: true,
        open_session_id: "session-child-contract".to_string(),
    };

    for event in [
        StorageEvent::SessionStart {
            session_id: session_id.to_string(),
            timestamp: Utc::now(),
            working_dir: working_dir.display().to_string(),
            parent_session_id: None,
            parent_storage_seq: None,
        },
        StorageEvent::SubRunStarted {
            turn_id: Some("turn-delivery-contract".to_string()),
            agent: agent.clone(),
            descriptor: Some(descriptor.clone()),
            tool_call_id: Some("call-delivery-contract".to_string()),
            resolved_overrides: astrcode_core::ResolvedSubagentContextOverrides {
                storage_mode: astrcode_core::SubRunStorageMode::IndependentSession,
                ..Default::default()
            },
            resolved_limits: astrcode_core::ResolvedExecutionLimitsSnapshot::default(),
            timestamp: Some(Utc::now()),
        },
        StorageEvent::SubRunFinished {
            turn_id: Some("turn-delivery-contract".to_string()),
            agent: agent.clone(),
            descriptor: Some(descriptor),
            tool_call_id: Some("call-delivery-contract".to_string()),
            result: astrcode_core::SubRunResult {
                status: astrcode_core::SubRunOutcome::Completed,
                handoff: Some(astrcode_core::SubRunHandoff {
                    summary: "child final reply summary".to_string(),
                    findings: Vec::new(),
                    artifacts: Vec::new(),
                }),
                failure: None,
            },
            step_count: 2,
            estimated_tokens: 88,
            timestamp: Some(Utc::now()),
        },
        StorageEvent::ChildSessionNotification {
            turn_id: Some("turn-delivery-contract".to_string()),
            agent,
            notification: astrcode_core::ChildSessionNotification {
                notification_id: "child-terminal:subrun-delivery-contract:completed".to_string(),
                child_ref,
                kind: astrcode_core::ChildSessionNotificationKind::Delivered,
                summary: "child final reply summary".to_string(),
                status: astrcode_core::AgentStatus::Completed,
                open_session_id: "session-child-contract".to_string(),
                source_tool_call_id: Some("call-delivery-contract".to_string()),
                final_reply_excerpt: Some("final answer excerpt".to_string()),
            },
            timestamp: Some(Utc::now()),
        },
    ] {
        log.append(&event).expect("event should append");
    }
}

#[tokio::test]
async fn child_delivery_projection_contract_exposes_status_source_and_final_excerpt() {
    let (state, _guard) = test_state(None);
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    seed_child_delivery_contract_session("child-delivery-contract-session", temp_dir.path());
    let app = build_api_router().with_state(state);

    let status_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(
                    "/api/v1/sessions/child-delivery-contract-session/subruns/\
                     subrun-delivery-contract",
                )
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");
    assert_eq!(status_response.status(), StatusCode::OK);
    let status_payload: SubRunStatusDto = serde_json::from_slice(
        &to_bytes(status_response.into_body(), usize::MAX)
            .await
            .expect("body should be readable"),
    )
    .expect("payload should deserialize");
    assert_eq!(status_payload.source, SubRunStatusSourceDto::Durable);
    assert_eq!(status_payload.status, "completed");

    let history_response = app
        .oneshot(
            Request::builder()
                .uri("/api/sessions/child-delivery-contract-session/history")
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");
    assert_eq!(history_response.status(), StatusCode::OK);
    let history_payload: SessionHistoryResponseDto = serde_json::from_slice(
        &to_bytes(history_response.into_body(), usize::MAX)
            .await
            .expect("body should be readable"),
    )
    .expect("history payload should deserialize");

    let delivery_event = history_payload
        .events
        .iter()
        .find_map(|envelope| match &envelope.event {
            AgentEventPayload::ChildSessionNotification {
                kind,
                status,
                open_session_id,
                final_reply_excerpt,
                ..
            } => Some((
                kind.clone(),
                status.clone(),
                open_session_id.clone(),
                final_reply_excerpt.clone(),
            )),
            _ => None,
        })
        .expect("child delivery notification event should exist");

    assert_eq!(delivery_event.0, ChildSessionNotificationKindDto::Delivered);
    assert_eq!(delivery_event.1, "completed");
    assert_eq!(delivery_event.2, "session-child-contract");
    assert_eq!(delivery_event.3.as_deref(), Some("final answer excerpt"));
}

// ============================================================================
// T019: Parent Summary List and Direct Child-Session Loading Contract Tests
// ============================================================================

/// 植入一个包含两个 child session 的父会话事件日志，
/// 一个成功完成，一个执行失败。
fn seed_parent_summary_list_session(session_id: &str, working_dir: &std::path::Path) {
    let mut log =
        EventLog::create(session_id, working_dir).expect("session file should be created");

    let child_ref_ok = astrcode_core::ChildAgentRef {
        agent_id: "agent-child-ok".to_string(),
        session_id: session_id.to_string(),
        sub_run_id: "subrun-ok".to_string(),
        parent_agent_id: Some("agent-parent".to_string()),
        lineage_kind: astrcode_core::ChildSessionLineageKind::Spawn,
        status: astrcode_core::AgentStatus::Completed,
        openable: true,
        open_session_id: "session-child-ok".to_string(),
    };
    let child_ref_fail = astrcode_core::ChildAgentRef {
        agent_id: "agent-child-fail".to_string(),
        session_id: session_id.to_string(),
        sub_run_id: "subrun-fail".to_string(),
        parent_agent_id: Some("agent-parent".to_string()),
        lineage_kind: astrcode_core::ChildSessionLineageKind::Spawn,
        status: astrcode_core::AgentStatus::Failed,
        openable: true,
        open_session_id: "session-child-fail".to_string(),
    };

    let agent_ok = AgentEventContext::sub_run(
        "agent-child-ok",
        "turn-parent",
        "explore",
        "subrun-ok",
        astrcode_core::SubRunStorageMode::IndependentSession,
        Some("session-child-ok".to_string()),
    );
    let agent_fail = AgentEventContext::sub_run(
        "agent-child-fail",
        "turn-parent",
        "explore",
        "subrun-fail",
        astrcode_core::SubRunStorageMode::IndependentSession,
        Some("session-child-fail".to_string()),
    );

    for event in [
        StorageEvent::SessionStart {
            session_id: session_id.to_string(),
            timestamp: Utc::now(),
            working_dir: working_dir.display().to_string(),
            parent_session_id: None,
            parent_storage_seq: None,
        },
        // 第一个 child：成功交付
        StorageEvent::ChildSessionNotification {
            turn_id: Some("turn-parent".to_string()),
            agent: agent_ok.clone(),
            notification: astrcode_core::ChildSessionNotification {
                notification_id: "child-terminal:subrun-ok:delivered".to_string(),
                child_ref: child_ref_ok,
                kind: astrcode_core::ChildSessionNotificationKind::Delivered,
                summary: "成功完成代码审查".to_string(),
                status: astrcode_core::AgentStatus::Completed,
                open_session_id: "session-child-ok".to_string(),
                source_tool_call_id: Some("call-ok".to_string()),
                final_reply_excerpt: Some("审查结果：代码质量良好".to_string()),
            },
            timestamp: Some(Utc::now()),
        },
        // 第二个 child：失败通知
        StorageEvent::ChildSessionNotification {
            turn_id: Some("turn-parent".to_string()),
            agent: agent_fail,
            notification: astrcode_core::ChildSessionNotification {
                notification_id: "child-terminal:subrun-fail:failed".to_string(),
                child_ref: child_ref_fail,
                kind: astrcode_core::ChildSessionNotificationKind::Failed,
                summary: "子 Agent 执行失败".to_string(),
                status: astrcode_core::AgentStatus::Failed,
                open_session_id: "session-child-fail".to_string(),
                source_tool_call_id: Some("call-fail".to_string()),
                final_reply_excerpt: None,
            },
            timestamp: Some(Utc::now()),
        },
    ] {
        log.append(&event).expect("event should append");
    }
}

/// 植入一个独立的 child session 事件日志，包含 thinking、tool activity 和 final reply。
fn seed_child_session_with_full_transcript(session_id: &str, working_dir: &std::path::Path) {
    let mut log =
        EventLog::create(session_id, working_dir).expect("session file should be created");
    let root = AgentEventContext::root_execution("agent-child-ok", "explore");

    for event in [
        StorageEvent::SessionStart {
            session_id: session_id.to_string(),
            timestamp: Utc::now(),
            working_dir: working_dir.display().to_string(),
            parent_session_id: Some("session-parent-direct".to_string()),
            parent_storage_seq: Some(2),
        },
        StorageEvent::UserMessage {
            turn_id: Some("turn-child".to_string()),
            agent: root.clone(),
            content: "审查 src/main.rs 文件".to_string(),
            origin: astrcode_core::UserMessageOrigin::User,
            timestamp: Utc::now(),
        },
        StorageEvent::ThinkingDelta {
            turn_id: Some("turn-child".to_string()),
            agent: root.clone(),
            token: "分析文件结构...".to_string(),
        },
        StorageEvent::AssistantFinal {
            turn_id: Some("turn-child".to_string()),
            agent: root,
            content: "审查结果：代码质量良好，无需修改。".to_string(),
            reasoning_content: None,
            reasoning_signature: None,
            timestamp: Some(Utc::now()),
        },
    ] {
        log.append(&event).expect("event should append");
    }
}

#[tokio::test]
async fn parent_summary_list_contract_returns_child_notifications_from_history() {
    // Why: 父会话的 /children/summary 必须返回所有可消费的 child notification，
    // 而不是 child 原始事件流，保证父视图只展示摘要
    let (state, _guard) = test_state(None);
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    seed_parent_summary_list_session("parent-summary-session", temp_dir.path());
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/sessions/parent-summary-session/children/summary")
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::OK);
    let payload: ParentChildSummaryListResponseDto = serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should be readable"),
    )
    .expect("payload should deserialize");

    assert_eq!(payload.items.len(), 2);

    // 验证第一个 child 通知：成功交付
    let ok_notification = payload
        .items
        .iter()
        .find(|item| item.child_ref.agent_id == "agent-child-ok")
        .expect("should find completed child notification");
    assert_eq!(
        ok_notification.kind,
        ChildSessionNotificationKindDto::Delivered
    );
    assert_eq!(ok_notification.status, "completed");
    assert_eq!(ok_notification.open_session_id, "session-child-ok");
    assert_eq!(
        ok_notification.final_reply_excerpt.as_deref(),
        Some("审查结果：代码质量良好")
    );

    // 验证第二个 child 通知：失败
    let fail_notification = payload
        .items
        .iter()
        .find(|item| item.child_ref.agent_id == "agent-child-fail")
        .expect("should find failed child notification");
    assert_eq!(
        fail_notification.kind,
        ChildSessionNotificationKindDto::Failed
    );
    assert_eq!(fail_notification.status, "failed");
    assert_eq!(fail_notification.open_session_id, "session-child-fail");
    assert!(fail_notification.final_reply_excerpt.is_none());
}

#[tokio::test]
async fn parent_summary_list_contract_notification_contains_stable_child_ref() {
    // Why: child_ref 是前端定位和打开子会话的唯一 stable 引用，
    // 必须包含 agentId、sessionId、subRunId、lineageKind、openSessionId
    let (state, _guard) = test_state(None);
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    seed_parent_summary_list_session("parent-ref-session", temp_dir.path());
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/sessions/parent-ref-session/children/summary")
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    let payload: ParentChildSummaryListResponseDto = serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should be readable"),
    )
    .expect("payload should deserialize");

    let child_ref = &payload
        .items
        .into_iter()
        .find(|item| item.child_ref.agent_id == "agent-child-ok")
        .expect("should find ok child")
        .child_ref;

    assert_eq!(child_ref.agent_id, "agent-child-ok");
    assert_eq!(child_ref.sub_run_id, "subrun-ok");
    assert_eq!(child_ref.lineage_kind, ChildSessionLineageKindDto::Spawn);
    assert!(child_ref.openable);
    assert_eq!(child_ref.open_session_id, "session-child-ok");
    assert_eq!(child_ref.parent_agent_id.as_deref(), Some("agent-parent"));
}

#[tokio::test]
async fn child_session_direct_loading_contract_returns_full_transcript() {
    // Why: 子会话必须通过标准 session history 入口直接加载，
    // 不能要求调用方从父会话 history 里重新过滤
    let (state, _guard) = test_state(None);
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    seed_child_session_with_full_transcript("session-child-direct", temp_dir.path());
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/sessions/session-child-direct/history")
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::OK);
    let payload: SessionHistoryResponseDto = serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should be readable"),
    )
    .expect("payload should deserialize");

    // 子会话必须包含 thinking、tool activity 和 final reply，
    // 而不是父视图的摘要通知
    let has_thinking = payload.events.iter().any(|envelope| {
        matches!(
            &envelope.event,
            AgentEventPayload::ThinkingDelta { delta, .. } if delta == "分析文件结构..."
        )
    });
    let has_assistant = payload.events.iter().any(|envelope| {
        matches!(
            &envelope.event,
            AgentEventPayload::AssistantMessage { content, .. } if content.contains("审查结果")
        )
    });
    assert!(has_thinking, "child session should contain thinking delta");
    assert!(
        has_assistant,
        "child session should contain assistant message"
    );

    // 不应包含 ChildSessionNotification（那是父视图的摘要投影）
    let has_parent_notification = payload.events.iter().any(|envelope| {
        matches!(
            &envelope.event,
            AgentEventPayload::ChildSessionNotification { .. }
        )
    });
    assert!(
        !has_parent_notification,
        "child session should not contain parent summary notifications"
    );
}

#[tokio::test]
async fn child_session_view_projection_contract_returns_readable_summary() {
    // Why: GET /api/sessions/:id/children/:child_session_id/view 必须返回
    // 可读的 child-session view projection，不含 raw JSON
    let (state, _guard) = test_state(None);
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    seed_parent_summary_list_session("parent-view-session", temp_dir.path());
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/sessions/parent-view-session/children/session-child-ok/view")
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let body_str = String::from_utf8(body_bytes.to_vec()).expect("body should be utf8");

    // 反序列化为 ChildSessionViewProjectionDto 验证结构
    let payload: serde_json::Value =
        serde_json::from_str(&body_str).expect("payload should be valid json");
    let view = payload
        .get("view")
        .expect("payload should have 'view' field");

    // 验证 child_ref 字段
    let child_ref = view
        .get("childRef")
        .expect("view should have childRef field");
    assert_eq!(child_ref["agentId"].as_str(), Some("agent-child-ok"));
    assert_eq!(
        child_ref["openSessionId"].as_str(),
        Some("session-child-ok")
    );

    // 验证 status 和 hasFinalReply 字段
    assert_eq!(view["status"].as_str(), Some("completed"));
    assert_eq!(view["hasFinalReply"].as_bool(), Some(true));

    // 验证不含 raw JSON 字段（如 inboxEnvelope、payload 等）
    assert!(
        view.get("inboxEnvelope").is_none(),
        "view projection must not contain inbox envelope"
    );
    assert!(
        view.get("rawPayload").is_none(),
        "view projection must not contain raw payload"
    );
}
