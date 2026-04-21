use std::{path::Path, sync::Arc};

use astrcode_core::{
    AgentEvent, AgentEventContext, AgentLifecycleStatus, ChildAgentRef, ChildSessionNotification,
    ChildSessionNotificationKind, DeleteProjectResult, EventStore, ParentDelivery,
    ParentDeliveryOrigin, ParentDeliveryPayload, ParentDeliveryTerminalSemantics, Phase,
    PromptMetricsPayload, SessionEventRecord, SessionId, SessionMeta, SessionTurnAcquireResult,
    StorageEvent, StorageEventPayload, StoredEvent, ToolExecutionResult, ToolOutputStream,
    UserMessageOrigin,
};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::json;
use tokio::sync::broadcast;

use super::{
    ConversationBlockFacts, ConversationBlockPatchFacts, ConversationBlockStatus,
    ConversationChildHandoffKind, ConversationDeltaFacts, ConversationDeltaProjector,
    ConversationPlanEventKind, ConversationStreamProjector, ConversationStreamReplayFacts,
    build_conversation_replay_frames, fallback_live_cursor, project_conversation_snapshot,
};
use crate::{
    SessionReplay, SessionRuntime,
    state::sample_spawn_child_ref,
    turn::test_support::{NoopMetrics, NoopPromptFactsProvider, test_kernel},
};

#[test]
fn snapshot_projects_tool_call_block_with_streams_and_terminal_fields() {
    let records = vec![
        record(
            "1.1",
            AgentEvent::ToolCallStart {
                turn_id: "turn-1".to_string(),
                agent: sample_agent_context(),
                tool_call_id: "call-1".to_string(),
                tool_name: "shell_command".to_string(),
                input: json!({ "command": "pwd" }),
            },
        ),
        record(
            "1.2",
            AgentEvent::ToolCallDelta {
                turn_id: "turn-1".to_string(),
                agent: sample_agent_context(),
                tool_call_id: "call-1".to_string(),
                tool_name: "shell_command".to_string(),
                stream: ToolOutputStream::Stdout,
                delta: "line-1\n".to_string(),
            },
        ),
        record(
            "1.3",
            AgentEvent::ToolCallDelta {
                turn_id: "turn-1".to_string(),
                agent: sample_agent_context(),
                tool_call_id: "call-1".to_string(),
                tool_name: "shell_command".to_string(),
                stream: ToolOutputStream::Stderr,
                delta: "warn\n".to_string(),
            },
        ),
        record(
            "1.4",
            AgentEvent::ToolCallResult {
                turn_id: "turn-1".to_string(),
                agent: sample_agent_context(),
                result: ToolExecutionResult {
                    tool_call_id: "call-1".to_string(),
                    tool_name: "shell_command".to_string(),
                    ok: false,
                    output: "line-1\n".to_string(),
                    error: Some("permission denied".to_string()),
                    metadata: Some(json!({ "path": "/tmp", "truncated": true })),
                    continuation: None,
                    duration_ms: 42,
                    truncated: true,
                },
            },
        ),
    ];

    let snapshot = project_conversation_snapshot(&records, Phase::CallingTool);
    let tool = snapshot
        .blocks
        .iter()
        .find_map(|block| match block {
            ConversationBlockFacts::ToolCall(block) => Some(block),
            _ => None,
        })
        .expect("tool block should exist");

    assert_eq!(tool.tool_call_id, "call-1");
    assert_eq!(tool.status, ConversationBlockStatus::Failed);
    assert_eq!(tool.streams.stdout, "line-1\n");
    assert_eq!(tool.streams.stderr, "warn\n");
    assert_eq!(tool.error.as_deref(), Some("permission denied"));
    assert_eq!(tool.duration_ms, Some(42));
    assert!(tool.truncated);
}

