use std::{
    collections::HashMap,
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use astrcode_core::{
    AgentEventContext, StorageEvent, StorageEventPayload, StoredEvent, UserMessageOrigin,
};
use astrcode_eval::{
    runner::{EvalRunner, EvalRunnerConfig, report::EvalTaskResultStatus},
    task::loader::TaskLoader,
};
use astrcode_support::hostpaths::project_dir_name;
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
    next_id: Arc<AtomicU64>,
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
        next_id: Arc::new(AtomicU64::new(1)),
    };

    let app = Router::new()
        .route("/", get(|| async { "ok" }))
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
    let id = state.next_id.fetch_add(1, Ordering::SeqCst);
    let session_id = format!("session-{id}");
    let working_dir = PathBuf::from(
        payload["workingDir"]
            .as_str()
            .expect("workingDir should be provided"),
    );
    let canonical_id = session_id.trim_start_matches("session-");
    let session_dir = state
        .projects_root
        .join(project_dir_name(&working_dir))
        .join("sessions")
        .join(canonical_id);
    fs::create_dir_all(&session_dir).expect("session dir should create");
    let log_path = session_dir.join(format!("session-{canonical_id}.jsonl"));
    write_events(
        &log_path,
        &[StoredEvent {
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
        }],
    );

    state.sessions.lock().await.insert(
        canonical_id.to_string(),
        SessionFixture {
            working_dir: working_dir.clone(),
            log_path,
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
    Json(_payload): Json<serde_json::Value>,
) -> (reqwest::StatusCode, Json<serde_json::Value>) {
    let turn_id = format!("turn-{}", state.next_id.fetch_add(1, Ordering::SeqCst));
    let fixture = state
        .sessions
        .lock()
        .await
        .get(&session_id)
        .cloned()
        .expect("session should exist");
    append_turn_events(&fixture.log_path, &turn_id, &fixture.working_dir);

    (
        reqwest::StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "status": "accepted",
            "turnId": turn_id,
            "sessionId": session_id,
        })),
    )
}

fn append_turn_events(log_path: &Path, turn_id: &str, working_dir: &Path) {
    let task_id = task_id_from_working_dir(working_dir).expect("task id should resolve");
    let mut next_seq = read_last_storage_seq(log_path) + 1;
    let agent = AgentEventContext::root_execution("agent-root", "default");
    let mut events = vec![StoredEvent {
        storage_seq: next_seq,
        event: StorageEvent {
            turn_id: Some(turn_id.to_string()),
            agent: agent.clone(),
            payload: StorageEventPayload::UserMessage {
                content: "eval prompt".to_string(),
                timestamp: Utc::now(),
                origin: UserMessageOrigin::User,
            },
        },
    }];
    next_seq += 1;

    let final_output = if task_id == "edit-status" {
        let status_path = working_dir.join("status.txt");
        fs::write(&status_path, "done\n").expect("status file should write");
        events.push(tool_event(next_seq, turn_id, &agent, "call-1", "Edit"));
        next_seq += 1;
        events.push(tool_result_event(
            next_seq, turn_id, &agent, "call-1", "Edit", "done\n",
        ));
        next_seq += 1;
        "done"
    } else {
        "plan"
    };

    events.push(StoredEvent {
        storage_seq: next_seq,
        event: StorageEvent {
            turn_id: Some(turn_id.to_string()),
            agent: agent.clone(),
            payload: StorageEventPayload::AssistantFinal {
                content: final_output.to_string(),
                reasoning_content: None,
                reasoning_signature: None,
                step_index: None,
                timestamp: Some(Utc::now()),
            },
        },
    });
    next_seq += 1;
    events.push(StoredEvent {
        storage_seq: next_seq,
        event: StorageEvent {
            turn_id: Some(turn_id.to_string()),
            agent,
            payload: StorageEventPayload::TurnDone {
                timestamp: Utc.with_ymd_and_hms(2026, 4, 20, 8, 0, 0).unwrap(),
                terminal_kind: None,
                reason: Some("completed".to_string()),
            },
        },
    });
    write_events(log_path, &events);
}

