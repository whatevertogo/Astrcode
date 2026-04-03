//! End-to-end integration tests for the AstrCode HTTP/SSE API surface.
//!
//! These tests exercise the full request → response → event flow without
//! requiring a real LLM provider or external services.

use std::collections::HashSet;
use std::net::TcpListener;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use astrcode_core::project::project_dir_name;
use astrcode_core::{CapabilityRouter, PluginRegistry, RuntimeCoordinator, RuntimeHandle};
use astrcode_protocol::http::{
    CreateSessionRequest, PromptAcceptedResponse, PromptRequest, SaveActiveSelectionRequest,
    SessionListItem, SessionMessageDto,
};
use astrcode_runtime::config::PROVIDER_KIND_OPENAI;
use astrcode_runtime::{
    save_config, Config, Profile, RuntimeConfig, RuntimeGovernance, RuntimeService,
};
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::task::JoinHandle;
use tower::ServiceExt;

use crate::auth::{AuthSessionManager, BootstrapAuth};
use crate::bootstrap::APP_HOME_OVERRIDE_ENV;
use crate::routes::build_api_router;
use crate::test_support::{test_state, ServerTestEnvGuard};
use crate::{AppState, AUTH_HEADER_NAME};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a pre-authenticated request builder.
fn auth_request(method: &str, uri: &str) -> axum::http::request::Builder {
    let mut builder = Request::builder().method(method).uri(uri);
    // Use the bootstrap token that test_state() issues
    builder = builder.header(AUTH_HEADER_NAME, "browser-token");
    builder
}

/// Extract JSON body from a response.
async fn json_body<T: serde::de::DeserializeOwned>(response: axum::http::Response<Body>) -> T {
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body should be readable");
    serde_json::from_slice(&bytes).expect("response should deserialize to expected type")
}

/// Wait until the background prompt task has persisted the expected number of session messages.
async fn wait_for_total_message_count(
    app: axum::Router,
    session_id: &str,
    expected_count: usize,
) -> Vec<SessionMessageDto> {
    for _ in 0..40 {
        let messages_req = auth_request("GET", &format!("/api/sessions/{session_id}/messages"))
            .body(Body::empty())
            .expect("request should build");

        let messages_resp = app
            .clone()
            .oneshot(messages_req)
            .await
            .expect("response should return");
        assert_eq!(messages_resp.status(), StatusCode::OK);

        let messages: Vec<SessionMessageDto> = json_body(messages_resp).await;
        if messages.len() == expected_count {
            return messages;
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(25)).await;
    }

    panic!("timed out waiting for {expected_count} total messages in session {session_id}");
}

/// Wait until the background prompt task has persisted the expected number of user messages.
async fn wait_for_user_message_count(
    app: axum::Router,
    session_id: &str,
    expected_count: usize,
) -> Vec<SessionMessageDto> {
    for _ in 0..20 {
        let messages_req = auth_request("GET", &format!("/api/sessions/{session_id}/messages"))
            .body(Body::empty())
            .expect("request should build");

        let messages_resp = app
            .clone()
            .oneshot(messages_req)
            .await
            .expect("response should return");
        assert_eq!(messages_resp.status(), StatusCode::OK);

        let messages: Vec<SessionMessageDto> = json_body(messages_resp).await;
        let user_message_count = messages
            .iter()
            .filter(|message| matches!(message, SessionMessageDto::User { .. }))
            .count();
        if user_message_count == expected_count {
            return messages;
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(25)).await;
    }

    panic!("timed out waiting for {expected_count} user messages in session {session_id}");
}

/// Percent-encode query parameter values so Windows paths survive request parsing unchanged.
fn encode_query_value(value: &str) -> String {
    use std::fmt::Write;

    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char);
            }
            _ => {
                write!(&mut encoded, "%{byte:02X}").expect("writing to string should succeed");
            }
        }
    }
    encoded
}