#[test]
fn snapshot_preserves_failed_tool_status_after_turn_done() {
    let records = vec![
        record(
            "1.1",
            AgentEvent::ToolCallStart {
                turn_id: "turn-1".to_string(),
                agent: sample_agent_context(),
                tool_call_id: "call-1".to_string(),
                tool_name: "shell_command".to_string(),
                input: json!({ "command": "missing-command" }),
            },
        ),
        record(
            "1.2",
            AgentEvent::ToolCallResult {
                turn_id: "turn-1".to_string(),
                agent: sample_agent_context(),
                result: ToolExecutionResult {
                    tool_call_id: "call-1".to_string(),
                    tool_name: "shell_command".to_string(),
                    ok: false,
                    output: String::new(),
                    error: Some("command not found".to_string()),
                    metadata: None,
                    continuation: None,
                    duration_ms: 127,
                    truncated: false,
                },
            },
        ),
        record(
            "1.3",
            AgentEvent::TurnDone {
                turn_id: "turn-1".to_string(),
                agent: sample_agent_context(),
            },
        ),
    ];

    let snapshot = project_conversation_snapshot(&records, Phase::Idle);
    let tool = snapshot
        .blocks
        .iter()
        .find_map(|block| match block {
            ConversationBlockFacts::ToolCall(block) => Some(block),
            _ => None,
        })
        .expect("tool block should exist");

    assert_eq!(tool.status, ConversationBlockStatus::Failed);
    assert_eq!(tool.error.as_deref(), Some("command not found"));
    assert_eq!(tool.duration_ms, Some(127));
}

#[test]
fn snapshot_projects_plan_blocks_in_durable_event_order() {
    let records = vec![
        record(
            "1.1",
            AgentEvent::ToolCallStart {
                turn_id: "turn-1".to_string(),
                agent: sample_agent_context(),
                tool_call_id: "call-plan-save".to_string(),
                tool_name: "upsertSessionPlan".to_string(),
                input: json!({
                    "title": "Cleanup crates",
                    "content": "# Plan: Cleanup crates"
                }),
            },
        ),
        record(
            "1.2",
            AgentEvent::ToolCallResult {
                turn_id: "turn-1".to_string(),
                agent: sample_agent_context(),
                result: ToolExecutionResult {
                    tool_call_id: "call-plan-save".to_string(),
                    tool_name: "upsertSessionPlan".to_string(),
                    ok: true,
                    output: "updated session plan".to_string(),
                    error: None,
                    metadata: Some(json!({
                        "planPath": "C:/Users/demo/.astrcode/projects/demo/sessions/session-1/plan/cleanup-crates.md",
                        "slug": "cleanup-crates",
                        "status": "draft",
                        "title": "Cleanup crates",
                        "updatedAt": "2026-04-19T09:00:00Z"
                    })),
                    continuation: None,
                    duration_ms: 7,
                    truncated: false,
                },
            },
        ),
        record(
            "1.3",
            AgentEvent::ToolCallStart {
                turn_id: "turn-1".to_string(),
                agent: sample_agent_context(),
                tool_call_id: "call-shell".to_string(),
                tool_name: "shell_command".to_string(),
                input: json!({ "command": "pwd" }),
            },
        ),
        record(
            "1.4",
            AgentEvent::ToolCallResult {
                turn_id: "turn-1".to_string(),
                agent: sample_agent_context(),
                result: ToolExecutionResult {
                    tool_call_id: "call-shell".to_string(),
                    tool_name: "shell_command".to_string(),
                    ok: true,
                    output: "D:/GitObjectsOwn/Astrcode".to_string(),
                    error: None,
                    metadata: None,
                    continuation: None,
                    duration_ms: 9,
                    truncated: false,
                },
            },
        ),
        record(
            "1.5",
            AgentEvent::ToolCallStart {
                turn_id: "turn-1".to_string(),
                agent: sample_agent_context(),
                tool_call_id: "call-plan-exit".to_string(),
                tool_name: "exitPlanMode".to_string(),
                input: json!({}),
            },
        ),
        record(
            "1.6",
            AgentEvent::ToolCallResult {
                turn_id: "turn-1".to_string(),
                agent: sample_agent_context(),
                result: ToolExecutionResult {
                    tool_call_id: "call-plan-exit".to_string(),
                    tool_name: "exitPlanMode".to_string(),
                    ok: true,
                    output: "Before exiting plan mode, do one final self-review.".to_string(),
                    error: None,
                    metadata: Some(json!({
                        "schema": "sessionPlanExitReviewPending",
                        "plan": {
                            "title": "Cleanup crates",
                            "planPath": "C:/Users/demo/.astrcode/projects/demo/sessions/session-1/plan/cleanup-crates.md"
                        },
                        "review": {
                            "kind": "final_review",
                            "checklist": [
                                "Re-check assumptions against the code you already inspected."
                            ]
                        },
                        "blockers": {
                            "missingHeadings": ["## Verification"],
                            "invalidSections": []
                        }
                    })),
                    continuation: None,
                    duration_ms: 5,
                    truncated: false,
                },
            },
        ),
    ];

    let snapshot = project_conversation_snapshot(&records, Phase::Idle);
    assert_eq!(snapshot.blocks.len(), 3);
    assert!(matches!(
        &snapshot.blocks[0],
        ConversationBlockFacts::Plan(block)
            if block.tool_call_id == "call-plan-save"
                && block.event_kind == ConversationPlanEventKind::Saved
    ));
    assert!(matches!(
        &snapshot.blocks[1],
        ConversationBlockFacts::ToolCall(block) if block.tool_call_id == "call-shell"
    ));
    assert!(matches!(
        &snapshot.blocks[2],
        ConversationBlockFacts::Plan(block)
            if block.tool_call_id == "call-plan-exit"
                && block.event_kind == ConversationPlanEventKind::ReviewPending
    ));
}

