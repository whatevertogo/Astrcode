use astrcode_protocol::http::{AuthExchangeRequest, AuthExchangeResponse};
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use crate::auth::BootstrapAuth;
use crate::routes::build_api_router;
use crate::test_support::test_state;

#[tokio::test]
async fn exchange_auth_issues_session_token_for_valid_bootstrap() {
    let (state, _guard) = test_state(None);
    let auth_sessions = state.auth_sessions.clone();
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/exchange")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&AuthExchangeRequest {
                        token: "browser-token".to_string(),
                    })
                    .expect("request json should serialize"),
                ))
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::OK);
    let payload: AuthExchangeResponse = serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body should be readable"),
    )
    .expect("response json should deserialize");
    assert!(payload.ok);
    assert!(!payload.token.is_empty());
    assert!(auth_sessions.validate(&payload.token));
}

#[tokio::test]
async fn exchange_auth_rejects_expired_bootstrap_token() {
    let (mut state, _guard) = test_state(None);
    state.bootstrap_auth = BootstrapAuth::new(
        "browser-token".to_string(),
        chrono::Utc::now().timestamp_millis() - 1,
    );
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/exchange")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&AuthExchangeRequest {
                        token: "browser-token".to_string(),
                    })
                    .expect("request json should serialize"),
                ))
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