fn tool_event(
    storage_seq: u64,
    turn_id: &str,
    agent: &AgentEventContext,
    tool_call_id: &str,
    tool_name: &str,
) -> StoredEvent {
    StoredEvent {
        storage_seq,
        event: StorageEvent {
            turn_id: Some(turn_id.to_string()),
            agent: agent.clone(),
            payload: StorageEventPayload::ToolCall {
                tool_call_id: tool_call_id.to_string(),
                tool_name: tool_name.to_string(),
                args: serde_json::json!({ "path": "status.txt" }),
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
                success: true,
                error: None,
                metadata: None,
                continuation: None,
                duration_ms: 5,
            },
        },
    }
}

fn task_id_from_working_dir(working_dir: &Path) -> Option<String> {
    let name = working_dir.file_name()?.to_str()?;
    let (task_id, suffix) = name.rsplit_once('-')?;
    suffix
        .chars()
        .all(|ch| ch.is_ascii_digit())
        .then(|| task_id.to_string())
}

fn read_last_storage_seq(log_path: &Path) -> u64 {
    fs::read_to_string(log_path)
        .ok()
        .and_then(|content| {
            content
                .lines()
                .rfind(|line| !line.trim().is_empty())
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

fn write_smoke_task_set(root: &Path) -> PathBuf {
    fs::create_dir_all(root.join("tasks")).expect("task dir should create");
    fs::write(
        root.join("task-set.yaml"),
        "tasks:\n  - tasks/direct-answer.yaml\n  - tasks/edit-status.yaml\n",
    )
    .expect("task set should write");
    fs::write(
        root.join("tasks/direct-answer.yaml"),
        r#"task_id: direct-answer
prompt: 直接回答 plan。
expected_outcome:
  max_tool_calls: 0
  max_turns: 1
  output_equals: "plan"
"#,
    )
    .expect("direct task should write");
    fs::write(
        root.join("tasks/edit-status.yaml"),
        r#"task_id: edit-status
prompt: 把 status.txt 写成 done。
expected_outcome:
  tool_pattern:
    - Edit
  max_tool_calls: 1
  file_changes:
    - path: status.txt
      exact: "done\n"
  max_turns: 1
"#,
    )
    .expect("edit task should write");
    root.join("task-set.yaml")
}

#[test]
fn smoke_task_set_loads_successfully() {
    let temp = tempdir().expect("tempdir should create");
    let task_set = write_smoke_task_set(temp.path());
    let loaded = TaskLoader::load_task_set(&task_set).expect("smoke task set should load");
    assert_eq!(loaded.tasks.len(), 2);
    assert!(loaded.warnings.is_empty());
}

#[test]
fn default_task_set_loads_successfully() {
    let task_set = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../eval-tasks/task-set.yaml");
    let loaded = TaskLoader::load_task_set(&task_set).expect("default task set should load");
    assert!(!loaded.tasks.is_empty());
    assert!(loaded.warnings.is_empty());
}

#[tokio::test]
async fn eval_runner_smoke_generates_report() {
    let temp = tempdir().expect("tempdir should create");
    let projects_root = temp.path().join("projects");
    fs::create_dir_all(&projects_root).expect("projects root should create");
    let task_set = write_smoke_task_set(temp.path());
    let output_path = temp.path().join("report.json");

    let server_addr = start_eval_test_server(projects_root.clone()).await;
    let report = EvalRunner::run(EvalRunnerConfig {
        server_url: format!("http://{server_addr}"),
        session_storage_root: projects_root,
        task_set,
        workspace_root: Some(temp.path().join("workspaces")),
        baseline: None,
        concurrency: 2,
        keep_workspace: false,
        output: Some(output_path.clone()),
        timeout: Duration::from_secs(5),
        poll_interval: Duration::from_millis(25),
        auth_token: None,
    })
    .await
    .expect("runner should succeed");

    assert_eq!(report.results.len(), 2);
    let failed_results = report
        .results
        .iter()
        .filter(|result| result.status != EvalTaskResultStatus::Pass)
        .map(|result| format!("{}: {:?}", result.task_id, result.status))
        .collect::<Vec<_>>();
    assert!(
        failed_results.is_empty(),
        "eval smoke tasks should pass:\n{}",
        failed_results.join("\n")
    );
    assert!(output_path.exists());
}