#[test]
fn snapshot_keeps_task_write_as_normal_tool_call_block() {
    let records = vec![
        record(
            "1.1",
            AgentEvent::ToolCallStart {
                turn_id: "turn-1".to_string(),
                agent: sample_agent_context(),
                tool_call_id: "call-task-write".to_string(),
                tool_name: "taskWrite".to_string(),
                input: json!({
                    "items": [
                        {
                            "content": "实现 authoritative task panel",
                            "status": "in_progress",
                            "activeForm": "正在实现 authoritative task panel"
                        }
                    ]
                }),
            },
        ),
        record(
            "1.2",
            AgentEvent::ToolCallResult {
                turn_id: "turn-1".to_string(),
                agent: sample_agent_context(),
                result: ToolExecutionResult {
                    tool_call_id: "call-task-write".to_string(),
                    tool_name: "taskWrite".to_string(),
                    ok: true,
                    output: "updated execution tasks".to_string(),
                    error: None,
                    metadata: Some(json!({
                        "schema": "executionTaskSnapshot",
                        "owner": "root-agent",
                        "cleared": false,
                        "items": [
                            {
                                "content": "实现 authoritative task panel",
                                "status": "in_progress",
                                "activeForm": "正在实现 authoritative task panel"
                            }
                        ]
                    })),
                    continuation: None,
                    duration_ms: 5,
                    truncated: false,
                },
            },
        ),
    ];

    let snapshot = project_conversation_snapshot(&records, Phase::CallingTool);
    assert_eq!(snapshot.blocks.len(), 1);
    assert!(matches!(
        &snapshot.blocks[0],
        ConversationBlockFacts::ToolCall(block)
            if block.tool_name == "taskWrite"
                && block.tool_call_id == "call-task-write"
                && block.summary.as_deref() == Some("updated execution tasks")
    ));
    assert!(
        snapshot
            .blocks
            .iter()
            .all(|block| !matches!(block, ConversationBlockFacts::Plan(_))),
        "taskWrite must not be projected onto the canonical plan surface"
    );
}