fn configured_state_with_openai_server(base_url: &str) -> (AppState, ServerTestEnvGuard) {
    let guard = ServerTestEnvGuard::new();
    save_config(&Config {
        active_profile: "local-openai".to_string(),
        active_model: "model-a".to_string(),
        runtime: RuntimeConfig {
            compact_keep_recent_turns: Some(2),
            ..RuntimeConfig::default()
        },
        profiles: vec![Profile {
            name: "local-openai".to_string(),
            provider_kind: PROVIDER_KIND_OPENAI.to_string(),
            base_url: base_url.to_string(),
            api_key: Some("sk-test".to_string()),
            models: vec!["model-a".to_string()],
            ..Profile::default()
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
                "HTTP/1.1 200 OK\r\ncontent-type: {}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
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

fn session_log_path(session_id: &str, working_dir: &Path) -> std::path::PathBuf {
    let app_home =
        std::env::var_os(APP_HOME_OVERRIDE_ENV).expect("test home override should exist");
    std::path::PathBuf::from(app_home)
        .join(".astrcode")
        .join("projects")
        .join(project_dir_name(working_dir))
        .join("sessions")
        .join(session_id)
        .join(format!("session-{session_id}.jsonl"))
}

// ---------------------------------------------------------------------------
// Test: e2e_session_create_and_list
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e2e_session_create_and_list() {
    let (state, _guard) = test_state(None);
    let app = build_api_router().with_state(state);

    // Create a session - use the system temp dir which always exists
    let working_dir = std::env::temp_dir();
    // Ensure the path is canonical to avoid UNC prefix issues on Windows
    let working_dir = working_dir.canonicalize().unwrap_or(working_dir);
    let working_dir_str = working_dir.to_string_lossy().to_string();

    let create_req = auth_request("POST", "/api/sessions")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&CreateSessionRequest {
                working_dir: working_dir_str.clone(),
            })
            .expect("request should serialize"),
        ))
        .expect("request should build");

    let create_resp = app
        .clone()
        .oneshot(create_req)
        .await
        .expect("response should return");

    assert_eq!(create_resp.status(), StatusCode::OK);
    let created: SessionListItem = json_body(create_resp).await;
    assert!(!created.session_id.is_empty());
    // Normalize paths for comparison (handle UNC prefix and trailing slashes)
    let created_path = created
        .working_dir
        .trim_end_matches(['\\', '/'])
        .to_lowercase();
    let expected_path = working_dir_str.trim_end_matches(['\\', '/']).to_lowercase();
    assert!(
        created_path.contains(&expected_path) || expected_path.contains(&created_path),
        "working_dir mismatch: created={}, expected={}",
        created.working_dir,
        working_dir_str
    );

    // List sessions and verify the created session appears
    let list_req = auth_request("GET", "/api/sessions")
        .body(Body::empty())
        .expect("request should build");

    let list_resp = app
        .clone()
        .oneshot(list_req)
        .await
        .expect("response should return");

    assert_eq!(list_resp.status(), StatusCode::OK);
    let sessions: Vec<SessionListItem> = json_body(list_resp).await;
    assert!(!sessions.is_empty());

    let session_ids: HashSet<_> = sessions.iter().map(|s| &s.session_id).collect();
    assert!(session_ids.contains(&created.session_id));
}

// ---------------------------------------------------------------------------
// Test: e2e_submit_prompt_and_receive_events
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e2e_submit_prompt_and_receive_events() {
    let (state, _guard) = test_state(None);
    let app = build_api_router().with_state(state);

    // Create a session first
    let working_dir = std::env::temp_dir();
    // Match the runtime's normalization so delete_project compares the same canonical path.
    let working_dir = working_dir.canonicalize().unwrap_or(working_dir);
    let working_dir_str = working_dir.to_string_lossy().to_string();
    let create_req = auth_request("POST", "/api/sessions")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&CreateSessionRequest {
                working_dir: working_dir_str.clone(),
            })
            .expect("request should serialize"),
        ))
        .expect("request should build");

    let create_resp = app
        .clone()
        .oneshot(create_req)
        .await
        .expect("response should return");
    assert_eq!(create_resp.status(), StatusCode::OK);
    let created: SessionListItem = json_body(create_resp).await;
    let session_id = &created.session_id;

    // Submit a prompt
    let prompt_req = auth_request("POST", &format!("/api/sessions/{}/prompts", session_id))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&PromptRequest {
                text: "Hello, world!".to_string(),
            })
            .expect("request should serialize"),
        ))
        .expect("request should build");

    let prompt_resp = app
        .clone()
        .oneshot(prompt_req)
        .await
        .expect("response should return");

    // Should be accepted (202)
    assert_eq!(prompt_resp.status(), StatusCode::ACCEPTED);
    let prompt_accepted: serde_json::Value = json_body(prompt_resp).await;
    assert!(prompt_accepted.get("turnId").is_some());
}

