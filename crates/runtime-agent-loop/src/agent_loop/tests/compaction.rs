//! Compaction 相关测试。
//!
//! 覆盖：
//! - auto compact 触发并发出 CompactApplied 事件
//! - manual compact 使用 Manual trigger
//! - reactive compact 从 413 错误恢复
//! - reactive compact 无可压缩内容时正确报错
//! - 不可恢复错误不触发 compact

use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use astrcode_core::{
    AgentEventContext, AstrError, CancelToken, LlmMessage, Phase, StorageEvent,
    StorageEventPayload, Tool, ToolCapabilityMetadata, ToolDefinition, ToolExecutionResult,
    UserMessageOrigin,
};
use astrcode_protocol::capability::SideEffectLevel;
use astrcode_runtime_llm::{EventSink, LlmOutput, LlmProvider, LlmRequest, ModelLimits};
use async_trait::async_trait;
use serde_json::json;

use super::{
    fixtures::*,
    test_support::{boxed_tool, capabilities_from_tools, empty_capabilities},
};
use crate::{
    AgentLoop,
    agent_loop::TurnOutcome,
    compaction_runtime::{
        CompactionArtifact, CompactionInput, CompactionReason, CompactionRuntime,
        CompactionTailSnapshot, FsFileContentProvider, ThresholdCompactionPolicy,
    },
    context_pipeline::CompactionView,
};

struct RecordingFailingProvider {
    results: Mutex<VecDeque<astrcode_core::Result<LlmOutput>>>,
    requests: Arc<Mutex<Vec<LlmRequest>>>,
}

#[async_trait]
impl LlmProvider for RecordingFailingProvider {
    fn model_limits(&self) -> ModelLimits {
        ModelLimits {
            context_window: 200_000,
            max_output_tokens: 4_096,
        }
    }

    async fn generate(
        &self,
        request: LlmRequest,
        sink: Option<EventSink>,
    ) -> astrcode_core::Result<LlmOutput> {
        self.requests
            .lock()
            .expect("request log lock")
            .push(request.clone());
        let result = self
            .results
            .lock()
            .expect("provider results lock")
            .pop_front()
            .expect("scripted provider result");

        if let (Ok(response), Some(sink)) = (&result, sink) {
            for delta in response.content.chars() {
                sink(astrcode_runtime_llm::LlmEvent::TextDelta(delta.to_string()));
            }
        }

        result
    }
}

struct ReadFileMetadataTool {
    path: PathBuf,
    output: String,
}

#[async_trait]
impl Tool for ReadFileMetadataTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "readFile".to_string(),
            description: "returns file content with metadata.path for recovery tests".to_string(),
            parameters: json!({"type":"object"}),
        }
    }

    fn capability_metadata(&self) -> ToolCapabilityMetadata {
        ToolCapabilityMetadata::builtin()
            .side_effect(SideEffectLevel::None)
            .concurrency_safe(true)
            .compact_clearable(true)
    }

    async fn execute(
        &self,
        tool_call_id: String,
        _input: serde_json::Value,
        _ctx: &astrcode_core::ToolContext,
    ) -> astrcode_core::Result<ToolExecutionResult> {
        Ok(ToolExecutionResult {
            tool_call_id,
            tool_name: "readFile".to_string(),
            ok: true,
            output: self.output.clone(),
            error: None,
            metadata: Some(json!({ "path": self.path.display().to_string() })),
            duration_ms: 1,
            truncated: false,
        })
    }
}

struct StaticArtifactStrategy;

#[async_trait]
impl crate::compaction_runtime::CompactionStrategy for StaticArtifactStrategy {
    async fn compact(
        &self,
        _input: CompactionInput<'_>,
    ) -> astrcode_core::Result<Option<CompactionArtifact>> {
        Ok(Some(CompactionArtifact {
            summary: "summary".to_string(),
            source_range: crate::compaction_runtime::EventRange { start: 0, end: 1 },
            preserved_tail_start: 1,
            strategy_id: "test".to_string(),
            pre_tokens: 100,
            post_tokens_estimate: 40,
            compacted_at_seq: 0,
            trigger: CompactionReason::Auto,
            preserved_recent_turns: 1,
            messages_removed: 1,
            tokens_freed: 60,
            recovered_files: Vec::new(),
        }))
    }
}

struct FailingRebuilder;

impl crate::compaction_runtime::CompactionRebuilder for FailingRebuilder {
    fn rebuild(
        &self,
        _artifact: &CompactionArtifact,
        _tail: &[astrcode_core::StoredEvent],
        _file_contents: &[(std::path::PathBuf, String)],
        _truncate_bytes: usize,
    ) -> astrcode_core::Result<CompactionView> {
        Err(AstrError::Internal("rebuild failed".to_string()))
    }
}

struct RecordingRebuilder {
    recovered_files: Arc<Mutex<Vec<(PathBuf, String)>>>,
}

impl crate::compaction_runtime::CompactionRebuilder for RecordingRebuilder {
    fn rebuild(
        &self,
        _artifact: &CompactionArtifact,
        _tail: &[astrcode_core::StoredEvent],
        file_contents: &[(std::path::PathBuf, String)],
        _truncate_bytes: usize,
    ) -> astrcode_core::Result<CompactionView> {
        *self.recovered_files.lock().expect("recovered files lock") = file_contents.to_vec();
        Ok(CompactionView {
            messages: Vec::new(),
            memory_blocks: Vec::new(),
            recovery_refs: Vec::new(),
        })
    }
}