#[test]
fn live_then_durable_tool_delta_dedupes_chunk_on_same_tool_block() {
    let facts = sample_stream_replay_facts(
        vec![record(
            "1.1",
            AgentEvent::ToolCallStart {
                turn_id: "turn-1".to_string(),
                agent: sample_agent_context(),
                tool_call_id: "call-1".to_string(),
                tool_name: "shell_command".to_string(),
                input: json!({ "command": "pwd" }),
            },
        )],
        vec![record(
            "1.2",
            AgentEvent::ToolCallDelta {
                turn_id: "turn-1".to_string(),
                agent: sample_agent_context(),
                tool_call_id: "call-1".to_string(),
                tool_name: "shell_command".to_string(),
                stream: ToolOutputStream::Stdout,
                delta: "line-1\n".to_string(),
            },
        )],
    );
    let mut stream = ConversationStreamProjector::new(Some("1.1".to_string()), &facts);

    let live_frames = stream.project_live_event(&AgentEvent::ToolCallDelta {
        turn_id: "turn-1".to_string(),
        agent: sample_agent_context(),
        tool_call_id: "call-1".to_string(),
        tool_name: "shell_command".to_string(),
        stream: ToolOutputStream::Stdout,
        delta: "line-1\n".to_string(),
    });
    assert_eq!(live_frames.len(), 1);

    let replayed = stream.recover_from(&facts);
    assert!(
        replayed.is_empty(),
        "durable replay should not duplicate the live-emitted chunk"
    );
}

#[test]
fn snapshot_tracks_last_durable_step_cursor_from_prompt_metrics() {
    let records = vec![
        record(
            "1.1",
            AgentEvent::PromptMetrics {
                turn_id: Some("turn-1".to_string()),
                agent: sample_agent_context(),
                metrics: PromptMetricsPayload {
                    step_index: 0,
                    estimated_tokens: 1200,
                    context_window: 200_000,
                    effective_window: 180_000,
                    threshold_tokens: 144_000,
                    truncated_tool_results: 0,
                    provider_input_tokens: Some(800),
                    provider_output_tokens: Some(120),
                    cache_creation_input_tokens: Some(0),
                    cache_read_input_tokens: Some(640),
                    provider_cache_metrics_supported: true,
                    prompt_cache_reuse_hits: 2,
                    prompt_cache_reuse_misses: 0,
                    prompt_cache_unchanged_layers: Vec::new(),
                    prompt_cache_diagnostics: None,
                },
            },
        ),
        record(
            "1.2",
            AgentEvent::AssistantMessage {
                turn_id: "turn-1".to_string(),
                agent: sample_agent_context(),
                content: "first step".to_string(),
                reasoning_content: None,
                step_index: Some(0),
            },
        ),
        record(
            "1.3",
            AgentEvent::PromptMetrics {
                turn_id: Some("turn-1".to_string()),
                agent: sample_agent_context(),
                metrics: PromptMetricsPayload {
                    step_index: 1,
                    estimated_tokens: 1600,
                    context_window: 200_000,
                    effective_window: 180_000,
                    threshold_tokens: 144_000,
                    truncated_tool_results: 0,
                    provider_input_tokens: Some(1100),
                    provider_output_tokens: Some(96),
                    cache_creation_input_tokens: Some(0),
                    cache_read_input_tokens: Some(896),
                    provider_cache_metrics_supported: true,
                    prompt_cache_reuse_hits: 3,
                    prompt_cache_reuse_misses: 0,
                    prompt_cache_unchanged_layers: Vec::new(),
                    prompt_cache_diagnostics: None,
                },
            },
        ),
    ];

    let snapshot = project_conversation_snapshot(&records, Phase::Streaming);

    assert_eq!(
        snapshot
            .step_progress
            .durable
            .as_ref()
            .map(|cursor| (cursor.turn_id.as_str(), cursor.step_index,)),
        Some(("turn-1", 1))
    );
    assert!(snapshot.step_progress.live.is_none());
}

