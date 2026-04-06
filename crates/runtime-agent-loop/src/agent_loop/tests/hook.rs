//! Lifecycle hook 集成测试。

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use astrcode_core::{
    AgentState, CancelToken, HookEvent, LlmMessage, StorageEvent, ToolCallRequest,
    UserMessageOrigin,
};
use astrcode_runtime_llm::LlmOutput;
use serde_json::json;

use super::{fixtures::*, test_support::capabilities_from_tools};
use crate::{
    AgentLoop,
    agent_loop::TurnOutcome,
    compaction_runtime::{
        CompactionArtifact, CompactionReason, CompactionRuntime, CompactionStrategy,
        ConversationViewRebuilder, EventRange, FsFileContentProvider, ThresholdCompactionPolicy,
    },
};

struct StaticArtifactStrategy;

#[async_trait::async_trait]
impl CompactionStrategy for StaticArtifactStrategy {
    async fn compact(
        &self,
        _input: crate::compaction_runtime::CompactionInput<'_>,
    ) -> astrcode_core::Result<Option<CompactionArtifact>> {
        Ok(Some(CompactionArtifact {
            summary: "static summary".to_string(),
            source_range: EventRange { start: 0, end: 1 },
            preserved_tail_start: 1,
            strategy_id: "test".to_string(),
            pre_tokens: 100,
            post_tokens_estimate: 40,
            compacted_at_seq: 0,
            trigger: CompactionReason::Manual,
            preserved_recent_turns: 1,
            messages_removed: 1,
            tokens_freed: 60,
            recovered_files: Vec::new(),
        }))
    }
}

#[tokio::test]
async fn pre_tool_hook_can_rewrite_args_and_post_success_hook_sees_final_payload() {
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([
            LlmOutput {
                content: String::new(),
                tool_calls: vec![ToolCallRequest {
                    id: "call-echo".to_string(),
                    name: "echoArgsTool".to_string(),
                    args: json!({ "value": "old" }),
                }],
                reasoning: None,
                usage: None,
                finish_reason: Default::default(),
            },
            LlmOutput {
                content: "done".to_string(),
                tool_calls: vec![],
                reasoning: None,
                usage: None,
                finish_reason: Default::default(),
            },
        ])),
        delay: std::time::Duration::from_millis(0),
    });
    let post_hits = Arc::new(Mutex::new(Vec::new()));
    let tools = astrcode_runtime_registry::ToolRegistry::builder()
        .register(Box::new(EchoArgsTool))
        .build();
    let loop_runner = AgentLoop::from_capabilities(
        Arc::new(StaticProviderFactory { provider }),
        capabilities_from_tools(tools),
    )
    .with_hook_handler(Arc::new(ReplaceArgsHook {
        tool_name: "echoArgsTool",
        replacement: json!({ "value": "new" }),
    }))
    .with_hook_handler(Arc::new(RecordingToolHook {
        event: HookEvent::PostToolUse,
        hits: Arc::clone(&post_hits),
    }));

    let (events, mut on_event) = collect_events();
    let outcome = loop_runner
        .run_turn(
            &make_state("rewrite args"),
            "turn-hook-success",
            &mut on_event,
            CancelToken::new(),
        )
        .await
        .expect("turn should complete");

    assert_eq!(outcome, TurnOutcome::Completed);
    let events = events.lock().expect("events lock");
    assert!(events.iter().any(|event| {
        matches!(
            event,
            StorageEvent::ToolCall { args, .. } if args == &json!({ "value": "new" })
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            event,
            StorageEvent::ToolResult { output, .. } if output == "{\"value\":\"new\"}"
        )
    }));

    let post_hits = post_hits.lock().expect("post hook hits");
    assert_eq!(post_hits.len(), 1);
    assert_eq!(post_hits[0].tool.args, json!({ "value": "new" }));
    assert_eq!(post_hits[0].result.output, "{\"value\":\"new\"}");
}

#[tokio::test]
async fn pre_tool_hook_can_block_tool_execution_without_running_the_tool() {
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([
            LlmOutput {
                content: String::new(),
                tool_calls: vec![ToolCallRequest {
                    id: "call-blocked".to_string(),
                    name: "policyTool".to_string(),
                    args: json!({}),
                }],
                reasoning: None,
                usage: None,
                finish_reason: Default::default(),
            },
            LlmOutput {
                content: "done".to_string(),
                tool_calls: vec![],
                reasoning: None,
                usage: None,
                finish_reason: Default::default(),
            },
        ])),
        delay: std::time::Duration::from_millis(0),
    });
    let executions = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let tools = astrcode_runtime_registry::ToolRegistry::builder()
        .register(Box::new(CountingTool {
            executions: Arc::clone(&executions),
        }))
        .build();
    let loop_runner = AgentLoop::from_capabilities(
        Arc::new(StaticProviderFactory { provider }),
        capabilities_from_tools(tools),
    )
    .with_hook_handler(Arc::new(BlockingToolHook {
        tool_name: "policyTool",
        reason: "blocked by hook policy",
    }));

    let (events, mut on_event) = collect_events();
    let outcome = loop_runner
        .run_turn(
            &make_state("block tool"),
            "turn-hook-block",
            &mut on_event,
            CancelToken::new(),
        )
        .await
        .expect("turn should complete");

    assert_eq!(outcome, TurnOutcome::Completed);
    assert_eq!(executions.load(std::sync::atomic::Ordering::SeqCst), 0);
    let events = events.lock().expect("events lock");
    assert!(events.iter().any(|event| {
        matches!(
            event,
            StorageEvent::ToolResult { error, success, .. }
                if !success && error.as_deref() == Some("hook 'blocking-tool-hook' blocked tool call: blocked by hook policy")
        )
    }));
}

