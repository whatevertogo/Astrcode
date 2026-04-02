use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::extract::State;
use axum::http::{Method, Request, StatusCode};
use axum::routing::get;
use axum::Router;
use tempfile::TempDir;
use tower::ServiceExt;

use astrcode_runtime::config::{DEEPSEEK_API_KEY_ENV, ENV_REFERENCE_PREFIX};

use crate::auth::secure_token_eq;
use crate::bootstrap::{build_cors_layer, inject_browser_bootstrap_html, serve_frontend_build};
use crate::mapper::api_key_preview;
use crate::routes::sessions::session_messages;
use crate::test_support::test_state;
use crate::{FrontendBuild, AUTH_HEADER_NAME, SESSION_CURSOR_HEADER_NAME};

#[test]
fn injects_browser_bootstrap_into_head() {
    let html = inject_browser_bootstrap_html(
        "<!doctype html><html><head><title>AstrCode</title></head><body><div id=\"root\"></div></body></html>",
        "http://127.0.0.1:62000",
        "browser-token",
    )
    .expect("bootstrap injection should succeed");

    assert!(html.contains("window.__ASTRCODE_BOOTSTRAP__"));
    assert!(html.contains("\"token\":\"browser-token\""));
    assert!(html.contains("\"serverOrigin\":\"http://127.0.0.1:62000\""));
    assert!(
        html.find("window.__ASTRCODE_BOOTSTRAP__")
            .expect("html should contain bootstrap script")
            < html.find("</head>").expect("html should contain head")
    );
}

#[tokio::test]
async fn serves_bootstrapped_index_for_spa_routes() {
    let temp_dir = TempDir::new().expect("temp dir should be creatable");
    std::fs::write(
        temp_dir.path().join("index.html"),
        "<!doctype html><html><head><title>AstrCode</title></head><body><div id=\"root\"></div></body></html>",
    )
    .expect("index.html should be writable");
    std::fs::create_dir_all(temp_dir.path().join("assets"))
        .expect("assets dir should be creatable");
    std::fs::write(
        temp_dir.path().join("assets").join("app.js"),
        "console.log('ok');",
    )
    .expect("asset file should be writable");

    let frontend_build = FrontendBuild {
        dist_dir: temp_dir.path().to_path_buf(),
        index_html: Arc::new(
            inject_browser_bootstrap_html(
                "<!doctype html><html><head><title>AstrCode</title></head><body><div id=\"root\"></div></body></html>",
                "http://127.0.0.1:65000",
                "browser-token",
            )
            .expect("bootstrap injection should succeed"),
        ),
    };
    let (state, _guard) = test_state(Some(frontend_build));

    let root = serve_frontend_build(
        State(state.clone()),
        Request::builder()
            .uri("/")
            .body(Body::empty())
            .expect("root request should be valid"),
    )
    .await;
    assert_eq!(root.status(), StatusCode::OK);
    let root_body = to_bytes(root.into_body(), usize::MAX)
        .await
        .expect("root response body should be readable");
    let root_body = String::from_utf8(root_body.to_vec()).expect("root body should be utf8");
    assert!(root_body.contains("window.__ASTRCODE_BOOTSTRAP__"));
    assert!(root_body.contains("<div id=\"root\"></div>"));

    let spa = serve_frontend_build(
        State(state.clone()),
        Request::builder()
            .uri("/projects/demo")
            .body(Body::empty())
            .expect("spa request should be valid"),
    )
    .await;
    assert_eq!(spa.status(), StatusCode::OK);
    let spa_body = to_bytes(spa.into_body(), usize::MAX)
        .await
        .expect("spa response body should be readable");
    let spa_body = String::from_utf8(spa_body.to_vec()).expect("spa body should be utf8");
    assert!(spa_body.contains("window.__ASTRCODE_BOOTSTRAP__"));

    let asset = serve_frontend_build(
        State(state.clone()),
        Request::builder()
            .uri("/assets/app.js")
            .body(Body::empty())
            .expect("asset request should be valid"),
    )
    .await;
    assert_eq!(asset.status(), StatusCode::OK);
    let asset_body = to_bytes(asset.into_body(), usize::MAX)
        .await
        .expect("asset response body should be readable");
    assert_eq!(asset_body.as_ref(), b"console.log('ok');");

    let missing_asset = serve_frontend_build(
        State(state),
        Request::builder()
            .uri("/assets/missing.js")
            .body(Body::empty())
            .expect("missing asset request should be valid"),
    )
    .await;
    assert_eq!(missing_asset.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn cors_preflight_allows_cache_control_for_sse_requests() {
    let app = Router::new()
        .route(
            "/api/sessions/demo/events",
            get(|| async { StatusCode::OK }),
        )
        .layer(build_cors_layer());

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::OPTIONS)
                .uri("/api/sessions/demo/events")
                .header("origin", "http://127.0.0.1:5173")
                .header("access-control-request-method", "GET")
                .header(
                    "access-control-request-headers",
                    "x-astrcode-token,cache-control",
                )
                .body(Body::empty())
                .expect("preflight request should be valid"),
        )
        .await
        .expect("preflight response should be returned");

    assert!(response.status().is_success());
    let allowed_headers = response
        .headers()
        .get("access-control-allow-headers")
        .and_then(|value| value.to_str().ok())
        .expect("cors preflight should expose allowed headers")
        .to_ascii_lowercase();
    assert!(allowed_headers.contains(AUTH_HEADER_NAME));
    assert!(allowed_headers.contains("cache-control"));
}

