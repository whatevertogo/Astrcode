use astrcode_core::StorageEventPayload;
use astrcode_protocol::http::{
    CompactSessionResponse, ConfigReloadResponse, PromptAcceptedResponse,
};
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use tower::ServiceExt;

use crate::{
    AUTH_HEADER_NAME,
    routes::build_api_router,
    test_support::{mark_session_running, stored_events_for_session, test_state},
};

async fn json_body<T: serde::de::DeserializeOwned>(response: axum::http::Response<Body>) -> T {
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body should be readable");
    serde_json::from_slice(&bytes).expect("response should deserialize")
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
    assert!(
        !state
            ._runtime_handles
            .session_runtime
            .list_running_sessions()
            .contains(&session.session_id.clone().into())
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
    assert!(
        !state
            ._runtime_handles
            .session_runtime
            .list_running_sessions()
            .contains(&session.session_id.clone().into())
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
    assert!(payload.deferred);
    let terminal_facts = state
        .app
        .terminal_snapshot_facts(&session.session_id)
        .await
        .expect("terminal facts should reflect pending compact");
    assert!(terminal_facts.control.manual_compact_pending);
    assert!(
        terminal_facts
            .slash_candidates
            .iter()
            .all(|candidate| candidate.id != "compact"),
        "pending compact should be observed through terminal discovery facts"
    );
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
async fn prompt_route_accepts_structured_skill_invocation() {
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
    let payload: PromptAcceptedResponse = json_body(response).await;
    assert!(!payload.turn_id.is_empty());
}

#[tokio::test]
async fn prompt_route_rejects_unknown_skill_invocation() {
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

    let events = stored_events_for_session(&state, &session.session_id).await;
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

#[tokio::test]
async fn compact_route_rejects_manual_compact_false() {
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
