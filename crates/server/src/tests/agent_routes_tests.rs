use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

use astrcode_core::{
    AgentEventContext, CancelToken, SpawnAgentParams, SpawnCapabilityGrant, ToolContext,
    agent::executor::SubAgentExecutor,
};
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use tower::ServiceExt;

use crate::{
    AUTH_HEADER_NAME, AppState,
    auth::{AuthSessionManager, BootstrapAuth},
    routes::build_api_router,
    test_support::{ManualWatchHarness, ServerTestContext, test_state, test_state_with_options},
};

async fn json_body<T: serde::de::DeserializeOwned>(response: axum::http::Response<Body>) -> T {
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body should be readable");
    serde_json::from_slice(&bytes).expect("response should deserialize")
}

fn write_agent_profile(project_dir: &Path, profile_id: &str, description: &str) {
    let agent_dir = project_dir.join(".astrcode").join("agents");
    fs::create_dir_all(&agent_dir).expect("agent dir should be created");
    fs::write(
        agent_dir.join(format!("{profile_id}.md")),
        format!(
            r#"---
name: {profile_id}
description: {description}
tools: ["Read", "Grep"]
---
请根据仓库上下文完成任务。
"#
        ),
    )
    .expect("agent profile should be written");
}

fn normalize_path(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .replace('\\', "/")
}

fn write_global_agent_profile(home_dir: &Path, profile_id: &str, description: &str) {
    let agent_dir = home_dir.join(".astrcode").join("agents");
    fs::create_dir_all(&agent_dir).expect("global agent dir should be created");
    fs::write(
        agent_dir.join(format!("{profile_id}.md")),
        format!(
            r#"---
name: {profile_id}
description: {description}
tools: ["Read", "Grep"]
---
请根据仓库上下文完成任务。
"#
        ),
    )
    .expect("global agent profile should be written");
}

async fn wait_until(timeout: Duration, mut predicate: impl FnMut() -> bool) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if predicate() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err("condition not satisfied before timeout".to_string())
}

#[tokio::test]
async fn execute_agent_returns_not_found_for_unknown_profile_without_creating_session() {
    let (state, _guard) = test_state(None).await;
    let project = tempfile::tempdir().expect("tempdir should be created");
    let before = state
        .app
        .list_sessions()
        .await
        .expect("sessions should list")
        .len();
    let app = build_api_router().with_state(state.clone());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/agents/missing/execute")
                .header(AUTH_HEADER_NAME, "browser-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "task": "run",
                        "workingDir": project.path().display().to_string()
                    })
                    .to_string(),
                ))
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let after = state
        .app
        .list_sessions()
        .await
        .expect("sessions should list")
        .len();
    assert_eq!(after, before, "invalid profile must not create session");
}

#[tokio::test]
async fn execute_agent_rejects_subagent_only_profile_without_creating_session() {
    let (state, _guard) = test_state(None).await;
    let project = tempfile::tempdir().expect("tempdir should be created");
    write_agent_profile(project.path(), "reviewer", "仓库审查");
    let before = state
        .app
        .list_sessions()
        .await
        .expect("sessions should list")
        .len();
    let app = build_api_router().with_state(state.clone());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/agents/reviewer/execute")
                .header(AUTH_HEADER_NAME, "browser-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "task": "run",
                        "workingDir": project.path().display().to_string()
                    })
                    .to_string(),
                ))
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let after = state
        .app
        .list_sessions()
        .await
        .expect("sessions should list")
        .len();
    assert_eq!(after, before, "非法 root profile 不得产生 session");
}

