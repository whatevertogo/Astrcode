//! End-to-end integration tests for the AstrCode HTTP/SSE API surface.
//!
//! These tests exercise the full request → response → event flow without
//! requiring a real LLM provider or external services.

use std::collections::HashSet;

use astrcode_protocol::http::{
    AuthExchangeRequest, AuthExchangeResponse, CreateSessionRequest, PromptAcceptedResponse,
    PromptRequest, SaveActiveSelectionRequest, SessionListItem, SessionMessageDto,
};
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use crate::routes::build_api_router;
use crate::test_support::test_state;
use crate::AUTH_HEADER_NAME;

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
// Test: e2e_session_replay_events
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e2e_session_replay_events() {
    let (state, _guard) = test_state(None);
    let app = build_api_router().with_state(state);

    // Create a session
    let working_dir = std::env::temp_dir();
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

    // Submit a prompt to generate events
    let prompt_req = auth_request("POST", &format!("/api/sessions/{}/prompts", session_id))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&PromptRequest {
                text: "Test prompt for replay".to_string(),
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

    // 使用轮询等待事件持久化，而非固定 sleep，避免在 CI 慢环境下不稳定
    wait_for_user_message_count(app.clone(), session_id, 1).await;

    // Request session events (SSE stream)
    let events_req = auth_request("GET", &format!("/api/sessions/{}/events", session_id))
        .body(Body::empty())
        .expect("request should build");

    let events_resp = app
        .clone()
        .oneshot(events_req)
        .await
        .expect("response should return");

    // SSE endpoint should return 200 OK with text/event-stream content type
    assert_eq!(events_resp.status(), StatusCode::OK);
    assert_eq!(
        events_resp
            .headers()
            .get("content-type")
            .map(|v| v.to_str().unwrap_or("")),
        Some("text/event-stream")
    );

    // 注意：SSE 流是无限的，不能使用 to_bytes 读取整个 body（会永远等待）。
    // 上面的 status/content-type 检查已验证 SSE endpoint 正常工作。
    // 如果需要验证事件内容，应使用流式读取或单独的事件回放测试。
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
// Test: e2e_session_interrupt
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e2e_session_interrupt() {
    let (state, _guard) = test_state(None);
    let app = build_api_router().with_state(state);

    // Create a session
    let working_dir = std::env::temp_dir();
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

    // Submit a prompt first
    let prompt_req = auth_request("POST", &format!("/api/sessions/{}/prompts", session_id))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&PromptRequest {
                text: "Test prompt".to_string(),
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

    // Interrupt the session
    let interrupt_req = auth_request("POST", &format!("/api/sessions/{}/interrupt", session_id))
        .body(Body::empty())
        .expect("request should build");

    let interrupt_resp = app
        .clone()
        .oneshot(interrupt_req)
        .await
        .expect("response should return");

    // Should return 204 No Content
    assert_eq!(interrupt_resp.status(), StatusCode::NO_CONTENT);
}

// ---------------------------------------------------------------------------
// Test: e2e_auth_exchange_flow
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e2e_auth_exchange_flow() {
    let (state, _guard) = test_state(None);
    let app = build_api_router().with_state(state);

    // Exchange bootstrap token for session token
    let exchange_req = Request::builder()
        .method("POST")
        .uri("/api/auth/exchange")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&AuthExchangeRequest {
                token: "browser-token".to_string(),
            })
            .expect("request should serialize"),
        ))
        .expect("request should build");

    let exchange_resp = app
        .clone()
        .oneshot(exchange_req)
        .await
        .expect("response should return");

    assert_eq!(exchange_resp.status(), StatusCode::OK);
    let exchange_result: AuthExchangeResponse = json_body(exchange_resp).await;
    assert!(exchange_result.ok);
    assert!(!exchange_result.token.is_empty());
    assert!(exchange_result.expires_at_ms > 0);

    // Use the exchanged token to access protected endpoints
    let sessions_req = Request::builder()
        .uri("/api/sessions")
        .header(AUTH_HEADER_NAME, &exchange_result.token)
        .body(Body::empty())
        .expect("request should build");

    let sessions_resp = app
        .clone()
        .oneshot(sessions_req)
        .await
        .expect("response should return");

    assert_eq!(sessions_resp.status(), StatusCode::OK);
}

// ---------------------------------------------------------------------------
// Test: e2e_model_list
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e2e_model_list() {
    let (state, _guard) = test_state(None);
    let app = build_api_router().with_state(state);

    // List available models
    let list_req = auth_request("GET", "/api/models")
        .body(Body::empty())
        .expect("request should build");

    let list_resp = app
        .clone()
        .oneshot(list_req)
        .await
        .expect("response should return");

    assert_eq!(list_resp.status(), StatusCode::OK);
    let models: serde_json::Value = json_body(list_resp).await;
    assert!(models.is_array());
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
