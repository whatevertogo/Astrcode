use std::{
    collections::HashMap,
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use astrcode_core::{
    AgentEventContext, StorageEvent, StorageEventPayload, StoredEvent, UserMessageOrigin,
};
use astrcode_eval::runner::{EvalRunner, EvalRunnerConfig};
use axum::{
    Json, Router,
    extract::{Path as AxumPath, State},
    routing::{get, post},
};
use chrono::{TimeZone, Utc};
use tempfile::tempdir;
use tokio::{net::TcpListener, sync::Mutex};

#[derive(Clone)]
struct MockServerState {
    projects_root: PathBuf,
    sessions: Arc<Mutex<HashMap<String, SessionFixture>>>,
}

#[derive(Clone)]
struct SessionFixture {
    working_dir: PathBuf,
    log_path: PathBuf,
}

async fn start_eval_test_server(projects_root: PathBuf) -> SocketAddr {
    let state = MockServerState {
        projects_root,
        sessions: Arc::new(Mutex::new(HashMap::new())),
    };

    let app = Router::new()
        .route(
            "/__astrcode__/run-info",
            get(|| async { Json(serde_json::json!({"ok": true})) }),
        )
        .route("/api/sessions", post(create_session))
        .route("/api/sessions/{id}/prompts", post(submit_prompt))
        .with_state(state);

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let addr = listener.local_addr().expect("addr should resolve");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("server should run");
    });
    addr
}

async fn create_session(
    State(state): State<MockServerState>,
    Json(payload): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    let session_id = format!(
        "session-{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap()
    );
    let working_dir = PathBuf::from(
        payload["workingDir"]
            .as_str()
            .expect("workingDir should be provided"),
    );
    let canonical_id = session_id.trim_start_matches("session-");
    let project_bucket = astrcode_core::project::project_dir_name(&working_dir);
    let session_dir = state
        .projects_root
        .join(project_bucket)
        .join("sessions")
        .join(canonical_id);
    fs::create_dir_all(&session_dir).expect("session dir should create");
    let log_path = session_dir.join(format!("session-{canonical_id}.jsonl"));
    let session_start = StoredEvent {
        storage_seq: 1,
        event: StorageEvent {
            turn_id: None,
            agent: AgentEventContext::default(),
            payload: StorageEventPayload::SessionStart {
                session_id: canonical_id.to_string(),
                timestamp: Utc::now(),
                working_dir: working_dir.display().to_string(),
                parent_session_id: None,
                parent_storage_seq: None,
            },
        },
    };
    write_events(&log_path, &[session_start]);

    state.sessions.lock().await.insert(
        canonical_id.to_string(),
        SessionFixture {
            working_dir: working_dir.clone(),
            log_path: log_path.clone(),
        },
    );

    Json(serde_json::json!({
        "sessionId": canonical_id,
        "workingDir": working_dir.display().to_string(),
    }))
}

async fn submit_prompt(
    State(state): State<MockServerState>,
    AxumPath(session_id): AxumPath<String>,
    Json(payload): Json<serde_json::Value>,
) -> (reqwest::StatusCode, Json<serde_json::Value>) {
    let prompt = payload["text"]
        .as_str()
        .expect("prompt text should exist")
        .to_string();
    let turn_id = format!("turn-{}", chrono::Utc::now().timestamp_nanos_opt().unwrap());
    let fixture = state
        .sessions
        .lock()
        .await
        .get(&session_id)
        .cloned()
        .expect("session should exist");

    apply_prompt_to_workspace(&fixture.working_dir, &prompt);
    append_turn_events(&fixture.log_path, &turn_id, &prompt, &fixture.working_dir);

    (
        reqwest::StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "turnId": turn_id,
            "sessionId": session_id,
        })),
    )
}

fn apply_prompt_to_workspace(working_dir: &Path, prompt: &str) {
    if prompt.contains("DEFAULT_RETRY_COUNT") {
        let path = working_dir.join("src/lib.rs");
        let content = fs::read_to_string(&path).expect("fixture file should read");
        let updated = content.replace(
            "pub const DEFAULT_RETRY_COUNT: u32 = 3;",
            "pub const DEFAULT_RETRY_COUNT: u32 = 5;",
        );
        fs::write(path, updated).expect("edited file should write");
    }

    if prompt.contains("status.txt") {
        fs::write(working_dir.join("status.txt"), "done\n").expect("status file should write");
    }
}