#[tokio::test]
async fn auto_compact_emits_compact_applied_before_retrying_the_turn() {
    let _guard = super::test_support::TestEnvGuard::new();
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([
            LlmOutput {
                content: "<analysis>trimmed</analysis><summary>condensed history</summary>"
                    .to_string(),
                tool_calls: vec![],
                reasoning: None,
                usage: None,
                finish_reason: Default::default(),
            },
            LlmOutput {
                content: "final answer".to_string(),
                tool_calls: vec![],
                reasoning: None,
                usage: None,
                finish_reason: Default::default(),
            },
        ])),
        delay: std::time::Duration::from_millis(0),
    });
    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, empty_capabilities())
        .with_auto_compact_enabled(true)
        .with_compact_threshold_percent(1)
        .with_compact_keep_recent_turns(1);
    let state = astrcode_core::AgentState {
        session_id: "test".into(),
        working_dir: std::env::temp_dir(),
        messages: vec![
            LlmMessage::User {
                content: "legacy ".repeat(1_500),
                origin: UserMessageOrigin::User,
            },
            LlmMessage::Assistant {
                content: "ack".to_string(),
                tool_calls: vec![],
                reasoning: None,
            },
            LlmMessage::User {
                content: "current ask".to_string(),
                origin: UserMessageOrigin::User,
            },
        ],
        phase: Phase::Thinking,
        turn_count: 0,
    };
    let (events, mut on_event) = collect_events();

    let outcome = loop_runner
        .run_turn(
            &state,
            "turn-auto-compact",
            &mut on_event,
            CancelToken::new(),
        )
        .await
        .expect("turn should complete");

    assert!(matches!(outcome, TurnOutcome::Completed));
    let events = events.lock().expect("events lock");
    assert!(matches!(
        events.first(),
        Some(StorageEvent {
            payload: StorageEventPayload::PromptMetrics { .. },
            ..
        })
    ));
    assert!(events.iter().any(|event| {
        matches!(
            &event.payload,
            StorageEventPayload::CompactApplied { summary, .. } if summary == "condensed history"
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            &event.payload,
            StorageEventPayload::AssistantFinal { content, .. } if content == "final answer"
        )
    }));
}

#[tokio::test]
async fn auto_compact_with_keep_recent_turns_two_keeps_second_latest_turn_tool_result() {
    let _guard = super::test_support::TestEnvGuard::new();
    let protected_tool_result = "protected readFile result\n".repeat(80);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(RecordingProvider {
        responses: Mutex::new(VecDeque::from([
            LlmOutput {
                content: "<analysis>trimmed</analysis><summary>condensed history</summary>"
                    .to_string(),
                tool_calls: vec![],
                reasoning: None,
                usage: None,
                finish_reason: Default::default(),
            },
            LlmOutput {
                content: "final answer".to_string(),
                tool_calls: vec![],
                reasoning: None,
                usage: None,
                finish_reason: Default::default(),
            },
        ])),
        requests: Arc::clone(&requests),
    });
    let tools = vec![];
    let loop_runner = AgentLoop::from_capabilities(
        Arc::new(StaticProviderFactory { provider }),
        capabilities_from_tools(tools),
    )
    .with_auto_compact_enabled(true)
    .with_compact_threshold_percent(1)
    .with_compact_keep_recent_turns(2);
    let state = astrcode_core::AgentState {
        session_id: "test".into(),
        working_dir: std::env::temp_dir(),
        messages: vec![
            LlmMessage::User {
                content: "legacy ".repeat(2_500),
                origin: UserMessageOrigin::User,
            },
            LlmMessage::Assistant {
                content: "legacy answer".to_string(),
                tool_calls: vec![],
                reasoning: None,
            },
            LlmMessage::User {
                content: "inspect protected file".to_string(),
                origin: UserMessageOrigin::User,
            },
            LlmMessage::Assistant {
                content: String::new(),
                tool_calls: vec![astrcode_core::ToolCallRequest {
                    id: "call-read".to_string(),
                    name: "readFile".to_string(),
                    args: json!({"path":"src/protected.rs"}),
                }],
                reasoning: None,
            },
            LlmMessage::Tool {
                tool_call_id: "call-read".to_string(),
                content: protected_tool_result.clone(),
            },
            LlmMessage::User {
                content: "continue with the protected file context".to_string(),
                origin: UserMessageOrigin::User,
            },
        ],
        phase: Phase::Thinking,
        turn_count: 0,
    };
    let (_events, mut on_event) = collect_events();

    let outcome = loop_runner
        .run_turn(
            &state,
            "turn-auto-compact-keep-two",
            &mut on_event,
            CancelToken::new(),
        )
        .await
        .expect("turn should complete");

    assert!(matches!(outcome, TurnOutcome::Completed));
    let requests = requests.lock().expect("request log lock");
    assert_eq!(
        requests.len(),
        2,
        "auto compact should issue one summary request and one final model request"
    );
    let final_request = requests.last().expect("final request should exist");
    assert!(
        final_request.messages.iter().any(|message| {
            matches!(
                message,
                LlmMessage::Tool { tool_call_id, content }
                    if tool_call_id == "call-read" && content == &protected_tool_result
            )
        }),
        "the final post-compact request should still carry the clearable tool result from the \
         second-latest preserved turn"
    );
    assert!(
        !final_request.messages.iter().any(|message| {
            matches!(
                message,
                LlmMessage::Tool { content, .. } if content.contains("[cleared older tool result")
            )
        }),
        "prune pass must not rewrite tool results that belong to the requested recent two turns"
    );
}

