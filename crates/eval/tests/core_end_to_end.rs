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
}

#[derive(Clone)]
struct SessionFixture {
    working_dir: PathBuf,
    log_path: PathBuf,
}

#[derive(Clone)]
struct MockScenario {
    steps: Vec<MockStep>,
    final_output: &'static str,
}

#[derive(Clone)]
enum MockStep {
    Read {
        path: &'static str,
    },
    Edit {
        path: &'static str,
        content: &'static str,
    },
    Write {
        path: &'static str,
        content: &'static str,
    },
    ApplyPatch {
        path: &'static str,
        content: &'static str,
    },
    Grep {
        path: &'static str,
        pattern: &'static str,
        output: &'static str,
    },
    Glob {
        pattern: &'static str,
        output: &'static str,
    },
    ListDir {
        path: &'static str,
        output: &'static str,
    },
    FindFiles {
        query: &'static str,
        output: &'static str,
    },
    Shell {
        command: &'static str,
        output: &'static str,
        success: bool,
        error: Option<&'static str>,
    },
    ToolSearch {
        query: &'static str,
        output: &'static str,
    },
    Skill {
        name: &'static str,
        output: &'static str,
    },
    SpawnAgent {
        task: &'static str,
        output: &'static str,
    },
    SendToAgent {
        agent_id: &'static str,
        message: &'static str,
        output: &'static str,
    },
    ObserveAgent {
        agent_id: &'static str,
        output: &'static str,
    },
    CloseAgent {
        agent_id: &'static str,
        output: &'static str,
    },
    EnterPlanMode {
        goal: &'static str,
        output: &'static str,
    },
    ExitPlanMode {
        reason: &'static str,
        output: &'static str,
    },
    UpsertSessionPlan {
        title: &'static str,
        output: &'static str,
    },
    TodoWrite {
        items: &'static [&'static str],
        output: &'static str,
    },
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
    let project_bucket = project_dir_name(&working_dir);
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
    let task_id = task_id_from_working_dir(&fixture.working_dir).expect("task id should resolve");
    let scenario = scenario_for(&task_id).expect("scenario should exist");

    append_turn_events(
        &fixture.log_path,
        &turn_id,
        &prompt,
        &fixture.working_dir,
        &scenario,
    );

    (
        reqwest::StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "turnId": turn_id,
            "sessionId": session_id,
        })),
    )
}

fn task_id_from_working_dir(working_dir: &Path) -> Option<String> {
    let name = working_dir.file_name()?.to_str()?;
    let (task_id, suffix) = name.rsplit_once('-')?;
    if suffix.chars().all(|ch| ch.is_ascii_digit()) {
        Some(task_id.to_string())
    } else {
        None
    }
}

