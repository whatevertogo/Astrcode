use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use tower::ServiceExt;

use crate::{AUTH_HEADER_NAME, routes::build_api_router, test_support::test_state};

// Why: 这些契约测试是 API 接口稳定性的核心保障，
// 防止 server 在重构后回退到隐式容错或启发式行为。

// ─── Prompt 提交契约 ──────────────────────────────────────

#[tokio::test]
async fn submit_prompt_contract_returns_accepted_shape() {
    let (state, _guard) = test_state(None).await;
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let created = state
        .app
        .create_session(temp_dir.path().display().to_string())
        .await
        .expect("session should be created");
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{}/prompts", created.session_id))
                .header(AUTH_HEADER_NAME, "browser-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({"text": "hello"}))
                        .expect("request should serialize"),
                ))
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let payload: serde_json::Value = serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should be readable"),
    )
    .expect("payload should deserialize");
    assert_eq!(payload["sessionId"], created.session_id.to_string());
    assert!(!payload["turnId"].as_str().unwrap_or_default().is_empty());
}

// ─── SubRun 状态查询契约 ──────────────────────────────────

#[tokio::test]
async fn subrun_status_contract_returns_default_for_missing_subrun() {
    // Why: 新架构对无匹配 agent 的 session 返回默认 subrun 视图（source=Live, lifecycle=Idle），
    // 而非 404，保证前端总有可渲染的 subrun 状态。
    let (state, _guard) = test_state(None).await;
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let created = state
        .app
        .create_session(temp_dir.path().display().to_string())
        .await
        .expect("session should be created");
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v1/sessions/{}/subruns/missing-subrun",
                    created.session_id
                ))
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::OK);
    let payload: serde_json::Value = serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should be readable"),
    )
    .expect("payload should deserialize");
    assert_eq!(payload["subRunId"], "missing-subrun");
    // lifecycle 和 source 序列化为小写枚举值
    assert_eq!(
        payload["lifecycle"].as_str().unwrap().to_lowercase(),
        "idle"
    );
    assert_eq!(payload["source"].as_str().unwrap().to_lowercase(), "live");
}

#[tokio::test]
async fn subrun_cancel_contract_returns_not_found_for_missing_subrun() {
    let (state, _guard) = test_state(None).await;
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let created = state
        .app
        .create_session(temp_dir.path().display().to_string())
        .await
        .expect("session should be created");
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/v1/sessions/{}/subruns/missing-subrun/cancel",
                    created.session_id
                ))
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ─── Session ID 格式校验 ───────────────────────────────────

#[tokio::test]
async fn session_routes_reject_invalid_session_id_format() {
    let (state, _guard) = test_state(None).await;
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/sessions/bad.id/history")
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn session_history_contract_rejects_scope_without_subrun_id() {
    let (state, _guard) = test_state(None).await;
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let created = state
        .app
        .create_session(temp_dir.path().display().to_string())
        .await
        .expect("session should be created");
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/sessions/{}/history?scope=self",
                    created.session_id
                ))
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn session_events_contract_rejects_scope_without_subrun_id() {
    let (state, _guard) = test_state(None).await;
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let created = state
        .app
        .create_session(temp_dir.path().display().to_string())
        .await
        .expect("session should be created");
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/sessions/{}/events?scope=self",
                    created.session_id
                ))
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn subrun_status_contract_rejects_invalid_session_id_format() {
    let (state, _guard) = test_state(None).await;
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/sessions/bad.id/subruns/missing-subrun")
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn subrun_cancel_route_returns_not_found_after_removal() {
    let (state, _guard) = test_state(None).await;
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let created = state
        .app
        .create_session(temp_dir.path().display().to_string())
        .await
        .expect("session should be created");
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/v1/sessions/{}/subruns/bad.id/cancel",
                    created.session_id
                ))
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    // legacy cancel route 已删除，统一走 close
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ─── Close Agent Route 契约测试 ─────────────────────────────

#[tokio::test]
async fn close_agent_route_closes_target_agent_and_returns_closed_ids() {
    let (state, _guard) = test_state(None).await;
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let created = state
        .app
        .create_session(temp_dir.path().display().to_string())
        .await
        .expect("session should be created");

    let profile = astrcode_core::AgentProfile {
        id: "explore".to_string(),
        name: "Explore".to_string(),
        description: "explore agent".to_string(),
        mode: astrcode_core::AgentMode::SubAgent,
        system_prompt: None,
        allowed_tools: Vec::new(),
        disallowed_tools: Vec::new(),
        model_preference: None,
    };
    let handle = state
        .app
        .kernel()
        .agent_control()
        .spawn(
            &profile,
            &created.session_id.to_string(),
            "turn-parent".to_string(),
            None,
        )
        .await
        .expect("agent should spawn");

    let app = build_api_router().with_state(state);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/v1/sessions/{}/agents/{}/close",
                    created.session_id, handle.agent_id
                ))
                .header(AUTH_HEADER_NAME, "browser-token")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"cascade":true}"#))
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: serde_json::Value = serde_json::from_slice(&bytes).expect("should deserialize");
    let closed_ids = payload["closedAgentIds"]
        .as_array()
        .expect("closedAgentIds should be array");
    assert!(
        closed_ids
            .iter()
            .any(|id| id.as_str() == Some(&handle.agent_id)),
        "closed list should contain the target agent"
    );
}

#[tokio::test]
async fn close_agent_route_accepts_empty_json_body() {
    let (state, _guard) = test_state(None).await;
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let created = state
        .app
        .create_session(temp_dir.path().display().to_string())
        .await
        .expect("session should be created");

    let profile = astrcode_core::AgentProfile {
        id: "explore".to_string(),
        name: "Explore".to_string(),
        description: "explore agent".to_string(),
        mode: astrcode_core::AgentMode::SubAgent,
        system_prompt: None,
        allowed_tools: Vec::new(),
        disallowed_tools: Vec::new(),
        model_preference: None,
    };
    let handle = state
        .app
        .kernel()
        .agent_control()
        .spawn(
            &profile,
            &created.session_id.to_string(),
            "turn-parent".to_string(),
            None,
        )
        .await
        .expect("agent should spawn");

    let app = build_api_router().with_state(state);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/v1/sessions/{}/agents/{}/close",
                    created.session_id, handle.agent_id
                ))
                .header(AUTH_HEADER_NAME, "browser-token")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    // close 接口不读取 body 字段，空 JSON body 也应保持同一契约行为。
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn close_agent_route_returns_not_found_for_unknown_agent() {
    // Why: 新架构对未知 agent 显式返回 NotFound，不做隐式幂等处理。
    // 调用方应通过 agent 树查询确认存在后再 close。
    let (state, _guard) = test_state(None).await;
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let created = state
        .app
        .create_session(temp_dir.path().display().to_string())
        .await
        .expect("session should be created");
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/v1/sessions/{}/agents/nonexistent-agent/close",
                    created.session_id
                ))
                .header(AUTH_HEADER_NAME, "browser-token")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"cascade":true}"#))
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "close for unknown agent should return 404"
    );
}

// ─── Scope 参数过滤契约 ─────────────────────────────────────

#[tokio::test]
async fn scope_parameter_without_subrun_id_is_rejected() {
    // Why: scope 参数只有在提供 subRunId 时才有意义
    let (state, _guard) = test_state(None).await;
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let created = state
        .app
        .create_session(temp_dir.path().display().to_string())
        .await
        .expect("session should be created");
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/sessions/{}/history?scope=directChildren",
                    created.session_id
                ))
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert!(
        response.status().is_client_error(),
        "scope without subRunId should be rejected, got: {}",
        response.status()
    );
}