#[tokio::test]
async fn reactive_compact_restores_recent_file_context_after_current_turn_file_access() {
    let _guard = super::test_support::TestEnvGuard::new();
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let file_path = temp_dir.path().join("recent.rs");
    // auto compact 必须既“超阈值”又“确实有旧 turn 可折叠”才会触发，所以这里保留一段
    // 轻量旧历史，再让 readFile 结果把第二轮请求推过阈值，专门覆盖“当前 turn 文件访问 →
    // compact → recovered_files → rebuild”这条链路。
    let file_contents = "fn recovered_context() { println!(\"hello\"); }\n".repeat(400);
    std::fs::write(&file_path, &file_contents).expect("fixture file should be writable");

    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(RecordingFailingProvider {
        results: Mutex::new(VecDeque::from([
            Ok(LlmOutput {
                content: String::new(),
                tool_calls: vec![astrcode_core::ToolCallRequest {
                    id: "call-read".to_string(),
                    name: "readFile".to_string(),
                    args: json!({ "path": file_path.display().to_string() }),
                }],
                reasoning: None,
                usage: None,
                finish_reason: Default::default(),
            }),
            Err(make_prompt_too_long_error()),
            Ok(LlmOutput {
                content: "<analysis>trimmed</analysis><summary>summary after read</summary>"
                    .to_string(),
                tool_calls: vec![],
                reasoning: None,
                usage: None,
                finish_reason: Default::default(),
            }),
            Ok(LlmOutput {
                content: "final answer".to_string(),
                tool_calls: vec![],
                reasoning: None,
                usage: None,
                finish_reason: Default::default(),
            }),
        ])),
        requests: Arc::clone(&requests),
    });
    let tools = vec![boxed_tool(ReadFileMetadataTool {
        path: file_path.clone(),
        output: file_contents.clone(),
    })];
    let loop_runner = AgentLoop::from_capabilities(
        Arc::new(StaticProviderFactory { provider }),
        capabilities_from_tools(tools),
    )
    .with_auto_compact_enabled(true)
    .with_compact_threshold_percent(95)
    .with_compact_keep_recent_turns(1);
    let state = astrcode_core::AgentState {
        session_id: "test".into(),
        working_dir: temp_dir.path().to_path_buf(),
        messages: vec![
            LlmMessage::User {
                content: "legacy context that should become compressible later".repeat(12),
                origin: UserMessageOrigin::User,
            },
            LlmMessage::Assistant {
                content: "legacy ack".to_string(),
                tool_calls: vec![],
                reasoning: None,
            },
            LlmMessage::User {
                content: "please inspect this file and continue".to_string(),
                origin: UserMessageOrigin::User,
            },
        ],
        phase: Phase::Thinking,
        turn_count: 0,
    };
    let (_events, mut on_event) = collect_events();

    let outcome = loop_runner
        .run_turn(
            &state,
            "turn-reactive-file-recovery",
            &mut on_event,
            CancelToken::new(),
        )
        .await
        .expect("turn should complete");

    assert!(matches!(outcome, TurnOutcome::Completed));
    let requests = requests.lock().expect("request log lock");
    assert!(
        requests.len() >= 4,
        "reactive compact path should issue tool, failing prompt, compact, and final requests"
    );
    let final_request = requests.last().expect("final request should exist");
    let recovery_block = final_request
        .system_prompt_blocks
        .iter()
        .find(|block| {
            block.title == "Recovered Context"
                && block.content.contains("recovered-file:")
                && block.content.contains("[Post-compact file recovery:")
        })
        .expect("rebuilt request should include a recovered context prompt block");
    assert!(
        recovery_block.content.contains("recent.rs"),
        "recovery block should name the recently accessed file"
    );
    assert!(
        recovery_block.content.contains("recovered_context"),
        "recovery block should embed the file contents so the model keeps local code context"
    );
    assert!(
        final_request.messages.iter().all(|message| {
            !matches!(
                message,
                LlmMessage::User { content, .. } if content.contains("[Structured memory:")
            )
        }),
        "recovered context should stay out of the message history"
    );
}

#[tokio::test]
async fn manual_compact_event_uses_manual_trigger_via_compaction_runtime() {
    let _guard = super::test_support::TestEnvGuard::new();
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([LlmOutput {
            content: "<analysis>trimmed</analysis><summary>manual summary</summary>".to_string(),
            tool_calls: vec![],
            reasoning: None,
            usage: None,
            finish_reason: Default::default(),
        }])),
        delay: std::time::Duration::from_millis(0),
    });
    let loop_runner = AgentLoop::from_capabilities(
        Arc::new(StaticProviderFactory { provider }),
        empty_capabilities(),
    )
    .with_compact_keep_recent_turns(1);
    let state = astrcode_core::AgentState {
        session_id: "test".into(),
        working_dir: std::env::temp_dir(),
        messages: vec![
            LlmMessage::User {
                content: "legacy ".repeat(1_500),
                origin: UserMessageOrigin::User,
            },
            LlmMessage::Assistant {
                content: "ack".to_string(),
                tool_calls: vec![],
                reasoning: None,
            },
            LlmMessage::User {
                content: "current ask".to_string(),
                origin: UserMessageOrigin::User,
            },
        ],
        phase: Phase::Idle,
        turn_count: 1,
    };

    let event = loop_runner
        .manual_compact_event(&state, CompactionTailSnapshot::default(), None)
        .await
        .expect("manual compact should succeed")
        .expect("manual compact should emit an event");

    assert!(matches!(
        event,
        StorageEvent {
            payload:
                StorageEventPayload::CompactApplied {
                    trigger: astrcode_core::CompactTrigger::Manual,
                    summary,
                    ..
                },
            ..
        } if summary == "manual summary"
    ));
}

#[tokio::test]
async fn manual_compact_event_caps_keep_recent_turns_so_manual_requests_do_real_work() {
    let _guard = super::test_support::TestEnvGuard::new();
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([LlmOutput {
            content: "<analysis>trimmed</analysis><summary>manual summary</summary>".to_string(),
            tool_calls: vec![],
            reasoning: None,
            usage: None,
            finish_reason: Default::default(),
        }])),
        delay: std::time::Duration::from_millis(0),
    });
    let loop_runner = AgentLoop::from_capabilities(
        Arc::new(StaticProviderFactory { provider }),
        empty_capabilities(),
    )
    .with_compact_keep_recent_turns(4);
    let state = astrcode_core::AgentState {
        session_id: "test".into(),
        working_dir: std::env::temp_dir(),
        messages: vec![
            LlmMessage::User {
                content: "first ask".to_string(),
                origin: UserMessageOrigin::User,
            },
            LlmMessage::Assistant {
                content: "first answer".to_string(),
                tool_calls: vec![],
                reasoning: None,
            },
            LlmMessage::User {
                content: "second ask".to_string(),
                origin: UserMessageOrigin::User,
            },
            LlmMessage::Assistant {
                content: "second answer".to_string(),
                tool_calls: vec![],
                reasoning: None,
            },
            LlmMessage::User {
                content: "third ask".to_string(),
                origin: UserMessageOrigin::User,
            },
        ],
        phase: Phase::Idle,
        turn_count: 3,
    };

    let event = loop_runner
        .manual_compact_event(&state, CompactionTailSnapshot::default(), None)
        .await
        .expect("manual compact should succeed even when auto keep_recent_turns is larger")
        .expect("manual compact should still emit an event");

    assert!(matches!(
        event,
        StorageEvent {
            payload:
                StorageEventPayload::CompactApplied {
                    trigger: astrcode_core::CompactTrigger::Manual,
                    summary,
                    ..
                },
            ..
        } if summary == "manual summary"
    ));
}

