//! 工具执行相关测试。
//!
//! 覆盖：
//! - 工具事件顺序
//! - 流式工具输出
//! - 并发安全工具并行执行
//! - 非安全工具顺序执行
//! - 并发上限限制
//! - 并行工具结果顺序保持
//! - 长工具链完成

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use astrcode_core::{CancelToken, StorageEvent, ToolCallRequest};
use astrcode_runtime_llm::LlmOutput;
use serde_json::json;

use super::{fixtures::*, test_support::capabilities_from_tools};
use crate::{AgentLoop, agent_loop::TurnOutcome};

#[tokio::test]
async fn tool_events_are_ordered_and_turn_finishes() {
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([
            LlmOutput {
                content: "".to_string(),
                tool_calls: vec![ToolCallRequest {
                    id: "call1".to_string(),
                    name: "quickTool".to_string(),
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

    let tools = astrcode_runtime_registry::ToolRegistry::builder()
        .register(Box::new(QuickTool))
        .build();
    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, capabilities_from_tools(tools));
    let state = make_state("list files");

    let (events, mut on_event) = collect_events();

    let outcome = loop_runner
        .run_turn(&state, "turn-1", &mut on_event, CancelToken::new())
        .await
        .expect("turn should complete");
    assert_eq!(outcome, TurnOutcome::Completed);

    let events = events.lock().expect("lock").clone();
    let start_idx = events
        .iter()
        .position(|e| matches!(e, StorageEvent::ToolCall { .. }))
        .expect("ToolCall event expected");
    let result_idx = events
        .iter()
        .position(|e| matches!(e, StorageEvent::ToolResult { .. }))
        .expect("ToolResult event expected");
    let done_idx = events
        .iter()
        .position(|e| matches!(e, StorageEvent::TurnDone { .. }))
        .expect("TurnDone event expected");

    assert!(start_idx < result_idx);
    assert!(result_idx < done_idx);
    assert!(matches!(
        &events[done_idx],
        StorageEvent::TurnDone { reason, .. } if reason.as_deref() == Some("completed")
    ));
}

#[tokio::test]
async fn streaming_tool_emits_deltas_before_tool_result() {
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([
            LlmOutput {
                content: "".to_string(),
                tool_calls: vec![ToolCallRequest {
                    id: "call-stream".to_string(),
                    name: "streamingTool".to_string(),
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

    let tools = astrcode_runtime_registry::ToolRegistry::builder()
        .register(Box::new(StreamingTool))
        .build();
    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, capabilities_from_tools(tools));
    let state = make_state("stream tool");

    let (events, mut on_event) = collect_events();

    let outcome = loop_runner
        .run_turn(&state, "turn-stream", &mut on_event, CancelToken::new())
        .await
        .expect("turn should complete");
    assert_eq!(outcome, TurnOutcome::Completed);

    let events = events.lock().expect("lock").clone();
    let call_idx = events
        .iter()
        .position(|event| matches!(event, StorageEvent::ToolCall { .. }))
        .expect("tool call event expected");
    let first_delta_idx = events
        .iter()
        .position(|event| matches!(event, StorageEvent::ToolCallDelta { .. }))
        .expect("tool call delta event expected");
    let result_idx = events
        .iter()
        .position(|event| matches!(event, StorageEvent::ToolResult { .. }))
        .expect("tool result event expected");

    assert!(call_idx < first_delta_idx);
    assert!(first_delta_idx < result_idx);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, StorageEvent::ToolCallDelta { .. }))
            .count(),
        2,
        "streaming tool should emit both stdout and stderr deltas"
    );
    assert!(matches!(
        &events[first_delta_idx],
        StorageEvent::ToolCallDelta {
            tool_name,
            delta,
            ..
        } if tool_name == "streamingTool" && delta == "stdout line\n"
    ));
}

#[tokio::test]
async fn concurrency_safe_tools_run_in_parallel() {
    let tracker = Arc::new(ConcurrencyTracker::default());
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([
            LlmOutput {
                content: String::new(),
                tool_calls: vec![
                    ToolCallRequest {
                        id: "call-safe-1".to_string(),
                        name: "parallelSafeTool".to_string(),
                        args: json!({ "delayMs": 120 }),
                    },
                    ToolCallRequest {
                        id: "call-safe-2".to_string(),
                        name: "parallelSafeTool".to_string(),
                        args: json!({ "delayMs": 120 }),
                    },
                ],
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
    let tools = astrcode_runtime_registry::ToolRegistry::builder()
        .register(Box::new(ConcurrencyTrackingTool {
            name: "parallelSafeTool",
            concurrency_safe: true,
            tracker: Arc::clone(&tracker),
        }))
        .build();
    let loop_runner = AgentLoop::from_capabilities(
        Arc::new(StaticProviderFactory { provider }),
        capabilities_from_tools(tools),
    );

    loop_runner
        .run_turn(
            &make_state("run parallel safe tools"),
            "turn-parallel-safe",
            &mut |_event| Ok(()),
            CancelToken::new(),
        )
        .await
        .expect("turn should complete");

    assert_eq!(tracker.started.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert!(
        tracker.max_active.load(std::sync::atomic::Ordering::SeqCst) >= 2,
        "safe tools should overlap in execution"
    );
}

#[tokio::test]
async fn unsafe_tools_remain_sequential() {
    let tracker = Arc::new(ConcurrencyTracker::default());
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([
            LlmOutput {
                content: String::new(),
                tool_calls: vec![
                    ToolCallRequest {
                        id: "call-unsafe-1".to_string(),
                        name: "sequentialUnsafeTool".to_string(),
                        args: json!({ "delayMs": 80 }),
                    },
                    ToolCallRequest {
                        id: "call-unsafe-2".to_string(),
                        name: "sequentialUnsafeTool".to_string(),
                        args: json!({ "delayMs": 80 }),
                    },
                ],
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
    let tools = astrcode_runtime_registry::ToolRegistry::builder()
        .register(Box::new(ConcurrencyTrackingTool {
            name: "sequentialUnsafeTool",
            concurrency_safe: false,
            tracker: Arc::clone(&tracker),
        }))
        .build();
    let loop_runner = AgentLoop::from_capabilities(
        Arc::new(StaticProviderFactory { provider }),
        capabilities_from_tools(tools),
    );

    loop_runner
        .run_turn(
            &make_state("run unsafe tools"),
            "turn-sequential-unsafe",
            &mut |_event| Ok(()),
            CancelToken::new(),
        )
        .await
        .expect("turn should complete");

    assert_eq!(tracker.started.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(
        tracker.max_active.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "unsafe tools must never overlap"
    );
}

#[tokio::test]
async fn max_tool_concurrency_limits_safe_parallelism() {
    let tracker = Arc::new(ConcurrencyTracker::default());
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([
            LlmOutput {
                content: String::new(),
                tool_calls: vec![
                    ToolCallRequest {
                        id: "call-limit-1".to_string(),
                        name: "limitedSafeTool".to_string(),
                        args: json!({ "delayMs": 80 }),
                    },
                    ToolCallRequest {
                        id: "call-limit-2".to_string(),
                        name: "limitedSafeTool".to_string(),
                        args: json!({ "delayMs": 80 }),
                    },
                ],
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
    let tools = astrcode_runtime_registry::ToolRegistry::builder()
        .register(Box::new(ConcurrencyTrackingTool {
            name: "limitedSafeTool",
            concurrency_safe: true,
            tracker: Arc::clone(&tracker),
        }))
        .build();
    let loop_runner = AgentLoop::from_capabilities(
        Arc::new(StaticProviderFactory { provider }),
        capabilities_from_tools(tools),
    )
    .with_max_tool_concurrency(1);

    loop_runner
        .run_turn(
            &make_state("limit safe concurrency"),
            "turn-limit-safe",
            &mut |_event| Ok(()),
            CancelToken::new(),
        )
        .await
        .expect("turn should complete");

    assert_eq!(
        tracker.max_active.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "configured concurrency limit should cap safe tool overlap"
    );
}

#[tokio::test]
async fn parallel_safe_tool_results_preserve_original_request_order() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let tracker = Arc::new(ConcurrencyTracker::default());
    let provider = Arc::new(RecordingProvider {
        responses: Mutex::new(VecDeque::from([
            LlmOutput {
                content: String::new(),
                tool_calls: vec![
                    ToolCallRequest {
                        id: "call-ordered-slow".to_string(),
                        name: "orderedSafeTool".to_string(),
                        args: json!({ "delayMs": 120 }),
                    },
                    ToolCallRequest {
                        id: "call-ordered-fast".to_string(),
                        name: "orderedSafeTool".to_string(),
                        args: json!({ "delayMs": 20 }),
                    },
                ],
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
        requests: Arc::clone(&requests),
    });
    let tools = astrcode_runtime_registry::ToolRegistry::builder()
        .register(Box::new(ConcurrencyTrackingTool {
            name: "orderedSafeTool",
            concurrency_safe: true,
            tracker,
        }))
        .build();
    let loop_runner = AgentLoop::from_capabilities(
        Arc::new(StaticProviderFactory { provider }),
        capabilities_from_tools(tools),
    );

    loop_runner
        .run_turn(
            &make_state("preserve tool order"),
            "turn-ordered-safe",
            &mut |_event| Ok(()),
            CancelToken::new(),
        )
        .await
        .expect("turn should complete");

    let requests = requests.lock().expect("recorded requests lock");
    let tool_messages = requests[1]
        .messages
        .iter()
        .filter_map(|message| match message {
            astrcode_core::LlmMessage::Tool { tool_call_id, .. } => Some(tool_call_id.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(
        tool_messages,
        vec![
            "call-ordered-slow".to_string(),
            "call-ordered-fast".to_string()
        ]
    );
}

#[tokio::test]
async fn parallel_safe_tool_results_stream_before_slower_peers_finish() {
    let tracker = Arc::new(ConcurrencyTracker::default());
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([
            LlmOutput {
                content: String::new(),
                tool_calls: vec![
                    ToolCallRequest {
                        id: "call-live-slow".to_string(),
                        name: "liveSafeTool".to_string(),
                        args: json!({ "delayMs": 120 }),
                    },
                    ToolCallRequest {
                        id: "call-live-fast".to_string(),
                        name: "liveSafeTool".to_string(),
                        args: json!({ "delayMs": 20 }),
                    },
                ],
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
    let tools = astrcode_runtime_registry::ToolRegistry::builder()
        .register(Box::new(ConcurrencyTrackingTool {
            name: "liveSafeTool",
            concurrency_safe: true,
            tracker,
        }))
        .build();
    let loop_runner = AgentLoop::from_capabilities(
        Arc::new(StaticProviderFactory { provider }),
        capabilities_from_tools(tools),
    );
    let (events, mut on_event) = collect_events();

    loop_runner
        .run_turn(
            &make_state("stream safe tool results"),
            "turn-live-safe",
            &mut on_event,
            CancelToken::new(),
        )
        .await
        .expect("turn should complete");

    let events = events.lock().expect("events lock").clone();
    let fast_result_idx = events
        .iter()
        .position(|event| {
            matches!(
                event,
                StorageEvent::ToolResult {
                    tool_call_id,
                    tool_name,
                    ..
                } if tool_name == "liveSafeTool" && tool_call_id == "call-live-fast"
            )
        })
        .expect("fast tool result event expected");
    let slow_result_idx = events
        .iter()
        .position(|event| {
            matches!(
                event,
                StorageEvent::ToolResult {
                    tool_call_id,
                    tool_name,
                    ..
                } if tool_name == "liveSafeTool" && tool_call_id == "call-live-slow"
            )
        })
        .expect("slow tool result event expected");

    assert!(
        fast_result_idx < slow_result_idx,
        "fast safe tool result should be emitted before the slower peer finishes"
    );
}

#[tokio::test]
async fn long_tool_chains_complete_without_a_step_cap() {
    let mut scripted = (0..8)
        .map(|idx| LlmOutput {
            content: format!("step-{idx}"),
            tool_calls: vec![ToolCallRequest {
                id: format!("call-{idx}"),
                name: "quickTool".to_string(),
                args: json!({}),
            }],
            reasoning: None,
            usage: None,
            finish_reason: Default::default(),
        })
        .collect::<Vec<_>>();
    scripted.push(LlmOutput {
        content: "done".to_string(),
        tool_calls: vec![],
        reasoning: None,
        usage: None,
        finish_reason: Default::default(),
    });

    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from(scripted)),
        delay: std::time::Duration::from_millis(0),
    });

    let tools = astrcode_runtime_registry::ToolRegistry::builder()
        .register(Box::new(QuickTool))
        .build();

    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, capabilities_from_tools(tools));
    let state = make_state("loop test");

    let (events, mut on_event) = collect_events();

    let outcome = loop_runner
        .run_turn(&state, "turn-4", &mut on_event, CancelToken::new())
        .await
        .expect("turn should complete");
    assert_eq!(outcome, TurnOutcome::Completed);

    let events = events.lock().expect("lock").clone();
    let tool_results = events
        .iter()
        .filter(|event| matches!(event, StorageEvent::ToolResult { .. }))
        .count();
    let has_turn_done = events.iter().any(|event| {
        matches!(
            event,
            StorageEvent::TurnDone { reason, .. } if reason.as_deref() == Some("completed")
        )
    });

    assert_eq!(tool_results, 8, "every scripted tool call should complete");
    assert!(
        has_turn_done,
        "completed turns should carry the completed reason"
    );
}

#[tokio::test]
async fn turn_done_event_is_emitted_once_for_multi_step_tool_turn() {
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([
            LlmOutput {
                content: "step-1".to_string(),
                tool_calls: vec![ToolCallRequest {
                    id: "call-once-1".to_string(),
                    name: "quickTool".to_string(),
                    args: json!({}),
                }],
                reasoning: None,
                usage: None,
                finish_reason: Default::default(),
            },
            LlmOutput {
                content: "step-2".to_string(),
                tool_calls: vec![ToolCallRequest {
                    id: "call-once-2".to_string(),
                    name: "quickTool".to_string(),
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

    let tools = astrcode_runtime_registry::ToolRegistry::builder()
        .register(Box::new(QuickTool))
        .build();
    let loop_runner = AgentLoop::from_capabilities(
        Arc::new(StaticProviderFactory { provider }),
        capabilities_from_tools(tools),
    );
    let (events, mut on_event) = collect_events();

    let outcome = loop_runner
        .run_turn(
            &make_state("emit turn done once"),
            "turn-once",
            &mut on_event,
            CancelToken::new(),
        )
        .await
        .expect("turn should complete");

    assert_eq!(outcome, TurnOutcome::Completed);

    let events = events.lock().expect("events lock").clone();
    let turn_done_count = events
        .iter()
        .filter(|event| matches!(event, StorageEvent::TurnDone { .. }))
        .count();
    assert_eq!(turn_done_count, 1, "turn done must be emitted exactly once");
}