fn scenario_for(task_id: &str) -> Option<MockScenario> {
    Some(match task_id {
        "file-read-accuracy" => scenario(
            vec![MockStep::Read { path: "README.md" }],
            "项目名称是 Astrcode Eval，第一条要点是这是一个用于离线评测 Agent 行为的示例项目。",
        ),
        "file-edit-precision" => scenario(
            vec![
                MockStep::Read { path: "src/lib.rs" },
                MockStep::Edit {
                    path: "src/lib.rs",
                    content: "pub const DEFAULT_RETRY_COUNT: u32 = 5;\n",
                },
            ],
            "已将 DEFAULT_RETRY_COUNT 更新为 5。",
        ),
        "tool-chain-efficiency" => scenario(
            vec![
                MockStep::Read {
                    path: "docs/plan.md",
                },
                MockStep::Edit {
                    path: "status.txt",
                    content: "done\n",
                },
            ],
            "已完成读取计划并将 status.txt 更新为 done。",
        ),
        "prompt-direct-answer" => scenario(Vec::new(), "plan"),
        "multi-read-context-summary" => scenario(
            vec![
                MockStep::Read {
                    path: "docs/context.md",
                },
                MockStep::Read {
                    path: "docs/constraints.md",
                },
            ],
            "compact 需要保留最近 2 轮，执行前先跑 cargo test -p astrcode-eval。",
        ),
        "write-plan-checklist" => scenario(
            vec![
                MockStep::Read {
                    path: "docs/spec.md",
                },
                MockStep::Edit {
                    path: "plan.md",
                    content: "# Draft Plan\n\n- [ ] Verification\n- [ ] Rollback\n",
                },
            ],
            "已补齐 plan.md 的 Verification 与 Rollback 检查清单。",
        ),
        "compact-context-retention" => scenario(
            vec![MockStep::Read {
                path: "compact-summary.md",
            }],
            "数据库连接池大小是 16，不能改动的 API 路径是 /v1/chat。",
        ),
        "compact-followup-edit" => scenario(
            vec![
                MockStep::Read { path: "summary.md" },
                MockStep::Edit {
                    path: "notes.txt",
                    content: "保留约束：日志级别必须保持 info\n已完成事项：迁移脚本已经生成\n",
                },
            ],
            "已把保留约束和已完成事项写入 notes.txt，两行摘要均已保留。",
        ),
        "plan-review-readiness" => scenario(
            vec![MockStep::Read {
                path: "draft-plan.md",
            }],
            "它缺少 ## Verification，这个关键章节补齐后才适合退出 plan mode。",
        ),
        "tool-argument-discipline" => scenario(
            vec![MockStep::Read {
                path: "config/app.toml",
            }],
            "read_timeout_secs 的值是 45。",
        ),
        "write-bootstrap-config" => scenario(
            vec![MockStep::Write {
                path: "config/generated.json",
                content: "{\n  \"env\": \"test\",\n  \"port\": 4173\n}\n",
            }],
            "已创建 config/generated.json，环境是 test，端口是 4173。",
        ),
        "grep-auth-error" => scenario(
            vec![MockStep::Grep {
                path: "logs/app.log",
                pattern: "AUTH-",
                output: "logs/app.log:7:[error] code=AUTH-409 token expired\n",
            }],
            "日志里的认证错误码是 AUTH-409。",
        ),
        "glob-release-notes" => scenario(
            vec![MockStep::Glob {
                pattern: "notes/*.md",
                output: "notes/2026-03.md\nnotes/2026-04.md\n",
            }],
            "最新的发布说明文件是 notes/2026-04.md。",
        ),
        "shell-read-version" => scenario(
            vec![MockStep::Shell {
                command: "cargo --version",
                output: "cargo 1.91.0-nightly (8f3d4c2 2026-04-10)\n",
                success: true,
                error: None,
            }],
            "cargo 版本是 cargo 1.91.0-nightly。",
        ),
        "apply-patch-banner" => scenario(
            vec![MockStep::ApplyPatch {
                path: "src/banner.txt",
                content: "release-channel=stable\n",
            }],
            "已把 banner 中的发布通道改为 stable。",
        ),
        "grep-read-edit-timeout" => scenario(
            vec![
                MockStep::Grep {
                    path: "src/settings.ts",
                    pattern: "REQUEST_TIMEOUT_MS",
                    output: "src/settings.ts:1:export const REQUEST_TIMEOUT_MS = 3000;\n",
                },
                MockStep::Read {
                    path: "src/settings.ts",
                },
                MockStep::Edit {
                    path: "src/settings.ts",
                    content: "export const REQUEST_TIMEOUT_MS = 4500;\n",
                },
            ],
            "已把 REQUEST_TIMEOUT_MS 调整为 4500。",
        ),
        "glob-read-write-summary" => scenario(
            vec![
                MockStep::Glob {
                    pattern: "notes/*.md",
                    output: "notes/2026-03.md\nnotes/2026-04.md\n",
                },
                MockStep::Read {
                    path: "notes/2026-04.md",
                },
                MockStep::Write {
                    path: "summary.md",
                    content: "最新版本是 2026-04，重点是补齐评测基线。\n",
                },
            ],
            "已生成 summary.md，并写入 2026-04 版本摘要。",
        ),
        "listdir-read-edit-status" => scenario(
            vec![
                MockStep::ListDir {
                    path: "docs",
                    output: "docs/todo.md\n",
                },
                MockStep::Read {
                    path: "docs/todo.md",
                },
                MockStep::Edit {
                    path: "status.md",
                    content: "status: ready-for-review\n",
                },
            ],
            "已根据 docs/todo.md 把 status.md 更新为 ready-for-review。",
        ),
        "findfiles-read-write-migration" => scenario(
            vec![
                MockStep::FindFiles {
                    query: "migration-plan.md",
                    output: "nested/docs/migration-plan.md\n",
                },
                MockStep::Read {
                    path: "nested/docs/migration-plan.md",
                },
                MockStep::Write {
                    path: "ops/checklist.md",
                    content: "- [ ] backup\n- [ ] dry-run\n- [ ] rollout\n",
                },
            ],
            "已根据 migration plan 生成 ops/checklist.md。",
        ),
        "read-edit-shell-verify" => scenario(
            vec![
                MockStep::Read {
                    path: "config/app.env",
                },
                MockStep::Edit {
                    path: "status.txt",
                    content: "verified\n",
                },
                MockStep::Shell {
                    command: "cat status.txt",
                    output: "verified\n",
                    success: true,
                    error: None,
                },
            ],
            "配置已确认，status.txt 已写成 verified 并完成校验。",
        ),
        "bugfix-null-guard" => scenario(
            vec![
                MockStep::Read {
                    path: "logs/panic.log",
                },
                MockStep::Edit {
                    path: "src/lib.rs",
                    content: "pub fn render_name(name: Option<&str>) -> &'static str {\n    \
                              name.unwrap_or(\"unknown\")\n}\n",
                },
            ],
            "已补上空值保护，render_name 在 name 为空时返回 unknown。",
        ),
        "feature-flag-endpoint" => scenario(
            vec![
                MockStep::Read {
                    path: "specs/feature.md",
                },
                MockStep::Write {
                    path: "src/feature_flags.rs",
                    content: "pub fn register_feature_routes() {\n    // expose /api/features for \
                              eval fixtures\n}\n",
                },
                MockStep::Edit {
                    path: "src/router.rs",
                    content: "pub fn mount_router() {\n    register_feature_routes();\n}\n",
                },
            ],
            "已新增 feature flag 路由并挂到 router。",
        ),
        "code-review-leak-fix" => scenario(
            vec![
                MockStep::Read { path: "review.md" },
                MockStep::Grep {
                    path: "src/service.rs",
                    pattern: "unwrap\\(",
                    output: "src/service.rs:2:    token.unwrap();\n",
                },
                MockStep::Edit {
                    path: "src/service.rs",
                    content: "pub fn load_token(token: Option<&str>) -> Result<&str, &'static \
                              str> {\n    token.ok_or(\"missing token\")\n}\n",
                },
            ],
            "已按 review 建议去掉 unwrap，改成显式错误返回。",
        ),
        "project-bootstrap" => scenario(
            vec![MockStep::Write {
                path: "src/main.ts",
                content: "export const boot = () => 'astrcode-eval';\n",
            }],
            "已初始化最小项目入口 src/main.ts。",
        ),
        "compact-retain-api-contract" => scenario(
            vec![MockStep::Read {
                path: "compact-summary.md",
            }],
            "compact 之后仍需保留 /api/sessions 契约，而且请求超时上限保持 30 秒。",
        ),
        "compact-multi-hop-followup" => scenario(
            vec![
                MockStep::Read {
                    path: "compact-summary.md",
                },
                MockStep::Edit {
                    path: "handoff.md",
                    content: "保留约束：worker 数量上限仍是 2\n已完成事项：trace 提取器已经稳定\n",
                },
            ],
            "已把 compact 后仍需保留的约束和完成事项写入 handoff.md。",
        ),
        "compact-history-priority" => scenario(
            vec![
                MockStep::Read {
                    path: "summary-1.md",
                },
                MockStep::Read {
                    path: "summary-2.md",
                },
            ],
            "较早的不变量是必须保留 UTF-8 输出，最近决策是把并发上限固定为 2。",
        ),
        "plan-enter-skeleton" => scenario(
            vec![
                MockStep::EnterPlanMode {
                    goal: "整理 release checklist",
                    output: "entered plan mode\n",
                },
                MockStep::UpsertSessionPlan {
                    title: "整理 release checklist",
                    output: "1. 收集现状\n2. 补齐检查项\n3. 执行验证\n",
                },
            ],
            "已进入 plan mode，并生成 3 步 release checklist 计划。",
        ),
        "plan-revise-after-read" => scenario(
            vec![
                MockStep::Read {
                    path: "docs/spec.md",
                },
                MockStep::EnterPlanMode {
                    goal: "按规格修订 rollout 计划",
                    output: "entered plan mode\n",
                },
                MockStep::UpsertSessionPlan {
                    title: "按规格修订 rollout 计划",
                    output: "1. 校对 SLA\n2. 补齐 Verification\n3. 标记 Rollback\n",
                },
            ],
            "我已按 docs/spec.md 修订计划，新增 Verification 与 Rollback 步骤。",
        ),
        "plan-exit-after-verification" => scenario(
            vec![
                MockStep::Read {
                    path: "draft-plan.md",
                },
                MockStep::ExitPlanMode {
                    reason: "verification 已完整",
                    output: "exit plan mode\n",
                },
            ],
            "draft-plan.md 已包含 Verification，可以退出 plan mode。",
        ),
        "plan-track-progress" => scenario(
            vec![
                MockStep::EnterPlanMode {
                    goal: "跟踪 eval 扩容执行",
                    output: "entered plan mode\n",
                },
                MockStep::UpsertSessionPlan {
                    title: "扩容 eval 任务",
                    output: "1. 补 YAML\n2. 补 fixtures\n3. 跑回归\n",
                },
                MockStep::TodoWrite {
                    items: &["补 YAML", "补 fixtures", "跑回归"],
                    output: "3 todos written\n",
                },
            ],
            "计划已同步到 todo，当前共有 3 个待办，下一步是补 fixtures。",
        ),
        "subagent-single-task" => scenario(
            vec![
                MockStep::SpawnAgent {
                    task: "总结 docs/brief.md",
                    output: "agent=agent-1\n",
                },
                MockStep::ObserveAgent {
                    agent_id: "agent-1",
                    output: "summary: 需要补 UI 冒烟\n",
                },
                MockStep::CloseAgent {
                    agent_id: "agent-1",
                    output: "closed\n",
                },
            ],
            "子智能体已完成独立总结，结论是需要补 UI 冒烟。",
        ),
        "subagent-parent-uses-result" => scenario(
            vec![
                MockStep::SpawnAgent {
                    task: "提取 module-a 要点",
                    output: "agent=agent-2\n",
                },
                MockStep::SendToAgent {
                    agent_id: "agent-2",
                    message: "只读 module-a.md 并返回一句摘要",
                    output: "sent\n",
                },
                MockStep::ObserveAgent {
                    agent_id: "agent-2",
                    output: "summary: module-a 负责 token 刷新\n",
                },
                MockStep::CloseAgent {
                    agent_id: "agent-2",
                    output: "closed\n",
                },
            ],
            "我已引用子智能体结果：module-a 负责 token 刷新。",
        ),
        "subagent-recovery-after-error" => scenario(
            vec![
                MockStep::SpawnAgent {
                    task: "检查 flaky case",
                    output: "agent=agent-3\n",
                },
                MockStep::ObserveAgent {
                    agent_id: "agent-3",
                    output: "error: missing fixture\n",
                },
                MockStep::SendToAgent {
                    agent_id: "agent-3",
                    message: "改读 fallback fixture 再重试",
                    output: "resent\n",
                },
                MockStep::ObserveAgent {
                    agent_id: "agent-3",
                    output: "summary: fallback fixture 可用\n",
                },
                MockStep::CloseAgent {
                    agent_id: "agent-3",
                    output: "closed\n",
                },
            ],
            "子智能体首次失败后已恢复，fallback fixture 可用。",
        ),
        "missing-file-findfiles-fallback" => scenario(
            vec![
                MockStep::FindFiles {
                    query: "target.md",
                    output: "docs/archive/target.md\n",
                },
                MockStep::Read {
                    path: "docs/archive/target.md",
                },
            ],
            "原路径不存在，但我通过 FindFiles 找到了 docs/archive/target.md。",
        ),
        "shell-failure-then-grep-log" => scenario(
            vec![
                MockStep::Shell {
                    command: "npm run build",
                    output: "command failed\n",
                    success: false,
                    error: Some("exit code 1"),
                },
                MockStep::Grep {
                    path: "logs/build.log",
                    pattern: "EADDRINUSE",
                    output: "logs/build.log:3:EADDRINUSE 127.0.0.1:3000\n",
                },
            ],
            "构建失败的根因是端口占用：EADDRINUSE 127.0.0.1:3000。",
        ),
        "toolsearch-skill-fallback" => scenario(
            vec![
                MockStep::ToolSearch {
                    query: "playwright browser automation",
                    output: "playwright\n",
                },
                MockStep::Skill {
                    name: "playwright",
                    output: "loaded skill: playwright\n",
                },
            ],
            "已通过 ToolSearch 选到 playwright，并切换到对应 skill。",
        ),
        "prompt-multi-request-chinese" => scenario(
            vec![
                MockStep::Read { path: "README.md" },
                MockStep::Write {
                    path: "answer.md",
                    content: "项目名称：Astrcode Eval\n待办：补 30+ 个复杂场景用例\n",
                },
            ],
            "我已完成两个请求：项目名称是 Astrcode Eval，并把待办摘要写入 answer.md。",
        ),
        "prompt-refuse-system-delete" => scenario(
            Vec::new(),
            "我不能删除系统文件或执行破坏性请求；如果你要清理项目目录，\
             我可以改成只删除仓库内的临时产物。",
        ),
        "prompt-markdown-format" => scenario(
            Vec::new(),
            "## 检查结果\n\n- 已确认评测基线存在\n- 下一步应补 fixtures\n\n```text\nstatus: \
             ready\n```",
        ),
        "large-file-targeted-read" => scenario(
            vec![MockStep::Read {
                path: "docs/large.txt",
            }],
            "大文件里标记的关键值是 retention_window=96。",
        ),
        "empty-dir-safe-response" => scenario(
            vec![MockStep::ListDir {
                path: "empty",
                output: "",
            }],
            "empty 目录当前没有文件。",
        ),
        "binary-file-skip" => scenario(
            vec![MockStep::Read {
                path: "assets/logo.bin",
            }],
            "这是一个二进制占位文件，不适合直接按文本编辑。",
        ),
        _ => return None,
    })
}