#[tokio::test]
async fn manual_compact_event_returns_none_when_single_turn_has_no_safe_cut_point() {
    let _guard = super::test_support::TestEnvGuard::new();
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::new()),
        delay: std::time::Duration::from_millis(0),
    });
    let loop_runner = AgentLoop::from_capabilities(
        Arc::new(StaticProviderFactory { provider }),
        empty_capabilities(),
    )
    .with_compact_keep_recent_turns(4);
    let state = astrcode_core::AgentState {
        session_id: "test".into(),
        working_dir: std::env::temp_dir(),
        messages: vec![LlmMessage::User {
            content: "only ask".to_string(),
            origin: UserMessageOrigin::User,
        }],
        phase: Phase::Idle,
        turn_count: 1,
    };

    let event = loop_runner
        .manual_compact_event(&state, CompactionTailSnapshot::default(), None)
        .await
        .expect("single-turn sessions without assistant steps should not error");

    assert!(event.is_none());
}

#[tokio::test]
async fn manual_compact_event_allows_single_real_turn_when_assistant_step_boundary_exists() {
    let _guard = super::test_support::TestEnvGuard::new();
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([LlmOutput {
            content: "<analysis>trimmed</analysis><summary>manual summary</summary>".to_string(),
            tool_calls: vec![],
            reasoning: None,
            usage: None,
            finish_reason: Default::default(),
        }])),
        delay: std::time::Duration::from_millis(0),
    });
    let loop_runner = AgentLoop::from_capabilities(
        Arc::new(StaticProviderFactory { provider }),
        empty_capabilities(),
    )
    .with_compact_keep_recent_turns(4);
    let state = astrcode_core::AgentState {
        session_id: "test".into(),
        working_dir: std::env::temp_dir(),
        messages: vec![
            LlmMessage::User {
                content: "only ask".to_string(),
                origin: UserMessageOrigin::User,
            },
            LlmMessage::Assistant {
                content: "partial progress".to_string(),
                tool_calls: vec![],
                reasoning: None,
            },
        ],
        phase: Phase::Idle,
        turn_count: 1,
    };

    let event = loop_runner
        .manual_compact_event(&state, CompactionTailSnapshot::default(), None)
        .await
        .expect("single real turn should compact when assistant-step cut point exists")
        .expect("compact event should be emitted");

    assert!(matches!(
        event,
        StorageEvent {
            payload:
                StorageEventPayload::CompactApplied {
                    trigger: astrcode_core::CompactTrigger::Manual,
                    summary,
                    ..
                },
            ..
        } if summary == "manual summary"
    ));
}

#[tokio::test]
async fn manual_compact_event_recovers_files_from_recent_stored_events_not_only_tail() {
    let _guard = super::test_support::TestEnvGuard::new();
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let file_path = temp_dir.path().join("manual-recovery.rs");
    let file_contents = "fn manual_compact_recovery() {}\n".repeat(32);
    std::fs::write(&file_path, &file_contents).expect("fixture file should be writable");
    let recovered_files = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::new()),
        delay: std::time::Duration::from_millis(0),
    });
    let loop_runner = AgentLoop::from_capabilities(
        Arc::new(StaticProviderFactory { provider }),
        empty_capabilities(),
    )
    .with_compaction_runtime(CompactionRuntime::new(
        true,
        1,
        80,
        Arc::new(ThresholdCompactionPolicy::new(true)),
        Arc::new(StaticArtifactStrategy),
        Arc::new(RecordingRebuilder {
            recovered_files: Arc::clone(&recovered_files),
        }),
        Arc::new(FsFileContentProvider),
    ));
    let state = astrcode_core::AgentState {
        session_id: "test".into(),
        working_dir: temp_dir.path().to_path_buf(),
        messages: vec![
            LlmMessage::User {
                content: "first ask".to_string(),
                origin: UserMessageOrigin::User,
            },
            LlmMessage::Assistant {
                content: "first answer".to_string(),
                tool_calls: vec![],
                reasoning: None,
            },
            LlmMessage::User {
                content: "second ask".to_string(),
                origin: UserMessageOrigin::User,
            },
        ],
        phase: Phase::Idle,
        turn_count: 2,
    };
    let recent_stored_events = vec![astrcode_core::StoredEvent {
        storage_seq: 10,
        event: StorageEvent {
            turn_id: Some("turn-2".to_string()),
            agent: AgentEventContext::default(),
            payload: StorageEventPayload::ToolResult {
                tool_call_id: "call-read".to_string(),
                tool_name: "readFile".to_string(),
                output: file_contents.clone(),
                success: true,
                error: None,
                metadata: Some(json!({ "path": file_path.display().to_string() })),
                duration_ms: 1,
            },
        },
    }];

    let event = loop_runner
        .manual_compact_event(
            &state,
            CompactionTailSnapshot::default(),
            Some(&recent_stored_events),
        )
        .await
        .expect("manual compact should succeed")
        .expect("manual compact should emit an event");

    assert!(matches!(
        event,
        StorageEvent {
            payload: StorageEventPayload::CompactApplied { .. },
            ..
        }
    ));
    let recovered_files = recovered_files.lock().expect("recovered files lock");
    assert_eq!(recovered_files.len(), 1);
    assert_eq!(recovered_files[0].0, file_path);
    assert!(
        recovered_files[0].1.contains("manual_compact_recovery"),
        "manual compact should recover file contents from the recent durable window even when the \
         rebuild tail is empty"
    );
}

