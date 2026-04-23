//! 终端查询子域集成测试。
//!
//! 验证终端查询在完整应用栈上的端到端行为，使用真实的 `App` 组装
//! （而非 mock），覆盖：
//! - 会话恢复候选列表过滤
//! - 快照查询与游标比较
//! - 终端摘要提取

use std::{path::Path, sync::Arc, time::Duration};

use astrcode_core::{AgentEvent, ExecutionTaskItem, ExecutionTaskStatus, TaskSnapshot};
use astrcode_session_runtime::{SessionControlStateSnapshot, SessionRuntime};
use async_trait::async_trait;
use tokio::time::timeout;

use crate::{
    App, AppKernelPort, AppSessionPort, ApplicationError, ComposerResolvedSkill, ComposerSkillPort,
    ConfigService, McpConfigScope, McpPort, McpServerStatusView, McpService,
    ProfileResolutionService,
    agent::{
        AgentOrchestrationService,
        test_support::{TestLlmBehavior, build_agent_test_harness},
    },
    composer::ComposerSkillSummary,
    mcp::RegisterMcpServerInput,
    terminal::{
        ConversationBlockFacts, ConversationFocus, TerminalRehydrateReason, TerminalStreamFacts,
    },
    test_support::StubSessionPort,
};

struct StaticComposerSkillPort {
    summaries: Vec<ComposerSkillSummary>,
}

impl ComposerSkillPort for StaticComposerSkillPort {
    fn list_skill_summaries(&self, _working_dir: &Path) -> Vec<ComposerSkillSummary> {
        self.summaries.clone()
    }

    fn resolve_skill(&self, _working_dir: &Path, skill_id: &str) -> Option<ComposerResolvedSkill> {
        self.summaries
            .iter()
            .find(|summary| summary.id == skill_id)
            .map(|summary| ComposerResolvedSkill {
                id: summary.id.clone(),
                description: summary.description.clone(),
                guide: format!("guide for {}", summary.id),
            })
    }
}

struct NoopMcpPort;

#[async_trait]
impl McpPort for NoopMcpPort {
    async fn list_server_status(&self) -> Vec<McpServerStatusView> {
        Vec::new()
    }

    async fn approve_server(&self, _server_signature: &str) -> Result<(), ApplicationError> {
        Ok(())
    }

    async fn reject_server(&self, _server_signature: &str) -> Result<(), ApplicationError> {
        Ok(())
    }

    async fn reconnect_server(&self, _name: &str) -> Result<(), ApplicationError> {
        Ok(())
    }

    async fn reset_project_choices(&self) -> Result<(), ApplicationError> {
        Ok(())
    }

    async fn upsert_server(&self, _input: &RegisterMcpServerInput) -> Result<(), ApplicationError> {
        Ok(())
    }

    async fn remove_server(
        &self,
        _scope: McpConfigScope,
        _name: &str,
    ) -> Result<(), ApplicationError> {
        Ok(())
    }

    async fn set_server_enabled(
        &self,
        _scope: McpConfigScope,
        _name: &str,
        _enabled: bool,
    ) -> Result<(), ApplicationError> {
        Ok(())
    }
}

struct TerminalAppHarness {
    app: App,
    session_runtime: Arc<SessionRuntime>,
}

fn build_terminal_app_harness(skill_ids: &[&str]) -> TerminalAppHarness {
    build_terminal_app_harness_with_behavior(
        skill_ids,
        TestLlmBehavior::Succeed {
            content: "子代理已完成。".to_string(),
        },
    )
}

fn build_terminal_app_harness_with_behavior(
    skill_ids: &[&str],
    llm_behavior: TestLlmBehavior,
) -> TerminalAppHarness {
    let harness = build_agent_test_harness(llm_behavior).expect("agent test harness should build");
    let kernel: Arc<dyn AppKernelPort> = harness.kernel.clone();
    let session_runtime = harness.session_runtime.clone();
    let session_port: Arc<dyn AppSessionPort> = session_runtime.clone();
    let app = build_terminal_app(
        kernel,
        session_port,
        harness.config_service.clone(),
        harness.profiles.clone(),
        Arc::new(StaticComposerSkillPort {
            summaries: skill_ids
                .iter()
                .map(|id| ComposerSkillSummary::new(*id, format!("{id} description")))
                .collect(),
        }),
        Arc::new(harness.service.clone()),
    );
    TerminalAppHarness {
        app,
        session_runtime,
    }
}