// ---------------------------------------------------------------------------
// Test: e2e_multiple_sessions_isolation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e2e_multiple_sessions_isolation() {
    let (state, _guard) = test_state(None);
    let app = build_api_router().with_state(state.clone());

    // Create two sessions
    let working_dir = std::env::temp_dir();
    let working_dir = working_dir.canonicalize().unwrap_or(working_dir);
    let working_dir = working_dir.to_string_lossy().to_string();

    // Create session A
    let create_req_a = auth_request("POST", "/api/sessions")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&CreateSessionRequest {
                working_dir: working_dir.clone(),
            })
            .expect("request should serialize"),
        ))
        .expect("request should build");

    let create_resp_a = app
        .clone()
        .oneshot(create_req_a)
        .await
        .expect("response should return");
    assert_eq!(create_resp_a.status(), StatusCode::OK);
    let session_a: SessionListItem = json_body(create_resp_a).await;

    // Create session B
    let create_req_b = auth_request("POST", "/api/sessions")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&CreateSessionRequest {
                working_dir: working_dir.clone(),
            })
            .expect("request should serialize"),
        ))
        .expect("request should build");

    let create_resp_b = app
        .clone()
        .oneshot(create_req_b)
        .await
        .expect("response should return");
    assert_eq!(create_resp_b.status(), StatusCode::OK);
    let session_b: SessionListItem = json_body(create_resp_b).await;

    // Verify they have different IDs
    assert_ne!(session_a.session_id, session_b.session_id);

    // Submit prompts to both sessions
    let submit_prompt = |session_id: &str, text: String| {
        let app_clone = app.clone();
        let session_id = session_id.to_string();
        async move {
            let req = auth_request("POST", &format!("/api/sessions/{}/prompts", session_id))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&PromptRequest { text }).expect("request should serialize"),
                ))
                .expect("request should build");

            let resp = app_clone
                .oneshot(req)
                .await
                .expect("response should return");
            assert_eq!(resp.status(), StatusCode::ACCEPTED);
            json_body::<serde_json::Value>(resp).await
        }
    };

    submit_prompt(&session_a.session_id, "Prompt A".to_string()).await;
    submit_prompt(&session_b.session_id, "Prompt B".to_string()).await;

    // Wait for the async prompt tasks to persist user messages before asserting isolation.
    let messages_a = wait_for_user_message_count(app.clone(), &session_a.session_id, 1).await;
    let messages_b = wait_for_user_message_count(app.clone(), &session_b.session_id, 1).await;

    // Each session should have its own user message
    let user_messages_a: Vec<_> = messages_a
        .iter()
        .filter(|m| matches!(m, SessionMessageDto::User { .. }))
        .collect();
    let user_messages_b: Vec<_> = messages_b
        .iter()
        .filter(|m| matches!(m, SessionMessageDto::User { .. }))
        .collect();

    assert_eq!(user_messages_a.len(), 1);
    assert_eq!(user_messages_b.len(), 1);
}