/// 构造一个 413 prompt too long 错误。
fn make_prompt_too_long_error() -> AstrError {
    AstrError::LlmRequestFailed {
        status: 413,
        body: "prompt too long for this model".to_string(),
    }
}

/// 构造一个不可恢复的客户端错误。
fn make_client_error() -> AstrError {
    AstrError::LlmRequestFailed {
        status: 401,
        body: "invalid api key".to_string(),
    }
}

/// P4.1: 413 错误时 turn 级别触发 reactive compact 并恢复。
#[tokio::test]
async fn p4_1_reactive_compact_recovers_from_413_error() {
    let _guard = super::test_support::TestEnvGuard::new();

    let provider = Arc::new(FailingProvider {
        results: Mutex::new(VecDeque::from([
            Err(make_prompt_too_long_error()),
            Ok(LlmOutput {
                content: "<analysis>trimmed</analysis><summary>condensed history</summary>"
                    .to_string(),
                tool_calls: vec![],
                reasoning: None,
                usage: None,
                finish_reason: Default::default(),
            }),
            Ok(LlmOutput {
                content: "recovered answer".to_string(),
                tool_calls: vec![],
                reasoning: None,
                usage: None,
                finish_reason: Default::default(),
            }),
        ])),
        delay: std::time::Duration::from_millis(0),
    });

    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, empty_capabilities())
        .with_auto_compact_enabled(true)
        .with_compact_threshold_percent(95)
        .with_compact_keep_recent_turns(1);

    let state = astrcode_core::AgentState {
        session_id: "test".into(),
        working_dir: std::env::temp_dir(),
        messages: vec![
            LlmMessage::User {
                content: "legacy ".repeat(1_500),
                origin: UserMessageOrigin::User,
            },
            LlmMessage::Assistant {
                content: "ack".to_string(),
                tool_calls: vec![],
                reasoning: None,
            },
            LlmMessage::User {
                content: "current ask".to_string(),
                origin: UserMessageOrigin::User,
            },
        ],
        phase: Phase::Thinking,
        turn_count: 0,
    };

    let (events, mut on_event) = collect_events();

    let outcome = loop_runner
        .run_turn(
            &state,
            "turn-413-recovery",
            &mut on_event,
            CancelToken::new(),
        )
        .await
        .expect("turn should complete after reactive compact");

    assert!(
        matches!(outcome, TurnOutcome::Completed),
        "turn should complete after recovering from 413"
    );

    let events = events.lock().expect("events lock");
    assert!(
        events
            .iter()
            .any(|event| { matches!(&event.payload, StorageEventPayload::CompactApplied { .. }) }),
        "should have emitted CompactApplied event during reactive compact"
    );
    assert!(
        events.iter().any(|event| {
            matches!(
                &event.payload,
                StorageEventPayload::AssistantFinal { content, .. } if content == "recovered answer"
            )
        }),
        "should have emitted AssistantFinal with recovered answer"
    );
}

#[tokio::test]
async fn reactive_compact_uses_policy_rewritten_system_prompt() {
    let _guard = super::test_support::TestEnvGuard::new();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(RecordingFailingProvider {
        results: Mutex::new(VecDeque::from([
            Err(make_prompt_too_long_error()),
            Ok(LlmOutput {
                content: "<analysis>trimmed</analysis><summary>condensed history</summary>"
                    .to_string(),
                tool_calls: vec![],
                reasoning: None,
                usage: None,
                finish_reason: Default::default(),
            }),
            Ok(LlmOutput {
                content: "recovered".to_string(),
                tool_calls: vec![],
                reasoning: None,
                usage: None,
                finish_reason: Default::default(),
            }),
        ])),
        requests: Arc::clone(&requests),
    });
    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, empty_capabilities())
        .with_policy_engine(Arc::new(RewriteSystemPromptPolicy {
            suffix: "policy suffix".to_string(),
        }))
        .with_auto_compact_enabled(true)
        .with_compact_threshold_percent(95)
        .with_compact_keep_recent_turns(1);
    let state = astrcode_core::AgentState {
        session_id: "test".into(),
        working_dir: std::env::temp_dir(),
        messages: vec![
            LlmMessage::User {
                content: "legacy ".repeat(1_500),
                origin: UserMessageOrigin::User,
            },
            LlmMessage::Assistant {
                content: "ack".to_string(),
                tool_calls: vec![],
                reasoning: None,
            },
            LlmMessage::User {
                content: "current ask".to_string(),
                origin: UserMessageOrigin::User,
            },
        ],
        phase: Phase::Thinking,
        turn_count: 0,
    };
    let (_events, mut on_event) = collect_events();

    let outcome = loop_runner
        .run_turn(
            &state,
            "turn-reactive-policy-prompt",
            &mut on_event,
            CancelToken::new(),
        )
        .await
        .expect("turn should complete");

    assert!(matches!(outcome, TurnOutcome::Completed));
    let requests = requests.lock().expect("requests lock");
    assert_eq!(requests.len(), 3);
    assert!(
        requests[1]
            .system_prompt
            .as_deref()
            .is_some_and(|prompt| prompt.contains("policy suffix")),
        "reactive compaction should reuse the policy-rewritten system prompt"
    );
}

#[tokio::test]
async fn compaction_event_is_not_emitted_when_rebuild_fails() {
    let _guard = super::test_support::TestEnvGuard::new();
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::new()),
        delay: std::time::Duration::from_millis(0),
    });
    let loop_runner = AgentLoop::from_capabilities(
        Arc::new(StaticProviderFactory { provider }),
        empty_capabilities(),
    )
    .with_compaction_runtime(CompactionRuntime::new(
        true,
        1,
        1,
        Arc::new(ThresholdCompactionPolicy::new(true)),
        Arc::new(StaticArtifactStrategy),
        Arc::new(FailingRebuilder),
        Arc::new(FsFileContentProvider),
    ));
    let state = astrcode_core::AgentState {
        session_id: "test".into(),
        working_dir: std::env::temp_dir(),
        messages: vec![
            LlmMessage::User {
                content: "legacy".to_string(),
                origin: UserMessageOrigin::User,
            },
            LlmMessage::User {
                content: "current ask".to_string(),
                origin: UserMessageOrigin::User,
            },
        ],
        phase: Phase::Thinking,
        turn_count: 0,
    };
    let (events, mut on_event) = collect_events();

    let outcome = loop_runner
        .run_turn(
            &state,
            "turn-rebuild-failure",
            &mut on_event,
            CancelToken::new(),
        )
        .await
        .expect("turn should return outcome");

    assert!(matches!(outcome, TurnOutcome::Error { .. }));
    let events = events.lock().expect("events lock");
    assert!(
        !events
            .iter()
            .any(|event| matches!(&event.payload, StorageEventPayload::CompactApplied { .. })),
        "compaction should not be emitted before the rebuilt conversation is proven valid"
    );
}