fn build_terminal_app(
    kernel: Arc<dyn AppKernelPort>,
    session_port: Arc<dyn AppSessionPort>,
    config: Arc<ConfigService>,
    profiles: Arc<ProfileResolutionService>,
    composer_skills: Arc<dyn ComposerSkillPort>,
    agent_service: Arc<AgentOrchestrationService>,
) -> App {
    let mcp_service = Arc::new(McpService::new(Arc::new(NoopMcpPort)));
    App::new(
        kernel,
        session_port,
        profiles,
        config,
        composer_skills,
        Arc::new(crate::governance_surface::GovernanceSurfaceAssembler::default()),
        Arc::new(crate::mode::builtin_mode_catalog().expect("builtin mode catalog should build")),
        mcp_service,
        agent_service,
    )
}

#[tokio::test]
async fn terminal_stream_facts_expose_live_llm_deltas_before_durable_completion() {
    let harness = build_terminal_app_harness_with_behavior(
        &[],
        TestLlmBehavior::Stream {
            reasoning_chunks: vec!["先".to_string(), "整理".to_string()],
            text_chunks: vec!["流".to_string(), "式".to_string()],
            final_content: "流式完成".to_string(),
            final_reasoning: Some("先整理".to_string()),
        },
    );
    let project = tempfile::tempdir().expect("tempdir should be created");
    let session = harness
        .app
        .create_session(project.path().display().to_string())
        .await
        .expect("session should be created");

    let TerminalStreamFacts::Replay(replay) = harness
        .app
        .terminal_stream_facts(&session.session_id, None)
        .await
        .expect("stream facts should build")
    else {
        panic!("fresh stream should start from replay facts");
    };
    let mut live_receiver = replay.stream.live_receiver;

    let accepted = harness
        .app
        .submit_prompt(&session.session_id, "请流式回答".to_string())
        .await
        .expect("prompt should submit");

    let mut live_events = Vec::new();
    for _ in 0..4 {
        live_events.push(
            timeout(Duration::from_secs(1), live_receiver.recv())
                .await
                .expect("live delta should arrive in time")
                .expect("live receiver should stay open"),
        );
    }

    assert!(matches!(
        &live_events[0],
        AgentEvent::ThinkingDelta { delta, .. } if delta == "先"
    ));
    assert!(matches!(
        &live_events[1],
        AgentEvent::ThinkingDelta { delta, .. } if delta == "整理"
    ));
    assert!(matches!(
        &live_events[2],
        AgentEvent::ModelDelta { delta, .. } if delta == "流"
    ));
    assert!(matches!(
        &live_events[3],
        AgentEvent::ModelDelta { delta, .. } if delta == "式"
    ));

    harness
        .session_runtime
        .wait_for_turn_terminal_snapshot(&session.session_id, accepted.turn_id.as_str())
        .await
        .expect("turn should settle");

    let snapshot = harness
        .app
        .terminal_snapshot_facts(&session.session_id)
        .await
        .expect("terminal snapshot should build");
    assert!(snapshot.transcript.blocks.iter().any(|block| matches!(
        block,
        ConversationBlockFacts::Assistant(block) if block.markdown == "流式完成"
    )));
    assert!(snapshot.transcript.blocks.iter().any(|block| matches!(
        block,
        ConversationBlockFacts::Thinking(block) if block.markdown == "先整理"
    )));
}

