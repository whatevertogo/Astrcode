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
    AstrError, CancelToken, LlmMessage, Phase, StorageEvent, Tool, ToolDefinition,
    ToolExecutionResult, ToolRegistry, UserMessageOrigin,
};
use astrcode_runtime_llm::{EventSink, LlmOutput, LlmProvider, LlmRequest, ModelLimits};
use async_trait::async_trait;
use serde_json::json;

use super::{
    fixtures::*,
    test_support::{capabilities_from_tools, empty_capabilities},
};
use crate::{
    AgentLoop,
    agent_loop::TurnOutcome,
    compaction_runtime::{
        CompactionArtifact, CompactionInput, CompactionReason, CompactionRuntime,
        CompactionTailSnapshot, FsFileContentProvider, ThresholdCompactionPolicy,
    },
    context_pipeline::ConversationView,
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
    ) -> astrcode_core::Result<ConversationView> {
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
    ) -> astrcode_core::Result<ConversationView> {
        *self.recovered_files.lock().expect("recovered files lock") = file_contents.to_vec();
        Ok(ConversationView::new(Vec::new()))
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
        Some(StorageEvent::PromptMetrics { .. })
    ));
    assert!(events.iter().any(|event| {
        matches!(
            event,
            StorageEvent::CompactApplied { summary, .. } if summary == "condensed history"
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            event,
            StorageEvent::AssistantFinal { content, .. } if content == "final answer"
        )
    }));
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
    let tools = ToolRegistry::builder()
        .register(Box::new(ReadFileMetadataTool {
            path: file_path.clone(),
            output: file_contents.clone(),
        }))
        .build();
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
    let final_request_messages = &requests
        .last()
        .expect("final request should exist")
        .messages;
    let recovery_message = final_request_messages
        .iter()
        .find_map(|message| match message {
            LlmMessage::User {
                content,
                origin: UserMessageOrigin::CompactSummary,
            } if content.contains("[Post-compact file recovery:") => Some(content.as_str()),
            _ => None,
        })
        .expect("rebuilt request should include a post-compact file recovery message");
    assert!(
        recovery_message.contains("recent.rs"),
        "recovery message should name the recently accessed file"
    );
    assert!(
        recovery_message.contains("recovered_context"),
        "recovery message should embed the file contents so the model keeps local code context"
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
        StorageEvent::CompactApplied {
            trigger: astrcode_core::CompactTrigger::Manual,
            ref summary,
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
        StorageEvent::CompactApplied {
            trigger: astrcode_core::CompactTrigger::Manual,
            ref summary,
            ..
        } if summary == "manual summary"
    ));
}

#[tokio::test]
async fn manual_compact_event_rejects_single_turn_sessions() {
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

    let error = loop_runner
        .manual_compact_event(&state, CompactionTailSnapshot::default(), None)
        .await
        .expect_err("single-turn sessions should reject manual compact");

    assert!(matches!(error, AstrError::Validation(_)));
    assert!(error.to_string().contains("at least 2 real user turns"));
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
        event: StorageEvent::ToolResult {
            turn_id: Some("turn-2".to_string()),
            tool_call_id: "call-read".to_string(),
            tool_name: "readFile".to_string(),
            output: file_contents.clone(),
            success: true,
            error: None,
            metadata: Some(json!({ "path": file_path.display().to_string() })),
            duration_ms: 1,
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

    assert!(matches!(event, StorageEvent::CompactApplied { .. }));
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
            .any(|event| { matches!(event, StorageEvent::CompactApplied { .. }) }),
        "should have emitted CompactApplied event during reactive compact"
    );
    assert!(
        events.iter().any(|event| {
            matches!(
                event,
                StorageEvent::AssistantFinal { content, .. } if content == "recovered answer"
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
            .any(|event| matches!(event, StorageEvent::CompactApplied { .. })),
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
        .filter(|event| matches!(event, StorageEvent::CompactApplied { .. }))
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
            .any(|event| { matches!(event, StorageEvent::CompactApplied { .. }) }),
        "should NOT have emitted CompactApplied for non-recoverable error"
    );
}
