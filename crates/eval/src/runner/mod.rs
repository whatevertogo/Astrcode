pub mod client;
pub mod report;
pub mod workspace;

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use astrcode_core::{StorageEvent, StorageEventPayload, StoredEvent, project::project_dir_name};
use tokio::{sync::Semaphore, task::JoinSet, time::sleep};

use self::{
    client::ServerControlClient,
    report::{
        EvalReport, EvalTaskMetrics, EvalTaskResult, EvalTaskResultStatus, ReportWriter,
        status_from_score,
    },
    workspace::WorkspaceManager,
};
use crate::{
    EvalError, EvalResult,
    diagnosis::{
        DiagnosisEngine, cascade_failure::CascadeFailureDetector,
        compact_loss::CompactInfoLossDetector, empty_turn::EmptyTurnDetector,
        subrun_budget::SubRunBudgetDetector, tool_loop::ToolLoopDetector,
    },
    task::{EvalTask, loader::TaskLoader, scorer::TaskScorer},
    trace::{SessionTrace, extractor::TraceExtractor},
};

#[derive(Debug, Clone)]
pub struct EvalRunnerConfig {
    pub server_url: String,
    pub session_storage_root: PathBuf,
    pub task_set: PathBuf,
    pub workspace_root: Option<PathBuf>,
    pub baseline: Option<PathBuf>,
    pub concurrency: usize,
    pub keep_workspace: bool,
    pub output: Option<PathBuf>,
    pub timeout: Duration,
    pub poll_interval: Duration,
    pub auth_token: Option<String>,
}

impl Default for EvalRunnerConfig {
    fn default() -> Self {
        Self {
            server_url: "http://127.0.0.1:5529".to_string(),
            session_storage_root: PathBuf::from(".astrcode/projects"),
            task_set: PathBuf::from("eval-tasks/task-set.yaml"),
            workspace_root: None,
            baseline: None,
            concurrency: 1,
            keep_workspace: false,
            output: None,
            timeout: Duration::from_secs(300),
            poll_interval: Duration::from_millis(500),
            auth_token: std::env::var("ASTRCODE_EVAL_TOKEN").ok(),
        }
    }
}

pub struct EvalRunner;

impl EvalRunner {
    pub async fn run(config: EvalRunnerConfig) -> EvalResult<EvalReport> {
        ensure_data_plane_access(&config.session_storage_root)?;

        let loaded = TaskLoader::load_task_set(&config.task_set)?;
        let task_load_warnings: Vec<String> = loaded
            .warnings
            .iter()
            .map(|warning| format!("跳过任务 {}: {}", warning.path.display(), warning.message))
            .collect();
        let workspace_root = config
            .workspace_root
            .clone()
            .unwrap_or_else(|| std::env::temp_dir().join("astrcode-eval-workspaces"));
        let workspace_manager = WorkspaceManager::new(workspace_root, config.keep_workspace);
        workspace_manager.create_root()?;

        let client = ServerControlClient::new(
            config.server_url.clone(),
            config.auth_token.clone(),
            config.timeout,
        )?;
        client.probe().await?;
        verify_session_storage_alignment(&client, &workspace_manager.root, &config).await?;

        let order: HashMap<String, usize> = loaded
            .tasks
            .iter()
            .enumerate()
            .map(|(index, task)| (task.task_id.clone(), index))
            .collect();

        let semaphore = Arc::new(Semaphore::new(config.concurrency.max(1)));
        let mut join_set = JoinSet::new();
        for task in loaded.tasks {
            let permit = Arc::clone(&semaphore)
                .acquire_owned()
                .await
                .map_err(|_| EvalError::validation("并发信号量已关闭"))?;
            let client = client.clone();
            let workspace_manager = workspace_manager.clone();
            let config = config.clone();
            join_set.spawn(async move {
                let result = execute_task(task, client, workspace_manager, &config).await;
                drop(permit);
                result
            });
        }

        let mut results = Vec::new();
        while let Some(joined) = join_set.join_next().await {
            let result = joined
                .map_err(|error| EvalError::validation(format!("评测任务 join 失败: {error}")))?;
            results.push(result);
        }
        results.sort_by_key(|result| order.get(&result.task_id).copied().unwrap_or(usize::MAX));

        let task_set_name = config
            .task_set
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("task-set")
            .to_string();
        let mut report = ReportWriter::build(task_set_name, results);
        report.warnings.extend(task_load_warnings);
        if let Some(baseline) = &config.baseline {
            ReportWriter::attach_baseline(&mut report, baseline, 0.05)?;
        }
        if let Some(output) = &config.output {
            ReportWriter::persist(&report, output)?;
        }
        Ok(report)
    }
}