#[tokio::test]
async fn execute_agent_rejects_invalid_execution_control_before_creating_session() {
    let (state, _guard) = test_state(None).await;
    let project = tempfile::tempdir().expect("tempdir should be created");
    let before = state
        .app
        .list_sessions()
        .await
        .expect("sessions should list")
        .len();
    let app = build_api_router().with_state(state.clone());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/agents/explore/execute")
                .header(AUTH_HEADER_NAME, "browser-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "task": "run",
                        "workingDir": project.path().display().to_string(),
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

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let after = state
        .app
        .list_sessions()
        .await
        .expect("sessions should list")
        .len();
    assert_eq!(after, before, "非法 execution control 不得创建 session");
}

#[tokio::test]
async fn execute_agent_rejects_unsupported_context_overrides_before_creating_session() {
    let (state, _guard) = test_state(None).await;
    let project = tempfile::tempdir().expect("tempdir should be created");
    let before = state
        .app
        .list_sessions()
        .await
        .expect("sessions should list")
        .len();
    let app = build_api_router().with_state(state.clone());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/agents/explore/execute")
                .header(AUTH_HEADER_NAME, "browser-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "task": "run",
                        "workingDir": project.path().display().to_string(),
                        "contextOverrides": {
                            "inheritWorkingDir": false
                        }
                    })
                    .to_string(),
                ))
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let after = state
        .app
        .list_sessions()
        .await
        .expect("sessions should list")
        .len();
    assert_eq!(after, before, "非法 context overrides 不得创建 session");
}

#[tokio::test]
async fn subagent_launch_uses_resolved_profile_and_inherits_parent_working_dir() {
    let (state, _guard) = test_state(None).await;
    let project = tempfile::tempdir().expect("tempdir should be created");
    write_agent_profile(project.path(), "reviewer", "仓库审查");
    let project_dir = normalize_path(project.path());

    let parent = state
        .app
        .create_session(project.path().display().to_string())
        .await
        .expect("parent session should be created");
    state
        .app
        .kernel()
        .register_root_agent(
            "root-agent".to_string(),
            parent.session_id.clone(),
            "root-profile".to_string(),
        )
        .await
        .expect("root agent should be registered");

    let ctx = ToolContext::new(
        parent.session_id.clone().into(),
        project.path().to_path_buf(),
        CancelToken::new(),
    )
    .with_turn_id("turn-1")
    .with_agent_context(AgentEventContext::root_execution(
        "root-agent",
        "root-profile",
    ));

    let result = state
        .app
        .agent()
        .launch(
            SpawnAgentParams {
                r#type: Some("reviewer".to_string()),
                description: "仓库审查".to_string(),
                prompt: "请阅读代码".to_string(),
                context: Some("关注最近修改".to_string()),
                capability_grant: Some(SpawnCapabilityGrant {
                    allowed_tools: vec!["observe".to_string()],
                }),
            },
            &ctx,
        )
        .await
        .expect("subagent should launch");

    let artifacts = &result
        .handoff()
        .as_ref()
        .expect("handoff should exist")
        .artifacts;
    let child_agent_id = artifacts
        .iter()
        .find(|artifact| artifact.kind == "agent")
        .map(|artifact| artifact.id.clone())
        .expect("child agent artifact should exist");
    let child_session_id = artifacts
        .iter()
        .find(|artifact| artifact.kind == "session")
        .map(|artifact| artifact.id.clone())
        .expect("child session artifact should exist");

    let subrun = state
        .app
        .get_subrun_status(&child_agent_id)
        .await
        .expect("subrun query should succeed")
        .expect("child subrun should exist");
    assert_eq!(subrun.agent_profile, "reviewer");
    assert_eq!(
        subrun.session_id, parent.session_id,
        "sub-run handle should stay attached to the parent session"
    );
    assert_eq!(
        subrun.child_session_id.as_deref(),
        Some(child_session_id.as_str()),
        "independent child session id should be preserved on the handle"
    );
    assert_eq!(
        subrun.resolved_limits.allowed_tools,
        vec!["observe".to_string()],
        "live status should expose the launch-time capability grant intersection"
    );
    assert!(
        subrun.resolved_limits.max_steps.is_some(),
        "live status should expose the launch-time max step limit"
    );

    let child_meta = state
        .app
        .list_sessions()
        .await
        .expect("sessions should list")
        .into_iter()
        .find(|meta| meta.session_id == child_session_id)
        .expect("child session meta should exist");
    assert_eq!(
        normalize_path(Path::new(&child_meta.working_dir)),
        project_dir
    );
}

