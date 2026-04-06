use astrcode_protocol::http::{ComposerOptionKindDto, ComposerOptionsResponseDto};
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
async fn composer_options_require_authentication() {
    let (state, _guard) = test_state(None);
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let session = state
        .service
        .create_session(temp_dir.path())
        .await
        .expect("session should be created");
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/sessions/{}/composer/options",
                    session.session_id
                ))
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn composer_options_expose_session_scoped_skill_entries() {
    let (state, _guard) = test_state(None);
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let session = state
        .service
        .create_session(temp_dir.path())
        .await
        .expect("session should be created");
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/sessions/{}/composer/options?kinds=skill&q=git",
                    session.session_id
                ))
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::OK);
    let payload: ComposerOptionsResponseDto = json_body(response).await;
    assert!(!payload.items.is_empty());
    assert!(
        payload
            .items
            .iter()
            .all(|item| item.kind == ComposerOptionKindDto::Skill)
    );
    assert!(payload.items.iter().any(|item| item.id == "git-commit"));
}

#[tokio::test]
async fn composer_options_expose_runtime_command_entries() {
    let (state, _guard) = test_state(None);
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let session = state
        .service
        .create_session(temp_dir.path())
        .await
        .expect("session should be created");
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/sessions/{}/composer/options?kinds=command&q=comp",
                    session.session_id
                ))
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::OK);
    let payload: ComposerOptionsResponseDto = json_body(response).await;
    assert_eq!(payload.items.len(), 1);
    assert_eq!(payload.items[0].kind, ComposerOptionKindDto::Command);
    assert_eq!(payload.items[0].id, "compact");
    assert_eq!(payload.items[0].insert_text, "/compact");
}

#[tokio::test]
async fn composer_options_reject_unknown_kind_filters() {
    let (state, _guard) = test_state(None);
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let session = state
        .service
        .create_session(temp_dir.path())
        .await
        .expect("session should be created");
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/sessions/{}/composer/options?kinds=skill,unknown",
                    session.session_id
                ))
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