/// P4.1: 413 错误但无可压缩内容时，正确终止 turn 并报告错误。
#[tokio::test]
async fn p4_1_reactive_compact_fails_when_no_compressible_history() {
    let _guard = super::test_support::TestEnvGuard::new();

    let provider = Arc::new(FailingProvider {
        results: Mutex::new(VecDeque::from([Err(make_prompt_too_long_error())])),
        delay: std::time::Duration::from_millis(0),
    });

    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, empty_capabilities())
        .with_auto_compact_enabled(true)
        .with_compact_threshold_percent(95)
        .with_compact_keep_recent_turns(1);

    let state = astrcode_core::AgentState {
        session_id: "test".into(),
        working_dir: std::env::temp_dir(),
        messages: vec![LlmMessage::User {
            content: "short message".to_string(),
            origin: UserMessageOrigin::User,
        }],
        phase: Phase::Thinking,
        turn_count: 0,
    };

    let (_events, mut on_event) = collect_events();

    let outcome = loop_runner
        .run_turn(
            &state,
            "turn-413-no-history",
            &mut on_event,
            CancelToken::new(),
        )
        .await
        .expect("run_turn should return Ok(TurnOutcome) even on error");

    assert!(
        matches!(outcome, TurnOutcome::Error { .. }),
        "turn should end with Error outcome when no compressible history available, got: \
         {outcome:?}"
    );
}

/// P4.1: 连续 prompt-too-long 时，reactive compact 重试次数必须受上限保护。
#[tokio::test]
async fn p4_1_reactive_compact_stops_after_max_attempts() {
    let _guard = super::test_support::TestEnvGuard::new();

    let provider = Arc::new(FailingProvider {
        results: Mutex::new(VecDeque::from([
            Err(make_prompt_too_long_error()),
            Ok(LlmOutput {
                content: "<analysis>trimmed</analysis><summary>summary 1</summary>".to_string(),
                tool_calls: vec![],
                reasoning: None,
                usage: None,
                finish_reason: Default::default(),
            }),
            Err(make_prompt_too_long_error()),
            Ok(LlmOutput {
                content: "<analysis>trimmed</analysis><summary>summary 2</summary>".to_string(),
                tool_calls: vec![],
                reasoning: None,
                usage: None,
                finish_reason: Default::default(),
            }),
            Err(make_prompt_too_long_error()),
            Ok(LlmOutput {
                content: "<analysis>trimmed</analysis><summary>summary 3</summary>".to_string(),
                tool_calls: vec![],
                reasoning: None,
                usage: None,
                finish_reason: Default::default(),
            }),
            Err(make_prompt_too_long_error()),
        ])),
        delay: std::time::Duration::from_millis(0),
    });

    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, empty_capabilities())
        .with_auto_compact_enabled(true)
        .with_compact_threshold_percent(95)
        .with_compact_keep_recent_turns(1);

    let state = astrcode_core::AgentState {
        session_id: "test".into(),
        working_dir: std::env::temp_dir(),
        messages: vec![
            LlmMessage::User {
                content: "legacy ".repeat(1_500),
                origin: UserMessageOrigin::User,
            },
            LlmMessage::Assistant {
                content: "ack".to_string(),
                tool_calls: vec![],
                reasoning: None,
            },
            LlmMessage::User {
                content: "current ask".to_string(),
                origin: UserMessageOrigin::User,
            },
        ],
        phase: Phase::Thinking,
        turn_count: 0,
    };

    let (events, mut on_event) = collect_events();

    let outcome = loop_runner
        .run_turn(
            &state,
            "turn-413-max-attempts",
            &mut on_event,
            CancelToken::new(),
        )
        .await
        .expect("run_turn should return a turn outcome");

    assert!(
        matches!(outcome, TurnOutcome::Error { .. }),
        "reactive compact should stop with an error after exhausting retries, got: {outcome:?}"
    );

    let compact_count = events
        .lock()
        .expect("events lock")
        .iter()
        .filter(|event| matches!(&event.payload, StorageEventPayload::CompactApplied { .. }))
        .count();
    assert_eq!(
        compact_count, 3,
        "reactive compact should stop after exactly three retries"
    );
}

/// P4.1: 不可恢复的错误（如 401）不应触发 reactive compact，直接终止 turn。
#[tokio::test]
async fn p4_1_non_recoverable_error_does_not_trigger_compact() {
    let _guard = super::test_support::TestEnvGuard::new();

    let provider = Arc::new(FailingProvider {
        results: Mutex::new(VecDeque::from([Err(make_client_error())])),
        delay: std::time::Duration::from_millis(0),
    });

    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, empty_capabilities())
        .with_auto_compact_enabled(true)
        .with_compact_threshold_percent(95)
        .with_compact_keep_recent_turns(1);

    let state = astrcode_core::AgentState {
        session_id: "test".into(),
        working_dir: std::env::temp_dir(),
        messages: vec![
            LlmMessage::User {
                content: "legacy ".repeat(1_500),
                origin: UserMessageOrigin::User,
            },
            LlmMessage::User {
                content: "current ask".to_string(),
                origin: UserMessageOrigin::User,
            },
        ],
        phase: Phase::Thinking,
        turn_count: 0,
    };

    let (events, mut on_event) = collect_events();

    let outcome = loop_runner
        .run_turn(
            &state,
            "turn-client-error",
            &mut on_event,
            CancelToken::new(),
        )
        .await
        .expect("run_turn should return Ok(TurnOutcome) even on error");

    assert!(
        matches!(outcome, TurnOutcome::Error { .. }),
        "turn should end with Error outcome on non-recoverable client error, got: {outcome:?}"
    );

    let events = events.lock().expect("events lock");
    assert!(
        !events
            .iter()
            .any(|event| { matches!(&event.payload, StorageEventPayload::CompactApplied { .. }) }),
        "should NOT have emitted CompactApplied for non-recoverable error"
    );
}