fn append_turn_events(log_path: &Path, turn_id: &str, prompt: &str, working_dir: &Path) {
    let mut next_seq = read_last_storage_seq(log_path) + 1;
    let agent = AgentEventContext::root_execution("agent-root", "default");
    let mut events = Vec::new();
    events.push(StoredEvent {
        storage_seq: next_seq,
        event: StorageEvent {
            turn_id: Some(turn_id.to_string()),
            agent: agent.clone(),
            payload: StorageEventPayload::UserMessage {
                content: prompt.to_string(),
                timestamp: Utc::now(),
                origin: UserMessageOrigin::User,
            },
        },
    });
    next_seq += 1;

    if prompt.contains("README.md") {
        let output = fs::read_to_string(working_dir.join("README.md")).expect("readme should read");
        events.push(tool_call_event(
            next_seq,
            turn_id,
            &agent,
            "call-read",
            "Read",
            serde_json::json!({"path":"README.md"}),
        ));
        next_seq += 1;
        events.push(tool_result_event(
            next_seq,
            turn_id,
            &agent,
            "call-read",
            "Read",
            &output,
            true,
            12,
        ));
        next_seq += 1;
        events.push(StoredEvent {
            storage_seq: next_seq,
            event: StorageEvent {
                turn_id: Some(turn_id.to_string()),
                agent: agent.clone(),
                payload: StorageEventPayload::AssistantFinal {
                    content: "项目名称是 Astrcode Eval，第一条要点是这是一个用于离线评测 Agent \
                              行为的示例项目。"
                        .to_string(),
                    reasoning_content: None,
                    reasoning_signature: None,
                    timestamp: Some(Utc::now()),
                },
            },
        });
        next_seq += 1;
    } else if prompt.contains("DEFAULT_RETRY_COUNT") {
        let original =
            fs::read_to_string(working_dir.join("src/lib.rs")).expect("lib.rs should read");
        events.push(tool_call_event(
            next_seq,
            turn_id,
            &agent,
            "call-read",
            "Read",
            serde_json::json!({"path":"src/lib.rs"}),
        ));
        next_seq += 1;
        events.push(tool_result_event(
            next_seq,
            turn_id,
            &agent,
            "call-read",
            "Read",
            &original,
            true,
            10,
        ));
        next_seq += 1;
        events.push(tool_call_event(
            next_seq,
            turn_id,
            &agent,
            "call-edit",
            "Edit",
            serde_json::json!({"path":"src/lib.rs"}),
        ));
        next_seq += 1;
        let updated =
            fs::read_to_string(working_dir.join("src/lib.rs")).expect("edited lib.rs should read");
        events.push(tool_result_event(
            next_seq,
            turn_id,
            &agent,
            "call-edit",
            "Edit",
            &updated,
            true,
            18,
        ));
        next_seq += 1;
        events.push(StoredEvent {
            storage_seq: next_seq,
            event: StorageEvent {
                turn_id: Some(turn_id.to_string()),
                agent: agent.clone(),
                payload: StorageEventPayload::AssistantFinal {
                    content: "已将 DEFAULT_RETRY_COUNT 更新为 5。".to_string(),
                    reasoning_content: None,
                    reasoning_signature: None,
                    timestamp: Some(Utc::now()),
                },
            },
        });
        next_seq += 1;
    } else if prompt.contains("status.txt") {
        let plan = fs::read_to_string(working_dir.join("docs/plan.md")).expect("plan should read");
        events.push(tool_call_event(
            next_seq,
            turn_id,
            &agent,
            "call-read",
            "Read",
            serde_json::json!({"path":"docs/plan.md"}),
        ));
        next_seq += 1;
        events.push(tool_result_event(
            next_seq,
            turn_id,
            &agent,
            "call-read",
            "Read",
            &plan,
            true,
            9,
        ));
        next_seq += 1;
        events.push(tool_call_event(
            next_seq,
            turn_id,
            &agent,
            "call-edit",
            "Edit",
            serde_json::json!({"path":"status.txt"}),
        ));
        next_seq += 1;
        let updated =
            fs::read_to_string(working_dir.join("status.txt")).expect("status should read");
        events.push(tool_result_event(
            next_seq,
            turn_id,
            &agent,
            "call-edit",
            "Edit",
            &updated,
            true,
            14,
        ));
        next_seq += 1;
        events.push(StoredEvent {
            storage_seq: next_seq,
            event: StorageEvent {
                turn_id: Some(turn_id.to_string()),
                agent: agent.clone(),
                payload: StorageEventPayload::AssistantFinal {
                    content: "已完成读取计划并将 status.txt 更新为 done。".to_string(),
                    reasoning_content: None,
                    reasoning_signature: None,
                    timestamp: Some(Utc::now()),
                },
            },
        });
        next_seq += 1;
    }

    events.push(StoredEvent {
        storage_seq: next_seq,
        event: StorageEvent {
            turn_id: Some(turn_id.to_string()),
            agent,
            payload: StorageEventPayload::TurnDone {
                timestamp: Utc.with_ymd_and_hms(2026, 4, 20, 8, 0, 0).unwrap(),
                reason: Some("completed".to_string()),
            },
        },
    });

    write_events(log_path, &events);
}

