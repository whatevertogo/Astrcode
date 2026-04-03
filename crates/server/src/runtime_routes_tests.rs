use axum::body::to_bytes;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use astrcode_core::{
    plugin::PluginEntry, CapabilityDescriptor, CapabilityKind, PluginHealth, PluginState,
    SideEffectLevel, StabilityLevel,
};
use astrcode_protocol::http::RuntimeStatusDto;
use serde_json::json;

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
async fn runtime_status_exposes_plugin_warnings() {
    let (state, _guard) = test_state(None);
    state
        .coordinator
        .plugin_registry()
        .replace_snapshot(vec![PluginEntry {
            manifest: astrcode_core::PluginManifest {
                name: "demo-plugin".to_string(),
                version: "0.1.0".to_string(),
                description: "demo".to_string(),
                plugin_type: vec![astrcode_core::PluginType::Tool],
                capabilities: Vec::new(),
                executable: Some("demo.exe".to_string()),
                args: Vec::new(),
                working_dir: None,
                repository: None,
            },
            state: PluginState::Initialized,
            health: PluginHealth::Healthy,
            failure_count: 0,
            capabilities: vec![CapabilityDescriptor {
                name: "demo.search".to_string(),
                kind: CapabilityKind::tool(),
                description: "search".to_string(),
                input_schema: json!({"type":"object"}),
                output_schema: json!({"type":"object"}),
                streaming: false,
                concurrency_safe: false,
                compact_clearable: false,
                profiles: vec!["coding".to_string()],
                tags: Vec::new(),
                permissions: Vec::new(),
                side_effect: SideEffectLevel::None,
                stability: StabilityLevel::Stable,
                metadata: json!(null),
            }],
            failure: None,
            warnings: vec![
                "skill 'repo-search' dropped unknown allowed tool 'missing.tool'".to_string(),
            ],
            last_checked_at: None,
        }]);
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
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: RuntimeStatusDto =
        serde_json::from_slice(&bytes).expect("runtime status should deserialize");
    assert_eq!(payload.plugins.len(), 1);
    assert!(payload.plugins[0]
        .warnings
        .iter()
        .any(|warning| warning.contains("missing.tool")));
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