pub fn session_log_path(
    session_storage_root: &Path,
    working_dir: &Path,
    session_id: &str,
) -> PathBuf {
    let canonical_session_id = canonical_session_id(session_id);
    session_storage_root
        .join(project_dir_name(working_dir))
        .join("sessions")
        .join(&canonical_session_id)
        .join(format!("session-{canonical_session_id}.jsonl"))
}

pub async fn wait_for_turn_done(
    session_log_path: &Path,
    turn_id: &str,
    timeout: Duration,
    poll_interval: Duration,
) -> EvalResult<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if session_log_path.exists() && turn_completed(session_log_path, turn_id)? {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(EvalError::timeout(format!(
                "等待 turn {turn_id} 完成超时: {}",
                session_log_path.display()
            )));
        }
        sleep(poll_interval).await;
    }
}

async fn execute_task(
    task: EvalTask,
    client: ServerControlClient,
    workspace_manager: WorkspaceManager,
    config: &EvalRunnerConfig,
) -> EvalTaskResult {
    let workspace_path = match workspace_manager.prepare(&task) {
        Ok(path) => path,
        Err(error) => return task_error(&task.task_id, EvalTaskResultStatus::Error, error),
    };

    let result = execute_task_inner(&task, &client, &workspace_path, config).await;
    if let Err(cleanup_error) = workspace_manager.cleanup(&workspace_path) {
        return task_error(&task.task_id, EvalTaskResultStatus::Error, cleanup_error);
    }

    match result {
        Ok(mut result) => {
            if config.keep_workspace {
                result.workspace_path = Some(workspace_path.display().to_string());
            }
            result
        },
        Err(error) => {
            let status = if matches!(error, EvalError::Timeout(_)) {
                EvalTaskResultStatus::Timeout
            } else {
                EvalTaskResultStatus::Error
            };
            task_error(&task.task_id, status, error)
        },
    }
}

async fn execute_task_inner(
    task: &EvalTask,
    client: &ServerControlClient,
    workspace_path: &Path,
    config: &EvalRunnerConfig,
) -> EvalResult<EvalTaskResult> {
    let working_dir = workspace_path.display().to_string();
    let session = client.create_session(&working_dir).await?;
    let session_log = session_log_path(
        &config.session_storage_root,
        Path::new(&session.working_dir),
        &session.session_id,
    );
    ensure_session_log_accessible(
        &session_log,
        config.poll_interval,
        session_storage_probe_timeout(config.poll_interval),
    )
    .await?;
    let accepted = client
        .submit_turn(&session.session_id, &task.prompt)
        .await?;
    wait_for_turn_done(
        &session_log,
        &accepted.turn_id,
        config.timeout,
        config.poll_interval,
    )
    .await?;

    let trace = TraceExtractor::extract_file(&session_log)?;
    let diagnosis = diagnose(&trace);
    let score = TaskScorer::score(task, &trace, workspace_path)?;
    let metrics = metrics_from_trace(&trace);

    Ok(EvalTaskResult {
        task_id: task.task_id.clone(),
        status: status_from_score(&score),
        score: score.score,
        diagnosis: Some(diagnosis),
        metrics,
        session_id: Some(accepted.session_id),
        workspace_path: None,
        error: None,
    })
}

fn diagnose(trace: &SessionTrace) -> crate::diagnosis::DiagnosisReport {
    let mut engine = DiagnosisEngine::new();
    engine.register(ToolLoopDetector::default());
    engine.register(CascadeFailureDetector);
    engine.register(CompactInfoLossDetector::default());
    engine.register(SubRunBudgetDetector);
    engine.register(EmptyTurnDetector::default());
    engine.diagnose_session(trace)
}

fn metrics_from_trace(trace: &SessionTrace) -> EvalTaskMetrics {
    let tool_calls = trace
        .turns
        .iter()
        .map(|turn| turn.tool_calls.len())
        .sum::<usize>();
    let duration_ms = trace
        .turns
        .iter()
        .flat_map(|turn| turn.tool_calls.iter().filter_map(|call| call.duration_ms))
        .sum::<u64>();
    let estimated_tokens = trace
        .turns
        .iter()
        .flat_map(|turn| {
            turn.prompt_metrics
                .iter()
                .map(|metrics| metrics.metrics.estimated_tokens as u64)
        })
        .sum::<u64>()
        + trace
            .turns
            .iter()
            .flat_map(|turn| {
                turn.sub_runs
                    .iter()
                    .filter_map(|sub_run| sub_run.estimated_tokens)
            })
            .sum::<u64>();

    EvalTaskMetrics {
        tool_calls,
        duration_ms,
        estimated_tokens,
        turn_count: trace.turns.len(),
    }
}