#[tokio::test]
async fn terminal_snapshot_facts_hydrate_history_control_and_slash_candidates() {
    let harness = build_terminal_app_harness(&["openspec-apply-change"]);
    let project = tempfile::tempdir().expect("tempdir should be created");
    let session = harness
        .app
        .create_session(project.path().display().to_string())
        .await
        .expect("session should be created");
    harness
        .app
        .submit_prompt(&session.session_id, "请总结当前仓库".to_string())
        .await
        .expect("prompt should submit");

    let facts = harness
        .app
        .terminal_snapshot_facts(&session.session_id)
        .await
        .expect("terminal snapshot should build");

    assert_eq!(facts.active_session_id, session.session_id);
    assert!(!facts.transcript.blocks.is_empty());
    assert!(facts.transcript.cursor.is_some());
    assert!(
        facts
            .slash_candidates
            .iter()
            .any(|candidate| candidate.id == "new")
    );
    assert!(
        facts
            .slash_candidates
            .iter()
            .any(|candidate| candidate.id == "resume")
    );
    assert!(
        facts
            .slash_candidates
            .iter()
            .any(|candidate| candidate.id == "compact")
    );
    assert!(
        facts
            .slash_candidates
            .iter()
            .any(|candidate| candidate.id == "openspec-apply-change")
    );
    assert!(
        facts
            .slash_candidates
            .iter()
            .all(|candidate| candidate.id != "skill")
    );
}

#[tokio::test]
async fn terminal_stream_facts_returns_replay_for_valid_cursor() {
    let harness = build_terminal_app_harness(&[]);
    let project = tempfile::tempdir().expect("tempdir should be created");
    let session = harness
        .app
        .create_session(project.path().display().to_string())
        .await
        .expect("session should be created");
    harness
        .app
        .submit_prompt(&session.session_id, "hello".to_string())
        .await
        .expect("prompt should submit");
    let snapshot = harness
        .app
        .terminal_snapshot_facts(&session.session_id)
        .await
        .expect("snapshot should build");
    let cursor = snapshot.transcript.cursor.clone();

    let facts = harness
        .app
        .terminal_stream_facts(&session.session_id, cursor.as_deref())
        .await
        .expect("stream facts should build");

    match facts {
        TerminalStreamFacts::Replay(replay) => {
            assert_eq!(replay.active_session_id, session.session_id);
            assert!(replay.replay.history.is_empty());
            assert!(replay.replay.replay_frames.is_empty());
            assert_eq!(
                replay
                    .replay
                    .seed_records
                    .last()
                    .map(|record| record.event_id.as_str()),
                snapshot.transcript.cursor.as_deref()
            );
        },
        TerminalStreamFacts::RehydrateRequired(_) => {
            panic!("valid cursor should not require rehydrate");
        },
    }
}

#[tokio::test]
async fn terminal_stream_facts_falls_back_to_rehydrate_for_future_cursor() {
    let harness = build_terminal_app_harness(&[]);
    let project = tempfile::tempdir().expect("tempdir should be created");
    let session = harness
        .app
        .create_session(project.path().display().to_string())
        .await
        .expect("session should be created");
    harness
        .app
        .submit_prompt(&session.session_id, "hello".to_string())
        .await
        .expect("prompt should submit");

    let facts = harness
        .app
        .terminal_stream_facts(&session.session_id, Some("999999.9"))
        .await
        .expect("stream facts should build");

    match facts {
        TerminalStreamFacts::Replay(_) => {
            panic!("future cursor should require rehydrate");
        },
        TerminalStreamFacts::RehydrateRequired(rehydrate) => {
            assert_eq!(rehydrate.reason, TerminalRehydrateReason::CursorExpired);
            assert_eq!(rehydrate.requested_cursor, "999999.9");
            assert!(rehydrate.latest_cursor.is_some());
        },
    }
}

