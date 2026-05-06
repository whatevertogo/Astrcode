use astrcode_core::{AgentEventContext, CancelToken, SpawnAgentParams, ToolContext};
use axum::{
    body::{Body, to_bytes},
    http::{Request, Response, StatusCode},
};
use tower::ServiceExt;

use crate::{
    AUTH_HEADER_NAME,
    routes::build_api_router,
    test_support::{
        ServerTestContext, seed_completed_root_turn, seed_unfinished_root_turn, test_state,
    },
};

// Why: 这些契约测试是 API 接口稳定性的核心保障，
// 防止 server 在重构后回退到隐式容错或启发式行为。

async fn submit_prompt_request(
    state: &crate::AppState,
    session_id: &str,
    request: serde_json::Value,
) -> Response<Body> {
    build_api_router()
        .with_state(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{session_id}/prompts"))
                .header(AUTH_HEADER_NAME, "browser-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&request).expect("request should serialize"),
                ))
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned")
}

async fn spawn_test_child_agent(
    ctx: &ServerTestContext,
    session_id: &str,
    working_dir: &std::path::Path,
) -> String {
    ctx.agent_control()
        .register_root_agent(
            "root-agent".to_string(),
            session_id.to_string(),
            "root-profile".to_string(),
        )
        .await
        .expect("root agent should be registered");

    let tool_ctx = ToolContext::new(
        session_id.to_string().into(),
        working_dir.to_path_buf(),
        CancelToken::new(),
    )
    .with_turn_id("turn-parent")
    .with_agent_context(AgentEventContext::root_execution(
        "root-agent",
        "root-profile",
    ));

    ctx.subagent_executor()
        .launch(
            SpawnAgentParams {
                r#type: Some("explore".to_string()),
                description: "explore agent".to_string(),
                prompt: "请阅读代码".to_string(),
                context: None,
            },
            &tool_ctx,
        )
        .await
        .expect("agent should launch")
        .handoff()
        .and_then(|handoff| {
            handoff
                .artifacts
                .iter()
                .find(|artifact| artifact.kind == "agent")
                .map(|artifact| artifact.id.clone())
        })
        .expect("spawned child should return agent artifact")
}

// ─── Prompt 提交契约 ──────────────────────────────────────

#[tokio::test]
async fn submit_prompt_contract_returns_accepted_shape() {
    let (state, _guard) = test_state(None).await;
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let created = state
        .session_catalog
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
    assert_eq!(payload["status"], "accepted");
    assert_eq!(payload["sessionId"], created.session_id.to_string());
    assert!(!payload["turnId"].as_str().unwrap_or_default().is_empty());
}

#[tokio::test]
async fn fork_session_contract_returns_new_session_meta() {
    let (state, _guard) = test_state(None).await;
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let created = state
        .session_catalog
        .create_session(temp_dir.path().display().to_string())
        .await
        .expect("session should be created");
    let submit_response = submit_prompt_request(
        &state,
        &created.session_id,
        serde_json::json!({ "text": "hello" }),
    )
    .await;
    assert_eq!(submit_response.status(), StatusCode::ACCEPTED);
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{}/fork", created.session_id))
                .header(AUTH_HEADER_NAME, "browser-token")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
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
    assert_eq!(payload["parentSessionId"], created.session_id);
    assert_ne!(payload["sessionId"], created.session_id);
}

#[tokio::test]
async fn fork_session_contract_accepts_completed_turn_id() {
    let (state, _guard) = test_state(None).await;
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let created = state
        .session_catalog
        .create_session(temp_dir.path().display().to_string())
        .await
        .expect("session should be created");
    seed_completed_root_turn(&state, &created.session_id, "turn-completed").await;
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{}/fork", created.session_id))
                .header(AUTH_HEADER_NAME, "browser-token")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"turnId":"turn-completed"}"#))
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
    assert_eq!(payload["parentSessionId"], created.session_id);
    assert_eq!(payload["parentStorageSeq"], 4);
    assert_ne!(payload["sessionId"], created.session_id);
}

#[tokio::test]
async fn fork_session_contract_rejects_unfinished_turn_id() {
    let (state, _guard) = test_state(None).await;
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let created = state
        .session_catalog
        .create_session(temp_dir.path().display().to_string())
        .await
        .expect("session should be created");
    seed_unfinished_root_turn(&state, &created.session_id, "turn-running").await;
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{}/fork", created.session_id))
                .header(AUTH_HEADER_NAME, "browser-token")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"turnId":"turn-running"}"#))
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload: serde_json::Value = serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should be readable"),
    )
    .expect("payload should deserialize");
    assert!(
        payload["error"]
            .as_str()
            .unwrap_or_default()
            .contains("has not completed"),
        "unfinished turn should return a specific validation error"
    );
}