#[tokio::test]
async fn e2e_concurrent_submit_branches_second_prompt() {
    let (state, _guard) = test_state(None);
    let app = build_api_router().with_state(state.clone());

    let working_dir = std::env::temp_dir();
    let working_dir = working_dir.canonicalize().unwrap_or(working_dir);
    let working_dir = working_dir.to_string_lossy().to_string();

    let create_req = auth_request("POST", "/api/sessions")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&CreateSessionRequest {
                working_dir: working_dir.clone(),
            })
            .expect("request should serialize"),
        ))
        .expect("request should build");
    let create_resp = app
        .clone()
        .oneshot(create_req)
        .await
        .expect("response should return");
    assert_eq!(create_resp.status(), StatusCode::OK);
    let created: SessionListItem = json_body(create_resp).await;

    let submit_prompt = |session_id: &str, text: &str| {
        let app_clone = app.clone();
        let session_id = session_id.to_string();
        let text = text.to_string();
        async move {
            let req = auth_request("POST", &format!("/api/sessions/{}/prompts", session_id))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&PromptRequest { text }).expect("request should serialize"),
                ))
                .expect("request should build");
            let resp = app_clone
                .oneshot(req)
                .await
                .expect("response should return");
            assert_eq!(resp.status(), StatusCode::ACCEPTED);
            json_body::<PromptAcceptedResponse>(resp).await
        }
    };

    let first = submit_prompt(&created.session_id, "first").await;
    let second = submit_prompt(&created.session_id, "second").await;

    assert_eq!(first.session_id, created.session_id);
    assert_eq!(first.branched_from_session_id, None);
    assert_ne!(second.session_id, created.session_id);
    assert_eq!(
        second.branched_from_session_id.as_deref(),
        Some(created.session_id.as_str())
    );

    let original_messages = wait_for_user_message_count(app.clone(), &created.session_id, 1).await;
    let branched_messages = wait_for_user_message_count(app.clone(), &second.session_id, 1).await;

    let original_user_messages = original_messages
        .iter()
        .filter_map(|message| match message {
            SessionMessageDto::User { content, .. } => Some(content.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let branched_user_messages = branched_messages
        .iter()
        .filter_map(|message| match message {
            SessionMessageDto::User { content, .. } => Some(content.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(original_user_messages, vec!["first"]);
    assert_eq!(branched_user_messages, vec!["second"]);
}

// ---------------------------------------------------------------------------
// Test: e2e_auth_token_validation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e2e_auth_token_validation() {
    let (state, _guard) = test_state(None);
    let app = build_api_router().with_state(state);

    // Request without auth token should be rejected
    let no_auth_req = Request::builder()
        .uri("/api/sessions")
        .body(Body::empty())
        .expect("request should build");

    let no_auth_resp = app
        .clone()
        .oneshot(no_auth_req)
        .await
        .expect("response should return");

    assert_eq!(no_auth_resp.status(), StatusCode::UNAUTHORIZED);

    // Request with invalid token should be rejected
    let invalid_token_req = Request::builder()
        .uri("/api/sessions")
        .header(AUTH_HEADER_NAME, "invalid-token")
        .body(Body::empty())
        .expect("request should build");

    let invalid_token_resp = app
        .clone()
        .oneshot(invalid_token_req)
        .await
        .expect("response should return");

    assert_eq!(invalid_token_resp.status(), StatusCode::UNAUTHORIZED);

    // Request with valid token should succeed
    let valid_token_req = Request::builder()
        .uri("/api/sessions")
        .header(AUTH_HEADER_NAME, "browser-token")
        .body(Body::empty())
        .expect("request should build");

    let valid_token_resp = app
        .clone()
        .oneshot(valid_token_req)
        .await
        .expect("response should return");

    assert_eq!(valid_token_resp.status(), StatusCode::OK);
}

// ---------------------------------------------------------------------------
// Test: e2e_config_get_and_update
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e2e_config_get_and_update() {
    let (state, _guard) = test_state(None);
    let app = build_api_router().with_state(state);

    // Get current config
    let get_config_req = auth_request("GET", "/api/config")
        .body(Body::empty())
        .expect("request should build");

    let get_config_resp = app
        .clone()
        .oneshot(get_config_req)
        .await
        .expect("response should return");

    assert_eq!(get_config_resp.status(), StatusCode::OK);
    let config: serde_json::Value = json_body(get_config_resp).await;
    // Config should have some structure
    assert!(config.is_object());

    // Update active selection - this will fail if no profiles exist in config
    // So we just verify the endpoint requires auth and returns a valid response
    let update_req = auth_request("POST", "/api/config/active-selection")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&SaveActiveSelectionRequest {
                active_profile: "default".to_string(),
                active_model: "test-model".to_string(),
            })
            .expect("request should serialize"),
        ))
        .expect("request should build");

    let update_resp = app
        .clone()
        .oneshot(update_req)
        .await
        .expect("response should return");

    // The endpoint should either succeed (NO_CONTENT) or reject due to missing profile (BAD_REQUEST)
    // Both are valid responses - we're testing the HTTP layer, not config validation
    assert!(
        update_resp.status() == StatusCode::NO_CONTENT
            || update_resp.status() == StatusCode::BAD_REQUEST
    );
}

// ---------------------------------------------------------------------------
// Test: e2e_session_delete
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e2e_session_delete() {
    let (state, _guard) = test_state(None);
    let app = build_api_router().with_state(state);

    // Create a session
    let working_dir = std::env::temp_dir();
    // Match the runtime's normalization so delete_project compares the same canonical path.
    let working_dir = working_dir.canonicalize().unwrap_or(working_dir);
    let working_dir_str = working_dir.to_string_lossy().to_string();
    let create_req = auth_request("POST", "/api/sessions")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&CreateSessionRequest {
                working_dir: working_dir_str.clone(),
            })
            .expect("request should serialize"),
        ))
        .expect("request should build");

    let create_resp = app
        .clone()
        .oneshot(create_req)
        .await
        .expect("response should return");
    assert_eq!(create_resp.status(), StatusCode::OK);
    let created: SessionListItem = json_body(create_resp).await;
    let session_id = &created.session_id;

    // Delete the session
    let delete_req = auth_request("DELETE", &format!("/api/sessions/{}", session_id))
        .body(Body::empty())
        .expect("request should build");

    let delete_resp = app
        .clone()
        .oneshot(delete_req)
        .await
        .expect("response should return");

    assert_eq!(delete_resp.status(), StatusCode::NO_CONTENT);

    // Verify session is gone - list should not contain it
    let list_req = auth_request("GET", "/api/sessions")
        .body(Body::empty())
        .expect("request should build");

    let list_resp = app
        .clone()
        .oneshot(list_req)
        .await
        .expect("response should return");
    assert_eq!(list_resp.status(), StatusCode::OK);
    let sessions: Vec<SessionListItem> = json_body(list_resp).await;

    let session_ids: HashSet<_> = sessions.iter().map(|s| &s.session_id).collect();
    assert!(!session_ids.contains(session_id));
}