#[tokio::test]
async fn terminal_stream_facts_rehydrates_when_cursor_is_missing_from_transcript() {
    let harness = build_terminal_app_harness(&[]);
    let project = tempfile::tempdir().expect("tempdir should be created");
    let session = harness
        .app
        .create_session(project.path().display().to_string())
        .await
        .expect("session should be created");
    harness
        .app
        .submit_prompt(&session.session_id, "hello".to_string())
        .await
        .expect("prompt should submit");

    let transcript = harness
        .session_runtime
        .session_transcript_snapshot(&session.session_id)
        .await
        .expect("transcript snapshot should build");
    let candidate = transcript
        .records
        .iter()
        .find_map(|record| {
            let (storage_seq, subindex) = record.event_id.split_once('.')?;
            let subindex = subindex.parse::<u32>().ok()?;
            Some(format!("{storage_seq}.{}", subindex.saturating_add(1)))
        })
        .expect("session should produce at least one durable cursor");

    let facts = harness
        .app
        .terminal_stream_facts(&session.session_id, Some(candidate.as_str()))
        .await
        .expect("stream facts should build");

    match facts {
        TerminalStreamFacts::Replay(_) => {
            panic!("missing transcript cursor should require rehydrate");
        },
        TerminalStreamFacts::RehydrateRequired(rehydrate) => {
            assert_eq!(rehydrate.reason, TerminalRehydrateReason::CursorExpired);
            assert_eq!(rehydrate.requested_cursor, candidate);
            assert_eq!(rehydrate.latest_cursor, transcript.cursor);
        },
    }
}

#[tokio::test]
async fn terminal_resume_candidates_use_server_fact_and_recent_sorting() {
    let harness = build_terminal_app_harness(&[]);
    let project = tempfile::tempdir().expect("tempdir should be created");
    let older_dir = project.path().join("older");
    let newer_dir = project.path().join("newer");
    std::fs::create_dir_all(&older_dir).expect("older dir should exist");
    std::fs::create_dir_all(&newer_dir).expect("newer dir should exist");
    let older = harness
        .app
        .create_session(older_dir.display().to_string())
        .await
        .expect("older session should be created");
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let newer = harness
        .app
        .create_session(newer_dir.display().to_string())
        .await
        .expect("newer session should be created");

    let candidates = harness
        .app
        .terminal_resume_candidates(Some("newer"), 20)
        .await
        .expect("resume candidates should build");

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].session_id, newer.session_id);
    let all_candidates = harness
        .app
        .terminal_resume_candidates(None, 20)
        .await
        .expect("resume candidates should build");
    assert_eq!(all_candidates[0].session_id, newer.session_id);
    assert_eq!(all_candidates[1].session_id, older.session_id);
}

#[tokio::test]
async fn terminal_child_summaries_only_return_direct_visible_children() {
    let harness = build_terminal_app_harness(&[]);
    let project = tempfile::tempdir().expect("tempdir should be created");
    let parent_dir = project.path().join("parent");
    let child_dir = project.path().join("child");
    let unrelated_dir = project.path().join("unrelated");
    std::fs::create_dir_all(&parent_dir).expect("parent dir should exist");
    std::fs::create_dir_all(&child_dir).expect("child dir should exist");
    std::fs::create_dir_all(&unrelated_dir).expect("unrelated dir should exist");
    let parent = harness
        .session_runtime
        .create_session(parent_dir.display().to_string())
        .await
        .expect("parent session should be created");
    let child = harness
        .session_runtime
        .create_session(child_dir.display().to_string())
        .await
        .expect("child session should be created");
    let unrelated = harness
        .session_runtime
        .create_session(unrelated_dir.display().to_string())
        .await
        .expect("unrelated session should be created");

    let root = harness
        .app
        .ensure_session_root_agent_context(&parent.session_id)
        .await
        .expect("root context should exist");

    harness
        .session_runtime
        .append_child_session_notification(
            &parent.session_id,
            "turn-parent",
            root.clone(),
            astrcode_core::ChildSessionNotification {
                notification_id: "child-1".to_string().into(),
                child_ref: astrcode_core::ChildAgentRef {
                    identity: astrcode_core::ChildExecutionIdentity {
                        agent_id: "agent-child".to_string().into(),
                        session_id: parent.session_id.clone().into(),
                        sub_run_id: "subrun-child".to_string().into(),
                    },
                    parent: astrcode_core::ParentExecutionRef {
                        parent_agent_id: root.agent_id.clone(),
                        parent_sub_run_id: None,
                    },
                    lineage_kind: astrcode_core::ChildSessionLineageKind::Spawn,
                    status: astrcode_core::AgentLifecycleStatus::Running,
                    open_session_id: child.session_id.clone().into(),
                },
                kind: astrcode_core::ChildSessionNotificationKind::Started,
                source_tool_call_id: Some("tool-call-1".to_string().into()),
                delivery: Some(astrcode_core::ParentDelivery {
                    idempotency_key: "child-1".to_string(),
                    origin: astrcode_core::ParentDeliveryOrigin::Explicit,
                    terminal_semantics: astrcode_core::ParentDeliveryTerminalSemantics::NonTerminal,
                    source_turn_id: Some("turn-child".to_string()),
                    payload: astrcode_core::ParentDeliveryPayload::Progress(
                        astrcode_core::ProgressParentDeliveryPayload {
                            message: "child progress".to_string(),
                        },
                    ),
                }),
            },
        )
        .await
        .expect("child notification should append");

    let accepted = harness
        .app
        .submit_prompt(&child.session_id, "child output".to_string())
        .await
        .expect("child prompt should submit");
    harness
        .session_runtime
        .wait_for_turn_terminal_snapshot(&child.session_id, accepted.turn_id.as_str())
        .await
        .expect("child turn should settle");
    harness
        .app
        .submit_prompt(&unrelated.session_id, "ignore me".to_string())
        .await
        .expect("unrelated prompt should submit");

    let children = harness
        .app
        .terminal_child_summaries(&parent.session_id)
        .await
        .expect("child summaries should build");

    assert_eq!(children.len(), 1);
    assert_eq!(children[0].node.child_session_id, child.session_id.into());
    assert!(
        children[0]
            .recent_output
            .as_deref()
            .is_some_and(|summary| summary.contains("子代理已完成"))
    );
}

