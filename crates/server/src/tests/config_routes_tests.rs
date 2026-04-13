use std::sync::atomic::Ordering;

use astrcode_protocol::http::{
    CompactSessionResponse, ConfigReloadResponse, PromptAcceptedResponse,
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
                            "tokenBudget": 256,
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
    assert_eq!(accepted_control.token_budget, Some(256));
    assert_eq!(accepted_control.max_steps, Some(7));
    assert_eq!(accepted_control.manual_compact, None);
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
                            "tokenBudget": 0
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