#[tokio::test]
async fn session_messages_exposes_cursor_header_to_cross_origin_clients() {
    let temp_dir = TempDir::new().expect("temp dir should be creatable");
    let (state, _guard) = test_state(None);
    let meta = state
        .service
        .create_session(temp_dir.path())
        .await
        .expect("session should be created");
    let app = Router::new()
        .route("/api/sessions/:id/messages", get(session_messages))
        .with_state(state)
        .layer(build_cors_layer());

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/api/sessions/{}/messages", meta.session_id))
                .header("origin", "http://127.0.0.1:5173")
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("messages request should be valid"),
        )
        .await
        .expect("messages response should be returned");

    assert_eq!(response.status(), StatusCode::OK);
    let cursor = response
        .headers()
        .get(SESSION_CURSOR_HEADER_NAME)
        .and_then(|value| value.to_str().ok())
        .expect("messages response should include cursor header");
    assert!(!cursor.is_empty());
    let exposed_headers = response
        .headers()
        .get("access-control-expose-headers")
        .and_then(|value| value.to_str().ok())
        .expect("cross-origin response should expose cursor header")
        .to_ascii_lowercase();
    assert!(exposed_headers.contains(SESSION_CURSOR_HEADER_NAME));
}

#[tokio::test]
async fn session_messages_requires_authentication() {
    let temp_dir = TempDir::new().expect("temp dir should be creatable");
    let (state, _guard) = test_state(None);
    let meta = state
        .service
        .create_session(temp_dir.path())
        .await
        .expect("session should be created");
    let app = Router::new()
        .route("/api/sessions/:id/messages", get(session_messages))
        .with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/api/sessions/{}/messages", meta.session_id))
                .body(Body::empty())
                .expect("messages request should be valid"),
        )
        .await
        .expect("messages response should be returned");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn session_messages_returns_not_found_for_unknown_session() {
    let (state, _guard) = test_state(None);
    let app = Router::new()
        .route("/api/sessions/:id/messages", get(session_messages))
        .with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/sessions/missing-session/messages")
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("messages request should be valid"),
        )
        .await
        .expect("messages response should be returned");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[test]
fn secure_token_eq_requires_exact_match() {
    assert!(secure_token_eq("browser-token", "browser-token"));
    assert!(!secure_token_eq("browser-token", "browser-token-x"));
    assert!(!secure_token_eq("browser-token", "browser-tokem"));
}

#[test]
fn api_key_preview_supports_explicit_env_and_literal_prefixes() {
    assert_eq!(
        api_key_preview(Some(&format!(
            "{}{}",
            ENV_REFERENCE_PREFIX, DEEPSEEK_API_KEY_ENV
        ))),
        format!("环境变量: {}", DEEPSEEK_API_KEY_ENV)
    );
    assert_eq!(api_key_preview(Some("literal:ABCD1234")), "****1234");
}