fn turn_completed(session_log_path: &Path, turn_id: &str) -> EvalResult<bool> {
    let content = fs::read_to_string(session_log_path).map_err(|error| {
        EvalError::io(
            format!("读取 session log {} 失败", session_log_path.display()),
            error,
        )
    })?;
    for (index, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let stored = serde_json::from_str::<StoredEvent>(line)
            .or_else(|_| {
                serde_json::from_str::<StorageEvent>(line).map(|event| StoredEvent {
                    storage_seq: 0,
                    event,
                })
            })
            .map_err(|error| EvalError::JsonLine {
                line: index + 1,
                source: error,
            })?;
        if stored.event.turn_id.as_deref() == Some(turn_id)
            && matches!(stored.event.payload, StorageEventPayload::TurnDone { .. })
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn ensure_data_plane_access(session_storage_root: &Path) -> EvalResult<()> {
    if session_storage_root.exists() && session_storage_root.is_dir() {
        Ok(())
    } else {
        Err(EvalError::validation(format!(
            "session_storage_root 不可访问，控制面/数据面不一致: {}",
            session_storage_root.display()
        )))
    }
}

async fn verify_session_storage_alignment(
    client: &ServerControlClient,
    workspace_root: &Path,
    config: &EvalRunnerConfig,
) -> EvalResult<()> {
    let probe_dir = workspace_root.join(format!(
        "__session-storage-probe-{}",
        chrono::Utc::now().timestamp_millis()
    ));
    fs::create_dir_all(&probe_dir).map_err(|error| {
        EvalError::io(
            format!(
                "创建 session storage probe 工作区 {} 失败",
                probe_dir.display()
            ),
            error,
        )
    })?;

    let result = async {
        let session = client
            .create_session(&probe_dir.display().to_string())
            .await?;
        let session_log = session_log_path(
            &config.session_storage_root,
            Path::new(&session.working_dir),
            &session.session_id,
        );
        ensure_session_log_accessible(
            &session_log,
            config.poll_interval,
            session_storage_probe_timeout(config.poll_interval),
        )
        .await
    }
    .await;

    if probe_dir.exists() {
        fs::remove_dir_all(&probe_dir).map_err(|error| {
            EvalError::io(
                format!(
                    "清理 session storage probe 工作区 {} 失败",
                    probe_dir.display()
                ),
                error,
            )
        })?;
    }

    result
}

async fn ensure_session_log_accessible(
    session_log_path: &Path,
    poll_interval: Duration,
    timeout: Duration,
) -> EvalResult<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if session_log_path.is_file() {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(EvalError::validation(format!(
                "session log 不可访问，控制面/数据面不一致: {}",
                session_log_path.display()
            )));
        }
        sleep(poll_interval).await;
    }
}

fn session_storage_probe_timeout(poll_interval: Duration) -> Duration {
    poll_interval
        .saturating_mul(4)
        .max(Duration::from_millis(250))
}

fn canonical_session_id(session_id: &str) -> String {
    session_id
        .trim()
        .strip_prefix("session-")
        .unwrap_or(session_id.trim())
        .to_string()
}

