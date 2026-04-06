use std::{net::TcpListener, sync::Arc, time::Duration};

use astrcode_core::{PluginRegistry, RuntimeCoordinator, RuntimeHandle};
use astrcode_protocol::http::{PromptAcceptedResponse, PromptRequest};
use astrcode_runtime::{
    Config, ModelConfig, Profile, RuntimeConfig, RuntimeGovernance, RuntimeService,
    config::PROVIDER_KIND_OPENAI, save_config,
};
use astrcode_runtime_registry::CapabilityRouter;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
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
        .create_session(temp_dir.path())
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
        .create_session(temp_dir.path())
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
        .create_session(temp_dir.path())
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
}