#[tokio::test]
async fn subagent_launch_rejects_missing_profile_without_creating_child_session() {
    let (state, _guard) = test_state(None).await;
    let project = tempfile::tempdir().expect("tempdir should be created");

    let parent = state
        .app
        .create_session(project.path().display().to_string())
        .await
        .expect("parent session should be created");
    state
        .app
        .kernel()
        .register_root_agent(
            "root-agent".to_string(),
            parent.session_id.clone(),
            "root-profile".to_string(),
        )
        .await
        .expect("root agent should be registered");

    let before = state
        .app
        .list_sessions()
        .await
        .expect("sessions should list")
        .len();
    let ctx = ToolContext::new(
        parent.session_id.clone().into(),
        project.path().to_path_buf(),
        CancelToken::new(),
    )
    .with_turn_id("turn-1")
    .with_agent_context(AgentEventContext::root_execution(
        "root-agent",
        "root-profile",
    ));

    let error = state
        .app
        .agent()
        .launch(
            SpawnAgentParams {
                r#type: Some("missing".to_string()),
                description: "缺失 profile".to_string(),
                prompt: "请阅读代码".to_string(),
                context: None,
                capability_grant: None,
            },
            &ctx,
        )
        .await
        .expect_err("missing profile should be rejected");
    assert!(
        error.to_string().contains("missing"),
        "error should mention missing profile: {error}"
    );

    let after = state
        .app
        .list_sessions()
        .await
        .expect("sessions should list")
        .len();
    assert_eq!(
        after, before,
        "无效 subagent profile 不得创建 child session"
    );
}

#[tokio::test]
async fn get_subrun_status_falls_back_to_durable_snapshot_with_resolved_limits() {
    let context = ServerTestContext::new();
    let initial_runtime = crate::bootstrap::bootstrap_server_runtime_with_options(
        crate::bootstrap::ServerBootstrapOptions {
            home_dir: Some(context.home_dir().to_path_buf()),
            enable_profile_watch: false,
            ..crate::bootstrap::ServerBootstrapOptions::default()
        },
    )
    .await
    .expect("server runtime should bootstrap");
    let project = tempfile::tempdir().expect("tempdir should be created");
    write_agent_profile(project.path(), "reviewer", "仓库审查");
    let parent = initial_runtime
        .app
        .create_session(project.path().display().to_string())
        .await
        .expect("parent session should be created");
    initial_runtime
        .app
        .kernel()
        .register_root_agent(
            "root-agent".to_string(),
            parent.session_id.clone(),
            "root-profile".to_string(),
        )
        .await
        .expect("root agent should be registered");

    let ctx = ToolContext::new(
        parent.session_id.clone().into(),
        project.path().to_path_buf(),
        CancelToken::new(),
    )
    .with_turn_id("turn-1")
    .with_agent_context(AgentEventContext::root_execution(
        "root-agent",
        "root-profile",
    ));

    let result = initial_runtime
        .app
        .agent()
        .launch(
            SpawnAgentParams {
                r#type: Some("reviewer".to_string()),
                description: "仓库审查".to_string(),
                prompt: "请阅读代码".to_string(),
                context: Some("关注最近修改".to_string()),
                capability_grant: Some(SpawnCapabilityGrant {
                    allowed_tools: vec!["observe".to_string()],
                }),
            },
            &ctx,
        )
        .await
        .expect("subagent should launch");

    let child_agent_id = result
        .handoff()
        .as_ref()
        .expect("handoff should exist")
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == "agent")
        .map(|artifact| artifact.id.clone())
        .expect("child agent artifact should exist");

    initial_runtime
        .app
        .close_agent(&parent.session_id, &child_agent_id)
        .await
        .expect("child should be closable so live handle disappears");

    drop(initial_runtime);

    let reloaded_runtime = crate::bootstrap::bootstrap_server_runtime_with_options(
        crate::bootstrap::ServerBootstrapOptions {
            home_dir: Some(context.home_dir().to_path_buf()),
            enable_profile_watch: false,
            ..crate::bootstrap::ServerBootstrapOptions::default()
        },
    )
    .await
    .expect("reloaded server runtime should bootstrap from durable state");
    let auth_sessions = std::sync::Arc::new(AuthSessionManager::default());
    auth_sessions.issue_test_token("browser-token");
    let app = build_api_router().with_state(AppState {
        app: std::sync::Arc::clone(&reloaded_runtime.app),
        governance: std::sync::Arc::clone(&reloaded_runtime.governance),
        auth_sessions,
        bootstrap_auth: BootstrapAuth::new(
            "browser-token".to_string(),
            chrono::Utc::now()
                .checked_add_signed(chrono::Duration::seconds(60))
                .expect("expiry should be valid")
                .timestamp_millis(),
        ),
        frontend_build: None,
        _runtime_handles: reloaded_runtime.handles,
    });
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v1/sessions/{}/subruns/{}",
                    parent.session_id, child_agent_id
                ))
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::OK);
    let payload: astrcode_protocol::http::SubRunStatusDto = json_body(response).await;
    assert_eq!(
        payload.source,
        astrcode_protocol::http::SubRunStatusSourceDto::Durable
    );
    assert_eq!(
        payload
            .resolved_limits
            .expect("durable fallback should expose resolved limits")
            .allowed_tools,
        vec!["observe".to_string()]
    );
}