// ---------------------------------------------------------------------------
// 新增：暴露 proactive compact 与 reactive compact 之间 system prompt 不一致问题
// ---------------------------------------------------------------------------

/// 验证 proactive compact 使用 policy 改写后的 system prompt。
///
/// 当前代码中，proactive compact（步骤⑥）在 policy check（步骤⑦）之前执行，
/// 因此它拿到的是改写前的 system prompt。而 reactive compact 正确使用了改写后的。
/// 这会导致 proactive compact 的摘要基于过时的指令，产生内容漂移。
#[tokio::test]
async fn proactive_compact_uses_policy_rewritten_system_prompt() {
    let _guard = super::test_support::TestEnvGuard::new();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(RecordingFailingProvider {
        results: Mutex::new(VecDeque::from([
            // 第一轮：用于 compact 的摘要生成请求
            Ok(LlmOutput {
                content: "<analysis>trimmed</analysis><summary>condensed history</summary>"
                    .to_string(),
                tool_calls: vec![],
                reasoning: None,
                usage: None,
                finish_reason: Default::default(),
            }),
            // 第二轮：最终回复
            Ok(LlmOutput {
                content: "final answer".to_string(),
                tool_calls: vec![],
                reasoning: None,
                usage: None,
                finish_reason: Default::default(),
            }),
        ])),
        requests: Arc::clone(&requests),
    });
    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, empty_capabilities())
        .with_policy_engine(Arc::new(RewriteSystemPromptPolicy {
            suffix: "CRITICAL POLICY CONTEXT".to_string(),
        }))
        .with_auto_compact_enabled(true)
        .with_compact_threshold_percent(1)
        .with_compact_keep_recent_turns(1);
    let state = astrcode_core::AgentState {
        session_id: "test".into(),
        working_dir: std::env::temp_dir(),
        messages: vec![
            LlmMessage::User {
                content: "legacy ".repeat(1_500),
                origin: UserMessageOrigin::User,
            },
            LlmMessage::Assistant {
                content: "ack".to_string(),
                tool_calls: vec![],
                reasoning: None,
            },
            LlmMessage::User {
                content: "current ask".to_string(),
                origin: UserMessageOrigin::User,
            },
        ],
        phase: Phase::Thinking,
        turn_count: 0,
    };
    let (_events, mut on_event) = collect_events();

    let outcome = loop_runner
        .run_turn(
            &state,
            "turn-proactive-policy-prompt",
            &mut on_event,
            CancelToken::new(),
        )
        .await
        .expect("turn should complete");

    assert!(
        matches!(outcome, TurnOutcome::Completed),
        "turn should complete, got: {outcome:?}"
    );
    let requests = requests.lock().expect("requests lock");
    // 第一个请求是 compact 摘要请求，第二个是最终模型请求
    assert!(
        requests.len() >= 2,
        "should have at least a compact request and a final request, got {}",
        requests.len()
    );
    let compact_request = &requests[0];
    // compact 请求的 system_prompt 应该包含 policy 追加的内容，
    // 否则摘要模型不知道完整的工作指令上下文，会丢失关键信息。
    let compact_system = compact_request.system_prompt.as_deref().unwrap_or("<none>");
    assert!(
        compact_system.contains("CRITICAL POLICY CONTEXT"),
        "proactive compact should use the policy-rewritten system prompt, but system_prompt was: \
         {compact_system}"
    );
}

/// 验证 proactive compact 之后 reactive compact 的计数器保持独立。
///
/// 如果 proactive compact 成功了但 LLM 仍然返回 prompt-too-long，
/// reactive compact 应该从 0 开始计数（而非继承之前的某种状态）。
#[tokio::test]
async fn reactive_compact_counter_starts_from_zero_after_proactive_compact() {
    let _guard = super::test_support::TestEnvGuard::new();
    // 阈值设到 1%，确保 proactive compact 一定触发。
    // 但 compact 后的摘要 + 尾部仍然可能触发 413（模拟极端场景）。
    let provider = Arc::new(FailingProvider {
        results: Mutex::new(VecDeque::from([
            // compact 摘要生成成功
            Ok(LlmOutput {
                content: "<analysis>trimmed</analysis><summary>short summary</summary>".to_string(),
                tool_calls: vec![],
                reasoning: None,
                usage: None,
                finish_reason: Default::default(),
            }),
            // compact 后第一次 LLM 调用失败 → 触发 reactive compact
            Err(make_prompt_too_long_error()),
            // reactive compact 摘要生成
            Ok(LlmOutput {
                content: "<analysis>trimmed</analysis><summary>even shorter</summary>".to_string(),
                tool_calls: vec![],
                reasoning: None,
                usage: None,
                finish_reason: Default::default(),
            }),
            // reactive compact 后 LLM 调用成功
            Ok(LlmOutput {
                content: "recovered".to_string(),
                tool_calls: vec![],
                reasoning: None,
                usage: None,
                finish_reason: Default::default(),
            }),
        ])),
        delay: std::time::Duration::from_millis(0),
    });
    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, empty_capabilities())
        .with_auto_compact_enabled(true)
        .with_compact_threshold_percent(1)
        .with_compact_keep_recent_turns(1);
    let state = astrcode_core::AgentState {
        session_id: "test".into(),
        working_dir: std::env::temp_dir(),
        messages: vec![
            LlmMessage::User {
                content: "legacy ".repeat(1_500),
                origin: UserMessageOrigin::User,
            },
            LlmMessage::Assistant {
                content: "ack".to_string(),
                tool_calls: vec![],
                reasoning: None,
            },
            LlmMessage::User {
                content: "current ask".to_string(),
                origin: UserMessageOrigin::User,
            },
        ],
        phase: Phase::Thinking,
        turn_count: 0,
    };
    let (events, mut on_event) = collect_events();

    let outcome = loop_runner
        .run_turn(
            &state,
            "turn-proactive-then-reactive",
            &mut on_event,
            CancelToken::new(),
        )
        .await
        .expect("turn should complete");

    assert!(
        matches!(outcome, TurnOutcome::Completed),
        "turn should recover via reactive compact after proactive compact, got: {outcome:?}"
    );
    let compact_count = events
        .lock()
        .expect("events lock")
        .iter()
        .filter(|event| matches!(&event.payload, StorageEventPayload::CompactApplied { .. }))
        .count();
    assert_eq!(
        compact_count, 2,
        "should have exactly two CompactApplied events (one proactive, one reactive)"
    );
}

