use std::sync::atomic::Ordering;

use astrcode_core::{SessionId, StorageEventPayload};
use astrcode_protocol::http::{
    CompactSessionResponse, ConfigReloadResponse, PromptAcceptedResponse,
};
#[cfg(feature = "debug-workbench")]
use astrcode_protocol::http::{
    RuntimeDebugOverviewDto, RuntimeDebugTimelineDto, SessionDebugAgentsDto, SessionDebugTraceDto,
};
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use tower::ServiceExt;

use crate::{AUTH_HEADER_NAME, routes::build_api_router, test_support::test_state};

async fn json_body<T: serde::de::DeserializeOwned>(response: axum::http::Response<Body>) -> T {
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body should be readable");
    serde_json::from_slice(&bytes).expect("response should deserialize")
}

#[tokio::test]
async fn config_reload_returns_runtime_status_when_idle() {
    let (state, _guard) = test_state(None).await;
    let app = build_api_router().with_state(state);

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
    let payload: ConfigReloadResponse = json_body(response).await;
    assert!(!payload.reloaded_at.is_empty());
    assert_eq!(payload.status.runtime_name, "astrcode-application");
}

#[cfg(feature = "debug-workbench")]
#[tokio::test]
async fn debug_runtime_overview_route_returns_workbench_overview() {
    let (state, _guard) = test_state(None).await;
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/debug/runtime/overview")
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::OK);
    let payload: RuntimeDebugOverviewDto = json_body(response).await;
    assert!(!payload.collected_at.is_empty());
}

#[cfg(feature = "debug-workbench")]
#[tokio::test]
async fn debug_runtime_timeline_route_returns_server_window_samples() {
    let (state, _guard) = test_state(None).await;
    let app = build_api_router().with_state(state.clone());
    let _ = state.debug_workbench.runtime_overview();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/debug/runtime/timeline")
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::OK);
    let payload: RuntimeDebugTimelineDto = json_body(response).await;
    assert!(
        !payload.samples.is_empty(),
        "timeline should contain at least the freshly recorded overview sample"
    );
}

#[cfg(feature = "debug-workbench")]
#[tokio::test]
async fn debug_session_trace_route_is_scoped_to_requested_session() {
    let (state, _guard) = test_state(None).await;
    let session_a = state
        .app
        .create_session(
            tempfile::tempdir()
                .expect("tempdir")
                .path()
                .display()
                .to_string(),
        )
        .await
        .expect("session should be created");
    let session_b = state
        .app
        .create_session(
            tempfile::tempdir()
                .expect("tempdir")
                .path()
                .display()
                .to_string(),
        )
        .await
        .expect("session should be created");
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/api/debug/sessions/{}/trace",
                    session_a.session_id
                ))
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::OK);
    let payload: SessionDebugTraceDto = json_body(response).await;
    assert_eq!(payload.session_id, session_a.session_id);
    assert_ne!(payload.session_id, session_b.session_id);
}

#[cfg(feature = "debug-workbench")]
#[tokio::test]
async fn debug_session_agents_route_is_scoped_to_requested_session() {
    let (state, _guard) = test_state(None).await;
    let session_a = state
        .app
        .create_session(
            tempfile::tempdir()
                .expect("tempdir")
                .path()
                .display()
                .to_string(),
        )
        .await
        .expect("session should be created");
    let session_b = state
        .app
        .create_session(
            tempfile::tempdir()
                .expect("tempdir")
                .path()
                .display()
                .to_string(),
        )
        .await
        .expect("session should be created");
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/api/debug/sessions/{}/agents",
                    session_a.session_id
                ))
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::OK);
    let payload: SessionDebugAgentsDto = json_body(response).await;
    assert_eq!(payload.session_id, session_a.session_id);
    assert_ne!(payload.session_id, session_b.session_id);
    assert_eq!(
        payload.nodes.first().map(|node| node.session_id.as_str()),
        Some(session_a.session_id.as_str())
    );
}