#[test]
fn stream_projector_marks_live_step_after_last_durable_step() {
    let facts = sample_stream_replay_facts(
        vec![record(
            "1.1",
            AgentEvent::PromptMetrics {
                turn_id: Some("turn-1".to_string()),
                agent: sample_agent_context(),
                metrics: PromptMetricsPayload {
                    step_index: 0,
                    estimated_tokens: 1200,
                    context_window: 200_000,
                    effective_window: 180_000,
                    threshold_tokens: 144_000,
                    truncated_tool_results: 0,
                    provider_input_tokens: Some(800),
                    provider_output_tokens: Some(120),
                    cache_creation_input_tokens: Some(0),
                    cache_read_input_tokens: Some(640),
                    provider_cache_metrics_supported: true,
                    prompt_cache_reuse_hits: 2,
                    prompt_cache_reuse_misses: 0,
                    prompt_cache_unchanged_layers: Vec::new(),
                    prompt_cache_diagnostics: None,
                },
            },
        )],
        Vec::new(),
    );
    let mut stream = ConversationStreamProjector::new(Some("1.1".to_string()), &facts);

    let live_frames = stream.project_live_event(&AgentEvent::ModelDelta {
        turn_id: "turn-1".to_string(),
        agent: sample_agent_context(),
        delta: "next step".to_string(),
    });

    assert_eq!(live_frames.len(), 1);
    assert_eq!(
        live_frames[0]
            .step_progress
            .durable
            .as_ref()
            .map(|cursor| (cursor.turn_id.as_str(), cursor.step_index)),
        Some(("turn-1", 0))
    );
    assert_eq!(
        live_frames[0]
            .step_progress
            .live
            .as_ref()
            .map(|cursor| (cursor.turn_id.as_str(), cursor.step_index)),
        Some(("turn-1", 1))
    );
}

#[test]
fn child_notification_patches_tool_block_and_appends_handoff_block() {
    let mut projector = ConversationDeltaProjector::new();
    projector.seed(&[record(
        "1.1",
        AgentEvent::ToolCallStart {
            turn_id: "turn-1".to_string(),
            agent: sample_agent_context(),
            tool_call_id: "call-spawn".to_string(),
            tool_name: "spawn_agent".to_string(),
            input: json!({ "task": "inspect" }),
        },
    )]);

    let deltas = projector.project_record(&record(
        "1.2",
        AgentEvent::ChildSessionNotification {
            turn_id: Some("turn-1".to_string()),
            agent: sample_agent_context(),
            notification: sample_child_notification(),
        },
    ));

    assert!(deltas.iter().any(|delta| matches!(
        delta,
        ConversationDeltaFacts::PatchBlock {
            block_id,
            patch: ConversationBlockPatchFacts::ReplaceChildRef { .. },
        } if block_id == "tool:call-spawn:call"
    )));
    assert!(deltas.iter().any(|delta| matches!(
        delta,
        ConversationDeltaFacts::AppendBlock {
            block,
        } if matches!(
            block.as_ref(),
            ConversationBlockFacts::ChildHandoff(block)
                if block.handoff_kind == ConversationChildHandoffKind::Returned
        )
    )));
}

#[test]
fn tool_result_continuation_alone_patches_tool_block() {
    let child_ref = sample_child_ref();
    let records = vec![
        record(
            "1.1",
            AgentEvent::ToolCallStart {
                turn_id: "turn-1".to_string(),
                agent: sample_agent_context(),
                tool_call_id: "call-spawn".to_string(),
                tool_name: "spawn".to_string(),
                input: json!({ "description": "inspect" }),
            },
        ),
        record(
            "1.2",
            AgentEvent::ToolCallResult {
                turn_id: "turn-1".to_string(),
                agent: sample_agent_context(),
                result: ToolExecutionResult {
                    tool_call_id: "call-spawn".to_string(),
                    tool_name: "spawn".to_string(),
                    ok: true,
                    output: "子 Agent 已启动：inspect".to_string(),
                    error: None,
                    metadata: Some(json!({ "schema": "subRunResult" })),
                    continuation: Some(astrcode_core::ExecutionContinuation::child_agent(
                        child_ref.clone(),
                    )),
                    duration_ms: 12,
                    truncated: false,
                },
            },
        ),
    ];

    let snapshot = project_conversation_snapshot(&records, Phase::CallingTool);
    let tool = snapshot
        .blocks
        .iter()
        .find_map(|block| match block {
            ConversationBlockFacts::ToolCall(block) => Some(block),
            _ => None,
        })
        .expect("tool block should exist");

    assert_eq!(tool.tool_call_id, "call-spawn");
    assert_eq!(tool.child_ref.as_ref(), Some(&child_ref));
}