#[tokio::test]
async fn fork_session_contract_rejects_mutually_exclusive_request() {
    let (state, _guard) = test_state(None).await;
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let created = state
        .session_catalog
        .create_session(temp_dir.path().display().to_string())
        .await
        .expect("session should be created");
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{}/fork", created.session_id))
                .header(AUTH_HEADER_NAME, "browser-token")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"turnId":"turn-1","storageSeq":42}"#))
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn fork_session_contract_returns_not_found_for_missing_session() {
    let (state, _guard) = test_state(None).await;
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions/nonexistent/fork")
                .header(AUTH_HEADER_NAME, "browser-token")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_project_contract_requires_valid_working_dir() {
    let (state, _guard) = test_state(None).await;
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/projects?workingDir=./definitely-missing-dir")
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn delete_project_contract_deletes_sessions_for_canonical_working_dir() {
    let (state, _guard) = test_state(None).await;
    let project = tempfile::tempdir().expect("tempdir should be created");
    let alias = project.path().join(".");
    let working_dir_query = alias.display().to_string().replace('\\', "/");
    let created = state
        .session_catalog
        .create_session(alias.display().to_string())
        .await
        .expect("session should be created");
    let app = build_api_router().with_state(state.clone());

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/projects?workingDir={working_dir_query}"))
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::OK);
    let list = state
        .session_catalog
        .list_session_metas()
        .await
        .expect("sessions should list");
    assert!(
        list.into_iter()
            .all(|meta| meta.session_id != created.session_id),
        "project delete should remove sessions that belong to the canonical working dir"
    );
}

// ─── SubRun 状态查询契约 ──────────────────────────────────

#[tokio::test]
async fn subrun_status_contract_returns_default_for_missing_subrun() {
    // Why: 新架构对无匹配 agent 的 session 返回默认 subrun 视图（source=Live, lifecycle=Idle），
    // 而非 404，保证前端总有可渲染的 subrun 状态。
    let (state, _guard) = test_state(None).await;
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let created = state
        .session_catalog
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
        payload["lifecycle"]
            .as_str()
            .expect("lifecycle should be a string")
            .to_lowercase(),
        "idle"
    );
    assert_eq!(
        payload["source"]
            .as_str()
            .expect("source should be a string")
            .to_lowercase(),
        "live"
    );
}

#[tokio::test]
async fn subrun_cancel_contract_returns_not_found_for_missing_subrun() {
    let (state, _guard) = test_state(None).await;
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let created = state
        .session_catalog
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
                .uri("/api/v1/conversation/sessions/bad.id/snapshot")
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn conversation_snapshot_contract_rejects_invalid_focus() {
    let (state, _guard) = test_state(None).await;
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let created = state
        .session_catalog
        .create_session(temp_dir.path().display().to_string())
        .await
        .expect("session should be created");
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v1/conversation/sessions/{}/snapshot?focus=bad-focus",
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
        .session_catalog
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

    // cancel route 已删除，统一走 close。
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ─── Close Agent Route 契约测试 ─────────────────────────────

#[tokio::test]
async fn close_agent_route_closes_target_agent_and_returns_closed_ids() {
    let (state, guard) = test_state(None).await;
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let created = state
        .session_catalog
        .create_session(temp_dir.path().display().to_string())
        .await
        .expect("session should be created");

    let child_agent_id = spawn_test_child_agent(&guard, &created.session_id, temp_dir.path()).await;

    let app = build_api_router().with_state(state);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/v1/sessions/{}/agents/{}/close",
                    created.session_id, child_agent_id
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
            .any(|id| id.as_str() == Some(child_agent_id.as_str())),
        "closed list should contain the target agent"
    );
}

#[tokio::test]
async fn close_agent_route_accepts_empty_json_body() {
    let (state, guard) = test_state(None).await;
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let created = state
        .session_catalog
        .create_session(temp_dir.path().display().to_string())
        .await
        .expect("session should be created");

    let child_agent_id = spawn_test_child_agent(&guard, &created.session_id, temp_dir.path()).await;

    let app = build_api_router().with_state(state);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/v1/sessions/{}/agents/{}/close",
                    created.session_id, child_agent_id
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
        .session_catalog
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