#[cfg(feature = "debug-workbench")]
#[tokio::test]
async fn debug_session_trace_route_returns_not_found_for_unknown_session() {
    let (state, _guard) = test_state(None).await;
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/debug/sessions/unknown-session/trace")
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn config_reload_rejects_when_session_is_running() {
    let (state, _guard) = test_state(None).await;
    let session = state
        .app
        .create_session(
            tempfile::tempdir()
                .expect("tempdir")
                .path()
                .display()
                .to_string(),
        )
        .await
        .expect("session should be created");
    let session_state = state
        .app
        .session_runtime()
        .get_session_state(&session.session_id.clone().into())
        .await
        .expect("session state should load");
    session_state.running.store(true, Ordering::SeqCst);
    let app = build_api_router().with_state(state);

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

    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn compact_route_defers_when_session_is_busy() {
    let (state, _guard) = test_state(None).await;
    let session = state
        .app
        .create_session(
            tempfile::tempdir()
                .expect("tempdir")
                .path()
                .display()
                .to_string(),
        )
        .await
        .expect("session should be created");
    let session_state = state
        .app
        .session_runtime()
        .get_session_state(&session.session_id.clone().into())
        .await
        .expect("session state should load");
    session_state.running.store(true, Ordering::SeqCst);
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{}/compact", session.session_id))
                .header(AUTH_HEADER_NAME, "browser-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "control": {
                            "manualCompact": true
                        }
                    })
                    .to_string(),
                ))
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let payload: CompactSessionResponse = json_body(response).await;
    assert!(payload.accepted);
    assert!(payload.deferred);
}

#[tokio::test]
async fn prompt_route_roundtrips_accepted_execution_control() {
    let (state, _guard) = test_state(None).await;
    let session = state
        .app
        .create_session(
            tempfile::tempdir()
                .expect("tempdir")
                .path()
                .display()
                .to_string(),
        )
        .await
        .expect("session should be created");
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{}/prompts", session.session_id))
                .header(AUTH_HEADER_NAME, "browser-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "text": "hello",
                        "control": {
                            "maxSteps": 7
                        }
                    })
                    .to_string(),
                ))
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let payload: PromptAcceptedResponse = json_body(response).await;
    let accepted_control = payload
        .accepted_control
        .expect("accepted control should be returned");
    assert_eq!(accepted_control.max_steps, Some(7));
    assert_eq!(accepted_control.manual_compact, None);
}

#[tokio::test]
async fn prompt_submission_registers_session_root_agent_context() {
    let (state, _guard) = test_state(None).await;
    let session = state
        .app
        .create_session(
            tempfile::tempdir()
                .expect("tempdir")
                .path()
                .display()
                .to_string(),
        )
        .await
        .expect("session should be created");

    state
        .app
        .submit_prompt(&session.session_id, "hello".to_string())
        .await
        .expect("prompt should be accepted");

    let root_status = state
        .app
        .get_root_agent_status(&session.session_id)
        .await
        .expect("root status query should succeed")
        .expect("ordinary prompt session should register an implicit root agent");
    assert!(
        root_status.agent_id.starts_with("root-agent:"),
        "implicit root agent id should be session-scoped: {}",
        root_status.agent_id
    );
    assert_eq!(root_status.agent_profile, "default");

    let events = state
        .app
        .session_runtime()
        .replay_stored_events(&SessionId::from(session.session_id.clone()))
        .await
        .expect("events should replay");
    let user_message = events
        .into_iter()
        .find(|stored| {
            matches!(
                stored.event.payload,
                StorageEventPayload::UserMessage { .. }
            )
        })
        .expect("user message event should exist");
    assert_eq!(
        user_message.event.agent.agent_id.as_deref(),
        Some(root_status.agent_id.as_str())
    );
    assert_eq!(
        user_message.event.agent.agent_profile.as_deref(),
        Some("default")
    );
}

#[tokio::test]
async fn prompt_route_rejects_invalid_execution_control() {
    let (state, _guard) = test_state(None).await;
    let session = state
        .app
        .create_session(
            tempfile::tempdir()
                .expect("tempdir")
                .path()
                .display()
                .to_string(),
        )
        .await
        .expect("session should be created");
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{}/prompts", session.session_id))
                .header(AUTH_HEADER_NAME, "browser-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "text": "hello",
                        "control": {
                            "maxSteps": 0
                        }
                    })
                    .to_string(),
                ))
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
