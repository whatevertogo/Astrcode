use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use crate::routes::build_api_router;
use crate::test_support::test_state;
use crate::AUTH_HEADER_NAME;

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