#[tokio::test]
async fn conversation_focus_snapshot_reads_child_session_transcript() {
    let harness = build_terminal_app_harness(&[]);
    let project = tempfile::tempdir().expect("tempdir should be created");
    let parent_dir = project.path().join("parent");
    let child_dir = project.path().join("child");
    std::fs::create_dir_all(&parent_dir).expect("parent dir should exist");
    std::fs::create_dir_all(&child_dir).expect("child dir should exist");
    let parent = harness
        .session_runtime
        .create_session(parent_dir.display().to_string())
        .await
        .expect("parent session should be created");
    let child = harness
        .session_runtime
        .create_session(child_dir.display().to_string())
        .await
        .expect("child session should be created");
    let root = harness
        .app
        .ensure_session_root_agent_context(&parent.session_id)
        .await
        .expect("root context should exist");

    harness
        .session_runtime
        .append_child_session_notification(
            &parent.session_id,
            "turn-parent",
            root.clone(),
            astrcode_core::ChildSessionNotification {
                notification_id: "child-1".to_string().into(),
                child_ref: astrcode_core::ChildAgentRef {
                    identity: astrcode_core::ChildExecutionIdentity {
                        agent_id: "agent-child".to_string().into(),
                        session_id: parent.session_id.clone().into(),
                        sub_run_id: "subrun-child".to_string().into(),
                    },
                    parent: astrcode_core::ParentExecutionRef {
                        parent_agent_id: root.agent_id.clone(),
                        parent_sub_run_id: None,
                    },
                    lineage_kind: astrcode_core::ChildSessionLineageKind::Spawn,
                    status: astrcode_core::AgentLifecycleStatus::Running,
                    open_session_id: child.session_id.clone().into(),
                },
                kind: astrcode_core::ChildSessionNotificationKind::Started,
                source_tool_call_id: Some("tool-call-1".to_string().into()),
                delivery: Some(astrcode_core::ParentDelivery {
                    idempotency_key: "child-1".to_string(),
                    origin: astrcode_core::ParentDeliveryOrigin::Explicit,
                    terminal_semantics: astrcode_core::ParentDeliveryTerminalSemantics::NonTerminal,
                    source_turn_id: Some("turn-child".to_string()),
                    payload: astrcode_core::ParentDeliveryPayload::Progress(
                        astrcode_core::ProgressParentDeliveryPayload {
                            message: "child progress".to_string(),
                        },
                    ),
                }),
            },
        )
        .await
        .expect("child notification should append");

    harness
        .app
        .submit_prompt(&parent.session_id, "parent prompt".to_string())
        .await
        .expect("parent prompt should submit");
    harness
        .app
        .submit_prompt(&child.session_id, "child prompt".to_string())
        .await
        .expect("child prompt should submit");

    let facts = harness
        .app
        .conversation_snapshot_facts(
            &parent.session_id,
            ConversationFocus::SubRun {
                sub_run_id: "subrun-child".to_string(),
            },
        )
        .await
        .expect("conversation focus snapshot should build");

    assert_eq!(facts.active_session_id, parent.session_id);
    assert!(facts.transcript.blocks.iter().any(|block| matches!(
        block,
        ConversationBlockFacts::User(block) if block.markdown == "child prompt"
    )));
    assert!(facts.transcript.blocks.iter().all(|block| !matches!(
        block,
        ConversationBlockFacts::User(block) if block.markdown == "parent prompt"
    )));
    assert!(facts.child_summaries.is_empty());
}