#[tokio::test]
async fn list_agents_uses_loader_backed_profiles() {
    let (state, _guard) = test_state(None).await;
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/agents")
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::OK);
    let payload: Vec<astrcode_protocol::http::AgentProfileDto> = json_body(response).await;
    assert!(
        payload.iter().any(|profile| profile.id == "explore"),
        "builtin loader profiles should be exposed"
    );
    assert!(
        payload.iter().all(|profile| profile.id != "root-agent"),
        "route should no longer return synthetic placeholder profiles"
    );
}

#[tokio::test]
async fn scoped_agent_profile_watch_refreshes_profiles_without_restart() {
    let watch = ManualWatchHarness::new();
    let (state, _guard) = test_state_with_options(
        None,
        crate::bootstrap::ServerBootstrapOptions {
            enable_profile_watch: true,
            watch_service_override: Some(watch.service()),
            ..crate::bootstrap::ServerBootstrapOptions::default()
        },
    )
    .await;
    let project = tempfile::tempdir().expect("tempdir should be created");
    let scoped_working_dir = project.path().display().to_string();
    write_agent_profile(project.path(), "reviewer", "初始描述");
    let session = state
        .app
        .create_session(scoped_working_dir.clone())
        .await
        .expect("session should be created to register watch source");
    let scoped_source = astrcode_application::WatchSource::AgentDefinitions {
        working_dir: session.working_dir.clone(),
    };
    watch
        .wait_for_source(&scoped_source, Duration::from_secs(5))
        .await
        .expect("scoped watch source should be registered before emitting changes");

    let first = state
        .app
        .list_agent_profiles_for_working_dir(project.path())
        .expect("profiles should resolve");
    let first_reviewer = first
        .iter()
        .find(|profile| profile.id == "reviewer")
        .expect("reviewer profile should exist");
    assert_eq!(first_reviewer.description, "初始描述");

    write_agent_profile(project.path(), "reviewer", "更新后的描述");
    watch.emit(
        scoped_source,
        vec![".astrcode/agents/reviewer.md".to_string()],
    );

    wait_until(Duration::from_secs(5), || {
        state
            .app
            .list_agent_profiles_for_working_dir(project.path())
            .ok()
            .and_then(|profiles| {
                profiles
                    .into_iter()
                    .find(|profile| profile.id == "reviewer")
                    .map(|profile| profile.description == "更新后的描述")
            })
            .unwrap_or(false)
    })
    .await
    .expect("scoped profile watch should refresh cached result");
}