#[tokio::test]
async fn runtime_query_builds_snapshot_and_stream_replay_facts() {
    let event_store = Arc::new(ReplayOnlyEventStore::new(vec![
        stored(
            1,
            storage_event(
                Some("turn-1"),
                StorageEventPayload::UserMessage {
                    content: "inspect repo".to_string(),
                    origin: UserMessageOrigin::User,
                    timestamp: Utc::now(),
                },
            ),
        ),
        stored(
            2,
            storage_event(
                Some("turn-1"),
                StorageEventPayload::ToolCall {
                    tool_call_id: "call-1".to_string(),
                    tool_name: "shell_command".to_string(),
                    args: json!({ "command": "pwd" }),
                },
            ),
        ),
        stored(
            3,
            storage_event(
                Some("turn-1"),
                StorageEventPayload::ToolCallDelta {
                    tool_call_id: "call-1".to_string(),
                    tool_name: "shell_command".to_string(),
                    stream: ToolOutputStream::Stdout,
                    delta: "D:/GitObjectsOwn/Astrcode\n".to_string(),
                },
            ),
        ),
        stored(
            4,
            storage_event(
                Some("turn-1"),
                StorageEventPayload::ToolResult {
                    tool_call_id: "call-1".to_string(),
                    tool_name: "shell_command".to_string(),
                    output: "D:/GitObjectsOwn/Astrcode\n".to_string(),
                    success: true,
                    error: None,
                    metadata: None,
                    continuation: None,
                    duration_ms: 7,
                },
            ),
        ),
        stored(
            5,
            storage_event(
                Some("turn-1"),
                StorageEventPayload::AssistantFinal {
                    content: "done".to_string(),
                    reasoning_content: Some("think".to_string()),
                    reasoning_signature: None,
                    step_index: None,
                    timestamp: None,
                },
            ),
        ),
    ]));
    let runtime = SessionRuntime::new(
        Arc::new(test_kernel(8192)),
        Arc::new(NoopPromptFactsProvider),
        event_store,
        Arc::new(NoopMetrics),
    );

    let snapshot = runtime
        .conversation_snapshot("session-1")
        .await
        .expect("snapshot should build");
    assert!(snapshot.blocks.iter().any(|block| matches!(
        block,
        ConversationBlockFacts::ToolCall(block)
            if block.tool_call_id == "call-1"
    )));

    let transcript = runtime
        .session_transcript_snapshot("session-1")
        .await
        .expect("transcript snapshot should build");
    assert!(transcript.records.len() > 4);
    let cursor = transcript.records[3].event_id.clone();

    let replay = runtime
        .conversation_stream_replay("session-1", Some(cursor.as_str()))
        .await
        .expect("replay facts should build");
    assert_eq!(
        replay
            .seed_records
            .last()
            .map(|record| record.event_id.as_str()),
        Some(cursor.as_str())
    );
    assert!(!replay.replay_frames.is_empty());
    assert_eq!(
        fallback_live_cursor(&replay).as_deref(),
        Some(cursor.as_str())
    );
}

fn sample_stream_replay_facts(
    seed_records: Vec<SessionEventRecord>,
    history: Vec<SessionEventRecord>,
) -> ConversationStreamReplayFacts {
    let (_, receiver) = broadcast::channel(8);
    let (_, live_receiver) = broadcast::channel(8);
    ConversationStreamReplayFacts {
        cursor: history.last().map(|record| record.event_id.clone()),
        phase: Phase::CallingTool,
        seed_records: seed_records.clone(),
        replay_frames: build_conversation_replay_frames(&seed_records, &history),
        replay: SessionReplay {
            history,
            receiver,
            live_receiver,
        },
    }
}