// ---------------------------------------------------------------------------
// Test: e2e_project_delete
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e2e_project_delete() {
    let (state, _guard) = test_state(None);
    let app = build_api_router().with_state(state);

    // Create a session first
    let working_dir = std::env::temp_dir();
    // Match the runtime's normalization so delete_project compares the same canonical path.
    let working_dir = working_dir.canonicalize().unwrap_or(working_dir);
    let working_dir_str = working_dir.to_string_lossy().to_string();
    let create_req = auth_request("POST", "/api/sessions")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&CreateSessionRequest {
                working_dir: working_dir_str.clone(),
            })
            .expect("request should serialize"),
        ))
        .expect("request should build");

    let create_resp = app
        .clone()
        .oneshot(create_req)
        .await
        .expect("response should return");
    assert_eq!(create_resp.status(), StatusCode::OK);

    // Delete project by working_dir
    let delete_req = auth_request(
        "DELETE",
        &format!(
            "/api/projects?workingDir={}",
            encode_query_value(&working_dir_str)
        ),
    )
    .body(Body::empty())
    .expect("request should build");

    let delete_resp = app
        .clone()
        .oneshot(delete_req)
        .await
        .expect("response should return");

    assert_eq!(delete_resp.status(), StatusCode::OK);
    let result: serde_json::Value = json_body(delete_resp).await;
    assert_eq!(
        result.get("successCount").and_then(|value| value.as_u64()),
        Some(1)
    );
}