fn task_error(
    task_id: &str,
    status: EvalTaskResultStatus,
    error: impl std::fmt::Display,
) -> EvalTaskResult {
    EvalTaskResult {
        task_id: task_id.to_string(),
        status,
        score: 0.0,
        diagnosis: None,
        metrics: EvalTaskMetrics::default(),
        session_id: None,
        workspace_path: None,
        error: Some(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        net::SocketAddr,
        path::{Path, PathBuf},
        time::Duration,
    };

    use astrcode_core::{AgentEventContext, StorageEvent, StorageEventPayload, StoredEvent};
    use axum::{
        Json, Router,
        extract::{Path as AxumPath, State},
        routing::{get, post},
    };
    use chrono::{TimeZone, Utc};
    use serde_json::json;
    use tempfile::tempdir;
    use tokio::net::TcpListener;

    use super::{EvalRunner, EvalRunnerConfig, session_log_path};

    #[derive(Clone)]
    struct MockState {
        projects_root: PathBuf,
    }

    async fn mock_server(projects_root: PathBuf) -> SocketAddr {
        let state = MockState { projects_root };
        let app = Router::new()
            .route("/__astrcode__/run-info", get(|| async { Json(json!({"ok": true})) }))
            .route(
                "/api/sessions",
                post(
                    |State(state): State<MockState>, Json(payload): Json<serde_json::Value>| async move {
                        let session_id = format!("session-{}", chrono::Utc::now().timestamp_millis());
                        let working_dir = payload["workingDir"]
                            .as_str()
                            .expect("workingDir should exist")
                            .to_string();
                        let session_log = session_log_path(
                            &state.projects_root,
                            Path::new(&working_dir),
                            &session_id,
                        );
                        if let Some(parent) = session_log.parent() {
                            fs::create_dir_all(parent).expect("session dir should create");
                        }
                        fs::write(&session_log, "").expect("session log should exist");
                        Json(json!({
                            "sessionId": session_id,
                            "workingDir": working_dir,
                        }))
                    },
                ),
            )
            .route(
                "/api/sessions/{id}/prompts",
                post(
                    |State(state): State<MockState>,
                     AxumPath(session_id): AxumPath<String>,
                     Json(payload): Json<serde_json::Value>| async move {
                        let turn_id = "turn-1".to_string();
                        let turn_id_for_writer = turn_id.clone();
                        let projects_root = state.projects_root.clone();
                        let session_id_clone = session_id.clone();
                        let should_hang = payload["text"].as_str() == Some("hang");
                        if !should_hang {
                            tokio::spawn(async move {
                                tokio::time::sleep(Duration::from_millis(50)).await;
                                let session_glob = format!(
                                    "{}/**/sessions/{}/session-{}.jsonl",
                                    projects_root.display(),
                                    session_id_clone.trim_start_matches("session-"),
                                    session_id_clone.trim_start_matches("session-"),
                                );
                                let log_path = glob::glob(&session_glob)
                                    .expect("glob should parse")
                                    .find_map(Result::ok)
                                    .expect("session log should exist");
                                let event = StoredEvent {
                                    storage_seq: 1,
                                    event: StorageEvent {
                                        turn_id: Some(turn_id_for_writer),
                                        agent: AgentEventContext::root_execution("agent-root", "default"),
                                        payload: StorageEventPayload::TurnDone {
                                            timestamp: Utc.with_ymd_and_hms(2026, 4, 20, 8, 0, 0).unwrap(),
                                            terminal_kind: Some(astrcode_core::TurnTerminalKind::Completed),
                                            reason: Some("completed".to_string()),
                                        },
                                    },
                                };
                                fs::write(&log_path, serde_json::to_string(&event).expect("event should serialize"))
                                    .expect("session log should write");
                            });
                        }
                        (
                            reqwest::StatusCode::ACCEPTED,
                            Json(json!({
                                "turnId": turn_id,
                                "sessionId": session_id,
                            })),
                        )
                    },
                ),
            )
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

    #[tokio::test]
    async fn runner_executes_tasks_with_mock_server() {
        let temp = tempdir().expect("tempdir should create");
        let task_dir = temp.path().join("eval-tasks");
        fs::create_dir_all(temp.path().join("projects")).expect("projects dir should create");
        fs::create_dir_all(task_dir.join("core")).expect("task dir should create");
        fs::write(
            task_dir.join("task-set.yaml"),
            "tasks:\n  - core/simple.yaml\n",
        )
        .expect("task set should write");
        fs::write(
            task_dir.join("core").join("simple.yaml"),
            r#"
task_id: simple
prompt: hello
expected_outcome:
  max_turns: 1
"#,
        )
        .expect("task file should write");

        let addr = mock_server(temp.path().join("projects")).await;
        let report = EvalRunner::run(EvalRunnerConfig {
            server_url: format!("http://{addr}"),
            session_storage_root: temp.path().join("projects"),
            task_set: task_dir.join("task-set.yaml"),
            workspace_root: Some(temp.path().join("workspaces")),
            baseline: None,
            concurrency: 2,
            keep_workspace: false,
            output: None,
            timeout: Duration::from_secs(3),
            poll_interval: Duration::from_millis(20),
            auth_token: None,
        })
        .await
        .expect("runner should succeed");

        assert_eq!(report.results.len(), 1);
        assert_eq!(
            report.results[0].status,
            crate::runner::report::EvalTaskResultStatus::Pass
        );
    }

    #[tokio::test]
    async fn runner_keeps_other_tasks_running_when_one_times_out() {
        let temp = tempdir().expect("tempdir should create");
        let task_dir = temp.path().join("eval-tasks");
        fs::create_dir_all(temp.path().join("projects")).expect("projects dir should create");
        fs::create_dir_all(task_dir.join("core")).expect("task dir should create");
        fs::write(
            task_dir.join("task-set.yaml"),
            "tasks:\n  - core/pass.yaml\n  - core/timeout.yaml\n",
        )
        .expect("task set should write");
        fs::write(
            task_dir.join("core").join("pass.yaml"),
            r#"
task_id: pass
prompt: hello
expected_outcome:
  max_turns: 1
"#,
        )
        .expect("pass task should write");
        fs::write(
            task_dir.join("core").join("timeout.yaml"),
            r#"
task_id: timeout
prompt: hang
expected_outcome:
  max_turns: 1
"#,
        )
        .expect("timeout task should write");

        let addr = mock_server(temp.path().join("projects")).await;
        let report = EvalRunner::run(EvalRunnerConfig {
            server_url: format!("http://{addr}"),
            session_storage_root: temp.path().join("projects"),
            task_set: task_dir.join("task-set.yaml"),
            workspace_root: Some(temp.path().join("workspaces")),
            baseline: None,
            concurrency: 2,
            keep_workspace: false,
            output: None,
            timeout: Duration::from_millis(150),
            poll_interval: Duration::from_millis(20),
            auth_token: None,
        })
        .await
        .expect("runner should complete with mixed outcomes");

        assert_eq!(report.results.len(), 2);
        assert!(
            report
                .results
                .iter()
                .any(|result| result.status == crate::runner::report::EvalTaskResultStatus::Pass)
        );
        assert!(report
            .results
            .iter()
            .any(|result| result.status == crate::runner::report::EvalTaskResultStatus::Timeout));
    }

    #[tokio::test]
    async fn runner_surfaces_task_load_warnings_in_report() {
        let temp = tempdir().expect("tempdir should create");
        let task_dir = temp.path().join("eval-tasks");
        fs::create_dir_all(temp.path().join("projects")).expect("projects dir should create");
        fs::create_dir_all(task_dir.join("core")).expect("task dir should create");
        fs::write(
            task_dir.join("task-set.yaml"),
            "tasks:\n  - core/valid.yaml\n  - core/missing.yaml\n",
        )
        .expect("task set should write");
        fs::write(
            task_dir.join("core").join("valid.yaml"),
            r#"
task_id: valid
prompt: hello
expected_outcome:
  max_turns: 1
"#,
        )
        .expect("valid task should write");

        let addr = mock_server(temp.path().join("projects")).await;
        let report = EvalRunner::run(EvalRunnerConfig {
            server_url: format!("http://{addr}"),
            session_storage_root: temp.path().join("projects"),
            task_set: task_dir.join("task-set.yaml"),
            workspace_root: Some(temp.path().join("workspaces")),
            baseline: None,
            concurrency: 1,
            keep_workspace: false,
            output: None,
            timeout: Duration::from_secs(3),
            poll_interval: Duration::from_millis(20),
            auth_token: None,
        })
        .await
        .expect("runner should succeed with warnings");

        assert_eq!(report.results.len(), 1);
        assert_eq!(report.warnings.len(), 1);
        assert!(report.warnings[0].contains("missing.yaml"));
    }

    #[tokio::test]
    async fn runner_fails_fast_when_session_log_is_not_reachable_from_configured_root() {
        let temp = tempdir().expect("tempdir should create");
        let server_projects = temp.path().join("server-projects");
        let wrong_projects = temp.path().join("wrong-projects");
        let task_dir = temp.path().join("eval-tasks");
        fs::create_dir_all(&server_projects).expect("server projects dir should create");
        fs::create_dir_all(&wrong_projects).expect("wrong projects dir should create");
        fs::create_dir_all(task_dir.join("core")).expect("task dir should create");
        fs::write(
            task_dir.join("task-set.yaml"),
            "tasks:\n  - core/simple.yaml\n",
        )
        .expect("task set should write");
        fs::write(
            task_dir.join("core").join("simple.yaml"),
            r#"
task_id: simple
prompt: hello
expected_outcome:
  max_turns: 1
"#,
        )
        .expect("task file should write");

        let addr = mock_server(server_projects).await;
        let error = EvalRunner::run(EvalRunnerConfig {
            server_url: format!("http://{addr}"),
            session_storage_root: wrong_projects,
            task_set: task_dir.join("task-set.yaml"),
            workspace_root: Some(temp.path().join("workspaces")),
            baseline: None,
            concurrency: 1,
            keep_workspace: false,
            output: None,
            timeout: Duration::from_secs(3),
            poll_interval: Duration::from_millis(20),
            auth_token: None,
        })
        .await
        .expect_err("runner should fail fast on control/data plane mismatch");

        let message = error.to_string();
        assert!(message.contains("控制面/数据面不一致"));
        assert!(!message.contains("等待 turn"));
    }
}