fn sample_agent_context() -> AgentEventContext {
    AgentEventContext::root_execution("agent-root", "default")
}

fn sample_child_ref() -> ChildAgentRef {
    sample_spawn_child_ref(AgentLifecycleStatus::Running)
}

fn sample_child_notification() -> ChildSessionNotification {
    ChildSessionNotification {
        notification_id: "child-note-1".to_string().into(),
        child_ref: sample_child_ref(),
        kind: ChildSessionNotificationKind::Delivered,
        source_tool_call_id: Some("call-spawn".to_string().into()),
        delivery: Some(ParentDelivery {
            idempotency_key: "delivery-1".to_string(),
            origin: ParentDeliveryOrigin::Explicit,
            terminal_semantics: ParentDeliveryTerminalSemantics::Terminal,
            source_turn_id: Some("turn-1".to_string()),
            payload: ParentDeliveryPayload::Progress(
                astrcode_core::ProgressParentDeliveryPayload {
                    message: "child finished".to_string(),
                },
            ),
        }),
    }
}

fn record(event_id: &str, event: AgentEvent) -> SessionEventRecord {
    SessionEventRecord {
        event_id: event_id.to_string(),
        event,
    }
}

fn stored(storage_seq: u64, event: StorageEvent) -> StoredEvent {
    StoredEvent { storage_seq, event }
}

fn storage_event(turn_id: Option<&str>, payload: StorageEventPayload) -> StorageEvent {
    StorageEvent {
        turn_id: turn_id.map(ToString::to_string),
        agent: sample_agent_context(),
        payload,
    }
}

struct ReplayOnlyEventStore {
    events: Vec<StoredEvent>,
}

impl ReplayOnlyEventStore {
    fn new(events: Vec<StoredEvent>) -> Self {
        Self { events }
    }
}

struct StubTurnLease;

impl astrcode_core::SessionTurnLease for StubTurnLease {}

#[async_trait]
impl EventStore for ReplayOnlyEventStore {
    async fn ensure_session(
        &self,
        _session_id: &SessionId,
        _working_dir: &Path,
    ) -> astrcode_core::Result<()> {
        Ok(())
    }

    async fn append(
        &self,
        _session_id: &SessionId,
        _event: &astrcode_core::StorageEvent,
    ) -> astrcode_core::Result<StoredEvent> {
        panic!("append should not be called in replay-only test store");
    }

    async fn replay(&self, _session_id: &SessionId) -> astrcode_core::Result<Vec<StoredEvent>> {
        Ok(self.events.clone())
    }

    async fn try_acquire_turn(
        &self,
        _session_id: &SessionId,
        _turn_id: &str,
    ) -> astrcode_core::Result<SessionTurnAcquireResult> {
        Ok(SessionTurnAcquireResult::Acquired(Box::new(StubTurnLease)))
    }

    async fn list_sessions(&self) -> astrcode_core::Result<Vec<SessionId>> {
        Ok(vec![SessionId::from("session-1".to_string())])
    }

    async fn list_session_metas(&self) -> astrcode_core::Result<Vec<SessionMeta>> {
        Ok(vec![SessionMeta {
            session_id: "session-1".to_string(),
            working_dir: ".".to_string(),
            display_name: "session-1".to_string(),
            title: "session-1".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            parent_session_id: None,
            parent_storage_seq: None,
            phase: Phase::Done,
        }])
    }

    async fn delete_session(&self, _session_id: &SessionId) -> astrcode_core::Result<()> {
        Ok(())
    }

    async fn delete_sessions_by_working_dir(
        &self,
        _working_dir: &str,
    ) -> astrcode_core::Result<DeleteProjectResult> {
        Ok(DeleteProjectResult {
            success_count: 0,
            failed_session_ids: Vec::new(),
        })
    }
}