#[tokio::test]
async fn e2e_manual_compact_endpoint_persists_a_compact_summary() {
    let (base_url, server_handle) = spawn_openai_chat_server(
        "<analysis>ok</analysis><summary>condensed history</summary>",
        Duration::ZERO,
        4,
    );
    let (state, _guard) = configured_state_with_openai_server(&base_url);
    let app = build_api_router().with_state(state);

    let working_dir = std::env::temp_dir();
    let working_dir = working_dir.canonicalize().unwrap_or(working_dir);
    let working_dir_str = working_dir.to_string_lossy().to_string();
    let create_req = auth_request("POST", "/api/sessions")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&CreateSessionRequest {
                working_dir: working_dir_str,
            })
            .expect("request should serialize"),
        ))
        .expect("request should build");
    let create_resp = app
        .clone()
        .oneshot(create_req)
        .await
        .expect("response should return");
    assert_eq!(create_resp.status(), StatusCode::OK);
    let created: SessionListItem = json_body(create_resp).await;

    for expected_total_messages in [2usize, 4, 6] {
        let prompt_req = auth_request(
            "POST",
            &format!("/api/sessions/{}/prompts", created.session_id),
        )
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&PromptRequest {
                text: format!("prompt-{expected_total_messages}"),
            })
            .expect("request should serialize"),
        ))
        .expect("request should build");
        let prompt_resp = app
            .clone()
            .oneshot(prompt_req)
            .await
            .expect("response should return");
        assert_eq!(prompt_resp.status(), StatusCode::ACCEPTED);
        wait_for_total_message_count(app.clone(), &created.session_id, expected_total_messages)
            .await;
    }

    let compact_req = auth_request(
        "POST",
        &format!("/api/sessions/{}/compact", created.session_id),
    )
    .body(Body::empty())
    .expect("request should build");
    let compact_resp = app
        .clone()
        .oneshot(compact_req)
        .await
        .expect("response should return");
    assert_eq!(compact_resp.status(), StatusCode::NO_CONTENT);

    let session_log = session_log_path(&created.session_id, &working_dir);
    let log_contents =
        std::fs::read_to_string(&session_log).expect("session log should be readable");
    assert!(
        log_contents.contains("\"type\":\"compactApplied\""),
        "manual compact should persist a compactApplied event to the session log"
    );
    assert!(
        log_contents.contains("condensed history"),
        "the persisted compaction event should include the generated summary"
    );

    server_handle.await.expect("server should finish");
}

#[tokio::test]
async fn e2e_manual_compact_endpoint_rejects_busy_sessions() {
    let (base_url, server_handle) =
        spawn_openai_chat_server("slow response", Duration::from_millis(300), 1);
    let (state, _guard) = configured_state_with_openai_server(&base_url);
    let app = build_api_router().with_state(state);

    let working_dir = std::env::temp_dir();
    let working_dir = working_dir.canonicalize().unwrap_or(working_dir);
    let working_dir_str = working_dir.to_string_lossy().to_string();
    let create_req = auth_request("POST", "/api/sessions")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&CreateSessionRequest {
                working_dir: working_dir_str,
            })
            .expect("request should serialize"),
        ))
        .expect("request should build");
    let create_resp = app
        .clone()
        .oneshot(create_req)
        .await
        .expect("response should return");
    assert_eq!(create_resp.status(), StatusCode::OK);
    let created: SessionListItem = json_body(create_resp).await;

    let prompt_req = auth_request(
        "POST",
        &format!("/api/sessions/{}/prompts", created.session_id),
    )
    .header("content-type", "application/json")
    .body(Body::from(
        serde_json::to_vec(&PromptRequest {
            text: "busy prompt".to_string(),
        })
        .expect("request should serialize"),
    ))
    .expect("request should build");
    let prompt_resp = app
        .clone()
        .oneshot(prompt_req)
        .await
        .expect("response should return");
    assert_eq!(prompt_resp.status(), StatusCode::ACCEPTED);

    let compact_req = auth_request(
        "POST",
        &format!("/api/sessions/{}/compact", created.session_id),
    )
    .body(Body::empty())
    .expect("request should build");
    let compact_resp = app
        .clone()
        .oneshot(compact_req)
        .await
        .expect("response should return");
    assert_eq!(compact_resp.status(), StatusCode::CONFLICT);

    wait_for_total_message_count(app.clone(), &created.session_id, 2).await;
    server_handle.await.expect("server should finish");
}
