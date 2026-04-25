use astrcode_core::{Phase, SessionId, StorageEventPayload, UserMessageOrigin};
use astrcode_protocol::http::{CompactSessionResponse, ConfigReloadResponse, PromptSubmitResponse};
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use tower::ServiceExt;

use crate::{
    AUTH_HEADER_NAME,
    routes::build_api_router,
    test_support::{mark_session_running, test_state},
};

async fn json_body<T: serde::de::DeserializeOwned>(response: axum::http::Response<Body>) -> T {
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body should be readable");
    serde_json::from_slice(&bytes).expect("response should deserialize")
}

async fn submit_prompt_request(
    state: &crate::AppState,
    session_id: &str,
    request: serde_json::Value,
) -> axum::http::Response<Body> {
    build_api_router()
        .with_state(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{session_id}/prompts"))
                .header(AUTH_HEADER_NAME, "browser-token")
                .header("content-type", "application/json")
                .body(Body::from(request.to_string()))
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned")
}

#[tokio::test]
async fn config_reload_returns_runtime_status_when_idle() {
    let (state, _guard) = test_state(None).await;
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
    let payload: ConfigReloadResponse = json_body(response).await;
    assert!(!payload.reloaded_at.is_empty());
    assert_eq!(payload.status.runtime_name, "astrcode-server");
}

#[tokio::test]
async fn config_reload_rejects_when_session_is_running() {
    let (state, _guard) = test_state(None).await;
    let session = state
        .session_catalog
        .create_session(
            tempfile::tempdir()
                .expect("tempdir")
                .path()
                .display()
                .to_string(),
        )
        .await
        .expect("session should be created");
    assert!(
        !state
            ._runtime_handles
            ._session_runtime_test_support
            .list_running_session_ids()
            .contains(&session.session_id)
    );
    mark_session_running(&state, &session.session_id).await;
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
async fn compact_route_accepts_immediately_when_only_previous_busy_flag_is_set() {
    let (state, _guard) = test_state(None).await;
    let session = state
        .session_catalog
        .create_session(
            tempfile::tempdir()
                .expect("tempdir")
                .path()
                .display()
                .to_string(),
        )
        .await
        .expect("session should be created");
    assert!(
        !state
            ._runtime_handles
            ._session_runtime_test_support
            .list_running_session_ids()
            .contains(&session.session_id)
    );
    mark_session_running(&state, &session.session_id).await;
    let app = build_api_router().with_state(state.clone());

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
    assert!(
        !payload.deferred,
        "session runtime busy state alone should not defer host-session compaction"
    );
}

#[tokio::test]
async fn prompt_route_accepts_structured_skill_invocation() {
    let (state, _guard) = test_state(None).await;
    let session = state
        .session_catalog
        .create_session(
            tempfile::tempdir()
                .expect("tempdir")
                .path()
                .display()
                .to_string(),
        )
        .await
        .expect("session should be created");
    let app = build_api_router().with_state(state.clone());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{}/prompts", session.session_id))
                .header(AUTH_HEADER_NAME, "browser-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "text": "提交当前修改",
                        "skillInvocation": {
                            "skillId": "git-commit",
                            "userPrompt": "提交当前修改"
                        }
                    })
                    .to_string(),
                ))
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let payload: PromptSubmitResponse = json_body(response).await;
    match payload {
        PromptSubmitResponse::Accepted { turn_id, .. } => assert!(!turn_id.is_empty()),
        PromptSubmitResponse::Handled { .. } => panic!("prompt should create a turn"),
    }
}

#[tokio::test]
async fn prompt_route_rejects_unknown_skill_invocation() {
    let (state, _guard) = test_state(None).await;
    let session = state
        .session_catalog
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
                        "text": "",
                        "skillInvocation": {
                            "skillId": "missing-skill"
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

#[tokio::test]
async fn prompt_submission_starts_root_runtime() {
    let (state, guard) = test_state(None).await;
    let session = state
        .session_catalog
        .create_session(
            tempfile::tempdir()
                .expect("tempdir")
                .path()
                .display()
                .to_string(),
        )
        .await
        .expect("session should be created");

    let response = submit_prompt_request(
        &state,
        &session.session_id,
        serde_json::json!({ "text": "hello" }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    let root_status = guard
        .agent_control()
        .query_root_status(&session.session_id)
        .await;
    assert!(
        root_status.is_some(),
        "prompt route should materialize the root agent and start the runtime path"
    );
    let stored = state
        .session_catalog
        .replay_stored_events(&SessionId::from(session.session_id.clone()))
        .await
        .expect("stored events should replay");
    assert!(
        stored.iter().any(|stored| matches!(
            &stored.event.payload,
            StorageEventPayload::UserMessage { content, origin, .. }
                if content == "hello" && matches!(origin, UserMessageOrigin::User)
        )),
        "prompt route should persist user input before the runtime finishes"
    );
    let control = state
        .session_catalog
        .session_control_state(&SessionId::from(session.session_id.clone()))
        .await
        .expect("control state should read");
    assert_eq!(control.phase, Phase::Thinking);
    assert!(
        control.active_turn_id.is_some(),
        "accepted prompt should keep input locked while the turn is active"
    );
}

#[tokio::test]
async fn compact_route_rejects_manual_compact_false() {
    let (state, _guard) = test_state(None).await;
    let session = state
        .session_catalog
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
                .uri(format!("/api/sessions/{}/compact", session.session_id))
                .header(AUTH_HEADER_NAME, "browser-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "control": {
                            "manualCompact": false
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