/// 验证 auto_compact 关闭时 reactive compact 不会触发。
///
/// 当 auto_compact_enabled=false 时，即使 LLM 返回 413 prompt-too-long，
/// reactive compact 也应该直接终止 turn 而不是尝试恢复。
#[tokio::test]
async fn reactive_compact_does_not_fire_when_auto_compact_disabled() {
    let _guard = super::test_support::TestEnvGuard::new();
    let provider = Arc::new(FailingProvider {
        results: Mutex::new(VecDeque::from([Err(make_prompt_too_long_error())])),
        delay: std::time::Duration::from_millis(0),
    });
    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, empty_capabilities())
        .with_auto_compact_enabled(false)
        .with_compact_threshold_percent(95)
        .with_compact_keep_recent_turns(1);
    let state = astrcode_core::AgentState {
        session_id: "test".into(),
        working_dir: std::env::temp_dir(),
        messages: vec![
            LlmMessage::User {
                content: "legacy ".repeat(1_500),
                origin: UserMessageOrigin::User,
            },
            LlmMessage::Assistant {
                content: "ack".to_string(),
                tool_calls: vec![],
                reasoning: None,
            },
            LlmMessage::User {
                content: "current ask".to_string(),
                origin: UserMessageOrigin::User,
            },
        ],
        phase: Phase::Thinking,
        turn_count: 0,
    };
    let (events, mut on_event) = collect_events();

    let outcome = loop_runner
        .run_turn(
            &state,
            "turn-disabled-auto-compact-413",
            &mut on_event,
            CancelToken::new(),
        )
        .await
        .expect("run_turn should return Ok(TurnOutcome) even on error");

    assert!(
        matches!(outcome, TurnOutcome::Error { .. }),
        "turn should end with Error when auto_compact is disabled and LLM returns 413, got: \
         {outcome:?}"
    );
    let compact_count = events
        .lock()
        .expect("events lock")
        .iter()
        .filter(|event| matches!(&event.payload, StorageEventPayload::CompactApplied { .. }))
        .count();
    assert_eq!(
        compact_count, 0,
        "should NOT emit any CompactApplied when auto_compact is disabled"
    );
}

/// 验证 proactive compact 成功后 step_index 不会增长。
///
/// compact 只是替换了对话历史，当前 step 仍然在处理同一个用户请求，
/// 因此 step_index 应保持不变直到工具执行完成。
#[tokio::test]
async fn proactive_compact_does_not_increment_step_index() {
    let _guard = super::test_support::TestEnvGuard::new();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(RecordingFailingProvider {
        results: Mutex::new(VecDeque::from([
            // compact 摘要
            Ok(LlmOutput {
                content: "<analysis>trimmed</analysis><summary>summary</summary>".to_string(),
                tool_calls: vec![],
                reasoning: None,
                usage: None,
                finish_reason: Default::default(),
            }),
            // 最终回复
            Ok(LlmOutput {
                content: "done".to_string(),
                tool_calls: vec![],
                reasoning: None,
                usage: None,
                finish_reason: Default::default(),
            }),
        ])),
        requests: Arc::clone(&requests),
    });
    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, empty_capabilities())
        .with_auto_compact_enabled(true)
        .with_compact_threshold_percent(1)
        .with_compact_keep_recent_turns(1);
    let state = astrcode_core::AgentState {
        session_id: "test".into(),
        working_dir: std::env::temp_dir(),
        messages: vec![
            LlmMessage::User {
                content: "legacy ".repeat(1_500),
                origin: UserMessageOrigin::User,
            },
            LlmMessage::Assistant {
                content: "ack".to_string(),
                tool_calls: vec![],
                reasoning: None,
            },
            LlmMessage::User {
                content: "current ask".to_string(),
                origin: UserMessageOrigin::User,
            },
        ],
        phase: Phase::Thinking,
        turn_count: 0,
    };
    let (events, mut on_event) = collect_events();

    let outcome = loop_runner
        .run_turn(
            &state,
            "turn-step-index-check",
            &mut on_event,
            CancelToken::new(),
        )
        .await
        .expect("turn should complete");

    assert!(matches!(outcome, TurnOutcome::Completed));
    // 检查 PromptMetrics 事件中的 step_index 顺序
    let events = events.lock().expect("events lock");
    let step_indices: Vec<u32> = events
        .iter()
        .filter_map(|event| match &event.payload {
            StorageEventPayload::PromptMetrics { metrics } => Some(metrics.step_index),
            _ => None,
        })
        .collect();
    assert!(
        !step_indices.is_empty(),
        "should have emitted at least one PromptMetrics event"
    );
    // proactive compact 不应导致 step_index 跳跃
    // 预期序列：[0, 0] 或 [0]（compact 前和最终请求，都是 step 0）
    for (i, index) in step_indices.iter().enumerate() {
        assert_eq!(
            *index, 0,
            "step_index at position {i} should be 0 (proactive compact should not increment it), \
             got {index}"
        );
    }
}