#[tokio::test]
async fn global_agent_profile_watch_invalidates_scoped_cache_without_restart() {
    let watch = ManualWatchHarness::new();
    let context = ServerTestContext::new();
    write_global_agent_profile(context.home_dir(), "watcher-profile", "全局初始描述");
    let runtime = crate::bootstrap::bootstrap_server_runtime_with_options(
        crate::bootstrap::ServerBootstrapOptions {
            home_dir: Some(context.home_dir().to_path_buf()),
            watch_service_override: Some(watch.service()),
            ..crate::bootstrap::ServerBootstrapOptions::default()
        },
    )
    .await
    .expect("server runtime should bootstrap");
    let app = std::sync::Arc::clone(&runtime.app);
    let _runtime_handles = std::sync::Arc::clone(&runtime.handles);
    let project = tempfile::tempdir().expect("tempdir should be created");
    app.create_session(project.path().display().to_string())
        .await
        .expect("session should be created to register watch source");

    let scoped_before = app
        .list_agent_profiles_for_working_dir(project.path())
        .expect("scoped profiles should resolve");
    assert_eq!(
        scoped_before
            .iter()
            .find(|profile| profile.id == "watcher-profile")
            .expect("custom global profile should exist in scoped view")
            .description,
        "全局初始描述"
    );
    let global_before = app
        .list_global_agent_profiles()
        .expect("global profiles should resolve");
    assert_eq!(
        global_before
            .iter()
            .find(|profile| profile.id == "watcher-profile")
            .expect("custom global profile should exist in global view")
            .description,
        "全局初始描述"
    );

    tokio::time::sleep(Duration::from_millis(150)).await;
    write_global_agent_profile(context.home_dir(), "watcher-profile", "全局更新描述");
    watch.emit(
        astrcode_application::WatchSource::GlobalAgentDefinitions,
        vec![".astrcode/agents/watcher-profile.md".to_string()],
    );

    wait_until(Duration::from_secs(5), || {
        let scoped_updated = app
            .list_agent_profiles_for_working_dir(project.path())
            .ok()
            .and_then(|profiles| {
                profiles
                    .into_iter()
                    .find(|profile| profile.id == "watcher-profile")
                    .map(|profile| profile.description == "全局更新描述")
            })
            .unwrap_or(false);
        let global_updated = app
            .list_global_agent_profiles()
            .ok()
            .and_then(|profiles| {
                profiles
                    .into_iter()
                    .find(|profile| profile.id == "watcher-profile")
                    .map(|profile| profile.description == "全局更新描述")
            })
            .unwrap_or(false);
        scoped_updated && global_updated
    })
    .await
    .expect("global profile watch should invalidate both global and scoped caches");
}

#[tokio::test]
async fn get_subrun_status_rejects_mismatched_root_subrun_id() {
    let (state, _guard) = test_state(None).await;
    let project = tempfile::tempdir().expect("tempdir should be created");
    let session = state
        .app
        .create_session(project.path().display().to_string())
        .await
        .expect("session should be created");
    state
        .app
        .kernel()
        .register_root_agent(
            "root-agent".to_string(),
            session.session_id.clone(),
            "root-profile".to_string(),
        )
        .await
        .expect("root agent should be registered");
    let app = build_api_router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v1/sessions/{}/subruns/not-the-root-subrun",
                    session.session_id
                ))
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn close_agent_rejects_cross_session_requests() {
    let (state, _guard) = test_state(None).await;
    let project = tempfile::tempdir().expect("tempdir should be created");
    let owner_session = state
        .app
        .create_session(project.path().display().to_string())
        .await
        .expect("owner session should be created");
    let other_session = state
        .app
        .create_session(project.path().display().to_string())
        .await
        .expect("other session should be created");
    state
        .app
        .kernel()
        .register_root_agent(
            "root-agent".to_string(),
            owner_session.session_id.clone(),
            "root-profile".to_string(),
        )
        .await
        .expect("root agent should be registered");
    let app = build_api_router().with_state(state.clone());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/v1/sessions/{}/agents/root-agent/close",
                    other_session.session_id
                ))
                .header(AUTH_HEADER_NAME, "browser-token")
                .body(Body::empty())
                .expect("request should be valid"),
        )
        .await
        .expect("response should be returned");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(
        state.app.kernel().get_handle("root-agent").await.is_some(),
        "跨 session 请求不得关闭目标 agent"
    );
}