fn scenario(steps: Vec<MockStep>, final_output: &'static str) -> MockScenario {
    MockScenario {
        steps,
        final_output,
    }
}

fn append_turn_events(
    log_path: &Path,
    turn_id: &str,
    prompt: &str,
    working_dir: &Path,
    scenario: &MockScenario,
) {
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

    for (index, step) in scenario.steps.iter().enumerate() {
        let tool_call_id = format!("call-{}", index + 1);
        events.push(tool_call_event(
            next_seq,
            turn_id,
            &agent,
            &tool_call_id,
            step.tool_name(),
            step.args(),
        ));
        next_seq += 1;

        let result = step.execute(working_dir);
        events.push(tool_result_event(
            next_seq,
            turn_id,
            &agent,
            ToolResultEventArgs {
                tool_call_id: &tool_call_id,
                tool_name: step.tool_name(),
                output: &result.output,
                success: result.success,
                error: result.error.as_deref(),
                duration_ms: 8 + index as u64 * 3,
            },
        ));
        next_seq += 1;
    }

    events.push(StoredEvent {
        storage_seq: next_seq,
        event: StorageEvent {
            turn_id: Some(turn_id.to_string()),
            agent: agent.clone(),
            payload: StorageEventPayload::AssistantFinal {
                content: scenario.final_output.to_string(),
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

struct StepResult {
    output: String,
    success: bool,
    error: Option<String>,
}

impl MockStep {
    fn tool_name(&self) -> &'static str {
        match self {
            MockStep::Read { .. } => "Read",
            MockStep::Edit { .. } => "Edit",
            MockStep::Write { .. } => "Write",
            MockStep::ApplyPatch { .. } => "ApplyPatch",
            MockStep::Grep { .. } => "Grep",
            MockStep::Glob { .. } => "Glob",
            MockStep::ListDir { .. } => "ListDir",
            MockStep::FindFiles { .. } => "FindFiles",
            MockStep::Shell { .. } => "Shell",
            MockStep::ToolSearch { .. } => "ToolSearch",
            MockStep::Skill { .. } => "Skill",
            MockStep::SpawnAgent { .. } => "SpawnAgent",
            MockStep::SendToAgent { .. } => "SendToAgent",
            MockStep::ObserveAgent { .. } => "ObserveAgent",
            MockStep::CloseAgent { .. } => "CloseAgent",
            MockStep::EnterPlanMode { .. } => "EnterPlanMode",
            MockStep::ExitPlanMode { .. } => "ExitPlanMode",
            MockStep::UpsertSessionPlan { .. } => "UpsertSessionPlan",
            MockStep::TodoWrite { .. } => "TodoWrite",
        }
    }

    fn args(&self) -> serde_json::Value {
        match self {
            MockStep::Read { path }
            | MockStep::Edit { path, .. }
            | MockStep::Write { path, .. }
            | MockStep::ApplyPatch { path, .. } => serde_json::json!({ "path": path }),
            MockStep::Grep { path, pattern, .. } => {
                serde_json::json!({ "path": path, "pattern": pattern })
            },
            MockStep::Glob { pattern, .. } => serde_json::json!({ "pattern": pattern }),
            MockStep::ListDir { path, .. } => serde_json::json!({ "path": path }),
            MockStep::FindFiles { query, .. } => serde_json::json!({ "query": query }),
            MockStep::Shell { command, .. } => serde_json::json!({ "command": command }),
            MockStep::ToolSearch { query, .. } => serde_json::json!({ "query": query }),
            MockStep::Skill { name, .. } => serde_json::json!({ "name": name }),
            MockStep::SpawnAgent { task, .. } => serde_json::json!({ "task": task }),
            MockStep::SendToAgent {
                agent_id, message, ..
            } => serde_json::json!({ "agentId": agent_id, "message": message }),
            MockStep::ObserveAgent { agent_id, .. } | MockStep::CloseAgent { agent_id, .. } => {
                serde_json::json!({ "agentId": agent_id })
            },
            MockStep::EnterPlanMode { goal, .. } => serde_json::json!({ "goal": goal }),
            MockStep::ExitPlanMode { reason, .. } => serde_json::json!({ "reason": reason }),
            MockStep::UpsertSessionPlan { title, .. } => serde_json::json!({ "title": title }),
            MockStep::TodoWrite { items, .. } => serde_json::json!({ "items": items }),
        }
    }

    fn execute(&self, working_dir: &Path) -> StepResult {
        match self {
            MockStep::Read { path } => StepResult {
                output: read_workspace_file(working_dir, path),
                success: true,
                error: None,
            },
            MockStep::Edit { path, content }
            | MockStep::Write { path, content }
            | MockStep::ApplyPatch { path, content } => {
                write_workspace_file(working_dir, path, content);
                StepResult {
                    output: (*content).to_string(),
                    success: true,
                    error: None,
                }
            },
            MockStep::Grep { output, .. }
            | MockStep::Glob { output, .. }
            | MockStep::ListDir { output, .. }
            | MockStep::FindFiles { output, .. }
            | MockStep::ToolSearch { output, .. }
            | MockStep::Skill { output, .. }
            | MockStep::SpawnAgent { output, .. }
            | MockStep::SendToAgent { output, .. }
            | MockStep::ObserveAgent { output, .. }
            | MockStep::CloseAgent { output, .. }
            | MockStep::EnterPlanMode { output, .. }
            | MockStep::ExitPlanMode { output, .. }
            | MockStep::UpsertSessionPlan { output, .. }
            | MockStep::TodoWrite { output, .. } => StepResult {
                output: (*output).to_string(),
                success: true,
                error: None,
            },
            MockStep::Shell {
                output,
                success,
                error,
                ..
            } => StepResult {
                output: (*output).to_string(),
                success: *success,
                error: error.map(|item| item.to_string()),
            },
        }
    }
}

fn read_workspace_file(working_dir: &Path, relative_path: &str) -> String {
    fs::read_to_string(working_dir.join(relative_path)).expect("workspace file should read")
}

fn write_workspace_file(working_dir: &Path, relative_path: &str, content: &str) {
    let path = working_dir.join(relative_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent dir should create");
    }
    fs::write(path, content).expect("workspace file should write");
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

struct ToolResultEventArgs<'a> {
    tool_call_id: &'a str,
    tool_name: &'a str,
    output: &'a str,
    success: bool,
    error: Option<&'a str>,
    duration_ms: u64,
}

fn tool_result_event(
    storage_seq: u64,
    turn_id: &str,
    agent: &AgentEventContext,
    args: ToolResultEventArgs<'_>,
) -> StoredEvent {
    StoredEvent {
        storage_seq,
        event: StorageEvent {
            turn_id: Some(turn_id.to_string()),
            agent: agent.clone(),
            payload: StorageEventPayload::ToolResult {
                tool_call_id: args.tool_call_id.to_string(),
                tool_name: args.tool_name.to_string(),
                output: args.output.to_string(),
                success: args.success,
                error: args.error.map(|item| item.to_string()),
                metadata: None,
                continuation: None,
                duration_ms: args.duration_ms,
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

    assert_eq!(report.results.len(), 43);
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
    assert_eq!(baseline.diffs.len(), 43);
    assert!(baseline.diffs.iter().all(|diff| diff.score_delta == 0.0
        && diff.tool_calls_delta == 0
        && diff.duration_ms_delta == 0
        && diff.estimated_tokens_delta == 0
        && !diff.regression));
}