#[tokio::test]
async fn post_tool_failure_hook_observes_failed_tool_results() {
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([
            LlmOutput {
                content: String::new(),
                tool_calls: vec![ToolCallRequest {
                    id: "call-failure".to_string(),
                    name: "failingExecutionTool".to_string(),
                    args: json!({}),
                }],
                reasoning: None,
                usage: None,
                finish_reason: Default::default(),
            },
            LlmOutput {
                content: "done".to_string(),
                tool_calls: vec![],
                reasoning: None,
                usage: None,
                finish_reason: Default::default(),
            },
        ])),
        delay: std::time::Duration::from_millis(0),
    });
    let failure_hits = Arc::new(Mutex::new(Vec::new()));
    let tools = astrcode_runtime_registry::ToolRegistry::builder()
        .register(Box::new(FailingExecutionTool))
        .build();
    let loop_runner = AgentLoop::from_capabilities(
        Arc::new(StaticProviderFactory { provider }),
        capabilities_from_tools(tools),
    )
    .with_hook_handler(Arc::new(RecordingToolHook {
        event: HookEvent::PostToolUseFailure,
        hits: Arc::clone(&failure_hits),
    }));

    let outcome = loop_runner
        .run_turn(
            &make_state("observe tool failure"),
            "turn-hook-failure",
            &mut |_event| Ok(()),
            CancelToken::new(),
        )
        .await
        .expect("turn should complete");

    assert_eq!(outcome, TurnOutcome::Completed);
    let failure_hits = failure_hits.lock().expect("failure hook hits");
    assert_eq!(failure_hits.len(), 1);
    assert!(!failure_hits[0].result.ok);
    assert_eq!(failure_hits[0].result.error.as_deref(), Some("tool failed"));
}

#[tokio::test]
async fn manual_compact_runs_pre_and_post_compact_hooks() {
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::new()),
        delay: std::time::Duration::from_millis(0),
    });
    let pre_hits = Arc::new(Mutex::new(Vec::new()));
    let post_hits = Arc::new(Mutex::new(Vec::new()));
    let tools = astrcode_runtime_registry::ToolRegistry::builder()
        .register(Box::new(EchoArgsTool))
        .build();
    let loop_runner = AgentLoop::from_capabilities(
        Arc::new(StaticProviderFactory { provider }),
        capabilities_from_tools(tools),
    )
    .with_compaction_runtime(CompactionRuntime::new(
        true,
        2,
        80,
        Arc::new(ThresholdCompactionPolicy::new(true)),
        Arc::new(StaticArtifactStrategy),
        Arc::new(ConversationViewRebuilder),
        Arc::new(FsFileContentProvider),
    ))
    .with_hook_handler(Arc::new(RecordingCompactHook {
        event: HookEvent::PreCompact,
        pre_hits: Arc::clone(&pre_hits),
        post_hits: Arc::clone(&post_hits),
    }))
    .with_hook_handler(Arc::new(RecordingCompactHook {
        event: HookEvent::PostCompact,
        pre_hits: Arc::clone(&pre_hits),
        post_hits: Arc::clone(&post_hits),
    }));

    let state = AgentState {
        session_id: "session-hook-compact".to_string(),
        working_dir: std::env::temp_dir(),
        messages: vec![
            LlmMessage::User {
                content: "turn-1".to_string(),
                origin: UserMessageOrigin::User,
            },
            LlmMessage::Assistant {
                content: "reply-1".to_string(),
                tool_calls: Vec::new(),
                reasoning: None,
            },
            LlmMessage::User {
                content: "turn-2".to_string(),
                origin: UserMessageOrigin::User,
            },
        ],
        phase: astrcode_core::Phase::Thinking,
        turn_count: 2,
    };

    let event = loop_runner
        .manual_compact_event(
            &state,
            crate::CompactionTailSnapshot::from_messages(&state.messages, 1),
            None,
        )
        .await
        .expect("manual compact should succeed");

    assert!(matches!(event, Some(StorageEvent::CompactApplied { .. })));
    let pre_hits = pre_hits.lock().expect("pre hits");
    assert_eq!(pre_hits.len(), 1);
    assert_eq!(
        pre_hits[0].reason,
        astrcode_core::HookCompactionReason::Manual
    );
    assert_eq!(
        pre_hits[0].messages, state.messages,
        "manual compact 的 pre-hook 应该看到完整消息，而不是精简后的空上下文"
    );
    assert_eq!(
        pre_hits[0].tools.len(),
        1,
        "manual compact 的 pre-hook 应该暴露当前可见工具列表"
    );
    assert_eq!(
        pre_hits[0].tools[0].name, "echoArgsTool",
        "manual compact 应向 hook 传递真实的工具 surface"
    );
    assert_eq!(
        pre_hits[0].system_prompt, None,
        "手动 compact 当前没有额外 system prompt，应显式传 None 而不是遗漏字段"
    );
    let post_hits = post_hits.lock().expect("post hits");
    assert_eq!(post_hits.len(), 1);
    assert_eq!(post_hits[0].summary, "static summary");
}