fn tool_call_event(
    storage_seq: u64,
    turn_id: &str,
    agent: &AgentEventContext,
    tool_call_id: &str,
    tool_name: &str,
    args: serde_json::Value,
) -> StoredEvent {
    StoredEvent {
        storage_seq,
        event: StorageEvent {
            turn_id: Some(turn_id.to_string()),
            agent: agent.clone(),
            payload: StorageEventPayload::ToolCall {
                tool_call_id: tool_call_id.to_string(),
                tool_name: tool_name.to_string(),
                args,
            },
        },
    }
}

fn tool_result_event(
    storage_seq: u64,
    turn_id: &str,
    agent: &AgentEventContext,
    tool_call_id: &str,
    tool_name: &str,
    output: &str,
    success: bool,
    duration_ms: u64,
) -> StoredEvent {
    StoredEvent {
        storage_seq,
        event: StorageEvent {
            turn_id: Some(turn_id.to_string()),
            agent: agent.clone(),
            payload: StorageEventPayload::ToolResult {
                tool_call_id: tool_call_id.to_string(),
                tool_name: tool_name.to_string(),
                output: output.to_string(),
                success,
                error: None,
                metadata: None,
                continuation: None,
                duration_ms,
            },
        },
    }
}

fn read_last_storage_seq(log_path: &Path) -> u64 {
    fs::read_to_string(log_path)
        .ok()
        .and_then(|content| {
            content
                .lines()
                .filter(|line| !line.trim().is_empty())
                .last()
                .and_then(|line| serde_json::from_str::<StoredEvent>(line).ok())
                .map(|event| event.storage_seq)
        })
        .unwrap_or(0)
}

fn write_events(log_path: &Path, events: &[StoredEvent]) {
    let mut existing = fs::read_to_string(log_path).unwrap_or_default();
    for event in events {
        existing.push_str(&serde_json::to_string(event).expect("stored event should serialize"));
        existing.push('\n');
    }
    fs::write(log_path, existing).expect("log should write");
}

#[tokio::test]
async fn core_task_set_runs_end_to_end_and_generates_report() {
    let temp = tempdir().expect("tempdir should create");
    let projects_root = temp.path().join("projects");
    fs::create_dir_all(&projects_root).expect("projects root should create");
    let output_path = temp.path().join("report.json");

    let server_addr = start_eval_test_server(projects_root.clone()).await;
    let task_set = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../eval-tasks/task-set.yaml");

    let report = EvalRunner::run(EvalRunnerConfig {
        server_url: format!("http://{server_addr}"),
        session_storage_root: projects_root,
        task_set,
        workspace_root: Some(temp.path().join("workspaces")),
        baseline: None,
        concurrency: 3,
        keep_workspace: false,
        output: Some(output_path.clone()),
        timeout: Duration::from_secs(5),
        poll_interval: Duration::from_millis(25),
        auth_token: None,
    })
    .await
    .expect("runner should succeed");

    assert_eq!(report.results.len(), 3);
    assert!(
        report.results.iter().all(
            |result| result.status == astrcode_eval::runner::report::EvalTaskResultStatus::Pass
        )
    );
    assert!(output_path.exists());
}

#[tokio::test]
async fn core_task_set_baseline_diff_is_stable_across_two_runs() {
    let temp = tempdir().expect("tempdir should create");
    let projects_root = temp.path().join("projects");
    fs::create_dir_all(&projects_root).expect("projects root should create");
    let baseline_path = temp.path().join("baseline.json");
    let second_path = temp.path().join("second.json");

    let server_addr = start_eval_test_server(projects_root.clone()).await;
    let task_set = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../eval-tasks/task-set.yaml");

    EvalRunner::run(EvalRunnerConfig {
        server_url: format!("http://{server_addr}"),
        session_storage_root: projects_root.clone(),
        task_set: task_set.clone(),
        workspace_root: Some(temp.path().join("workspaces-first")),
        baseline: None,
        concurrency: 3,
        keep_workspace: false,
        output: Some(baseline_path.clone()),
        timeout: Duration::from_secs(5),
        poll_interval: Duration::from_millis(25),
        auth_token: None,
    })
    .await
    .expect("first run should succeed");

    let report = EvalRunner::run(EvalRunnerConfig {
        server_url: format!("http://{server_addr}"),
        session_storage_root: projects_root,
        task_set,
        workspace_root: Some(temp.path().join("workspaces-second")),
        baseline: Some(baseline_path),
        concurrency: 3,
        keep_workspace: false,
        output: Some(second_path),
        timeout: Duration::from_secs(5),
        poll_interval: Duration::from_millis(25),
        auth_token: None,
    })
    .await
    .expect("second run should succeed");

    let baseline = report.baseline.expect("baseline diff should exist");
    assert_eq!(baseline.diffs.len(), 3);
    assert!(baseline.diffs.iter().all(|diff| diff.score_delta == 0.0
        && diff.tool_calls_delta == 0
        && diff.duration_ms_delta == 0
        && diff.estimated_tokens_delta == 0
        && !diff.regression));
}