#[test]
fn cursor_is_after_head_treats_equal_cursor_as_caught_up() {
    assert!(
        !super::cursor::cursor_is_after_head("12.3", Some("12.3"))
            .expect("equal cursor should parse")
    );
    assert!(
        super::cursor::cursor_is_after_head("12.4", Some("12.3"))
            .expect("newer cursor should parse")
    );
    assert!(
        !super::cursor::cursor_is_after_head("12.2", Some("12.3"))
            .expect("older cursor should parse")
    );
}

#[tokio::test]
async fn terminal_control_facts_include_authoritative_active_tasks() {
    let harness = build_agent_test_harness(TestLlmBehavior::Succeed {
        content: "unused".to_string(),
    })
    .expect("agent test harness should build");
    let project = tempfile::tempdir().expect("tempdir should be created");
    let session_port: Arc<dyn AppSessionPort> = Arc::new(StubSessionPort {
        working_dir: Some(project.path().display().to_string()),
        control_state: Some(SessionControlStateSnapshot {
            phase: astrcode_core::Phase::Idle,
            active_turn_id: Some("turn-1".to_string()),
            manual_compact_pending: false,
            compacting: false,
            last_compact_meta: None,
            current_mode_id: astrcode_core::ModeId::code(),
            last_mode_changed_at: None,
        }),
        active_task_snapshot: Arc::new(std::sync::Mutex::new(Some(TaskSnapshot {
            owner: astrcode_session_runtime::ROOT_AGENT_ID.to_string(),
            items: vec![
                ExecutionTaskItem {
                    content: "实现 authoritative task panel".to_string(),
                    status: ExecutionTaskStatus::InProgress,
                    active_form: Some("正在实现 authoritative task panel".to_string()),
                },
                ExecutionTaskItem {
                    content: "补充前端 hydration 测试".to_string(),
                    status: ExecutionTaskStatus::Pending,
                    active_form: None,
                },
            ],
        }))),
        ..StubSessionPort::default()
    });
    let app = build_terminal_app(
        harness.kernel.clone(),
        session_port,
        harness.config_service.clone(),
        harness.profiles.clone(),
        Arc::new(StaticComposerSkillPort {
            summaries: Vec::new(),
        }),
        Arc::new(harness.service.clone()),
    );

    let control = app
        .terminal_control_facts("session-test")
        .await
        .expect("terminal control should build");

    assert_eq!(control.current_mode_id, "code");
    assert_eq!(control.active_turn_id.as_deref(), Some("turn-1"));
    assert!(control.active_plan.is_none());
    assert!(
        !project.path().join(".astrcode").exists(),
        "task facts query must not materialize canonical session plan artifacts"
    );
    assert_eq!(
        control.active_tasks,
        Some(vec![
            crate::terminal::TaskItemFacts {
                content: "实现 authoritative task panel".to_string(),
                status: ExecutionTaskStatus::InProgress,
                active_form: Some("正在实现 authoritative task panel".to_string()),
            },
            crate::terminal::TaskItemFacts {
                content: "补充前端 hydration 测试".to_string(),
                status: ExecutionTaskStatus::Pending,
                active_form: None,
            },
        ])
    );
}
