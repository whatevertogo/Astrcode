//! 取消相关测试。
//!
//! 覆盖：
//! - 工具执行中取消
//! - 并行安全工具取消传播
//! - LLM 流式输出中取消

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use astrcode_core::{CancelToken, StorageEvent, ToolCallRequest};
use astrcode_runtime_llm::LlmOutput;
use serde_json::json;
use tokio::time::{Duration, sleep};

use super::{
    fixtures::*,
    test_support::{capabilities_from_tools, empty_capabilities},
};
use crate::{AgentLoop, agent_loop::TurnOutcome};

#[tokio::test]
async fn interrupt_emits_error_and_turn_done() {
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([LlmOutput {
            content: "".to_string(),
            tool_calls: vec![ToolCallRequest {
                id: "call-slow".to_string(),
                name: "slowTool".to_string(),
                args: json!({}),
            }],
            reasoning: None,
            usage: None,
            finish_reason: Default::default(),
        }])),
        delay: std::time::Duration::from_millis(0),
    });

    let tools = astrcode_runtime_registry::ToolRegistry::builder()
        .register(Box::new(SlowTool))
        .build();

    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, capabilities_from_tools(tools));
    let state = make_state("run slow");

    let events: Arc<Mutex<Vec<StorageEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let cancel = CancelToken::new();
    let cancel_clone = cancel.clone();
    let events_clone = events.clone();

    let cancel_task = tokio::spawn(async move {
        sleep(Duration::from_millis(40)).await;
        cancel_clone.cancel();
    });

    let mut on_event = move |e: StorageEvent| {
        events_clone.lock().expect("lock").push(e);
        Ok(())
    };
    let outcome = loop_runner
        .run_turn(&state, "turn-2", &mut on_event, cancel)
        .await
        .expect("turn should end cleanly");
    assert_eq!(outcome, TurnOutcome::Cancelled);
    cancel_task.await.expect("cancel task should join");

    let events = events.lock().expect("lock").clone();
    let has_error = events
        .iter()
        .any(|e| matches!(e, StorageEvent::Error { message, .. } if message == "interrupted"));
    let has_done = events.iter().any(|e| {
        matches!(
            e,
            StorageEvent::TurnDone { reason, .. } if reason.as_deref() == Some("cancelled")
        )
    });

    assert!(has_error, "should have Error(interrupted)");
    assert!(has_done, "should have TurnDone");
}

#[tokio::test]
async fn cancellation_propagates_to_parallel_safe_tools() {
    let tracker = Arc::new(ConcurrencyTracker::default());
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([LlmOutput {
            content: String::new(),
            tool_calls: vec![
                ToolCallRequest {
                    id: "call-cancel-1".to_string(),
                    name: "cancelSafeTool".to_string(),
                    args: json!({ "delayMs": 250 }),
                },
                ToolCallRequest {
                    id: "call-cancel-2".to_string(),
                    name: "cancelSafeTool".to_string(),
                    args: json!({ "delayMs": 250 }),
                },
            ],
            reasoning: None,
            usage: None,
            finish_reason: Default::default(),
        }])),
        delay: std::time::Duration::from_millis(0),
    });
    let tools = astrcode_runtime_registry::ToolRegistry::builder()
        .register(Box::new(ConcurrencyTrackingTool {
            name: "cancelSafeTool",
            concurrency_safe: true,
            tracker: Arc::clone(&tracker),
        }))
        .build();
    let loop_runner = AgentLoop::from_capabilities(
        Arc::new(StaticProviderFactory { provider }),
        capabilities_from_tools(tools),
    );
    let cancel = CancelToken::new();
    let cancel_clone = cancel.clone();

    let cancel_task = tokio::spawn(async move {
        sleep(Duration::from_millis(40)).await;
        cancel_clone.cancel();
    });

    let outcome = loop_runner
        .run_turn(
            &make_state("cancel safe tools"),
            "turn-cancel-safe",
            &mut |_event| Ok(()),
            cancel,
        )
        .await
        .expect("turn should end cleanly");
    cancel_task.await.expect("cancel task should join");

    assert_eq!(outcome, TurnOutcome::Cancelled);
    assert_eq!(tracker.started.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(
        tracker.cancelled.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "all running safe tools should observe cancellation"
    );
}

#[tokio::test]
async fn deltas_emit_before_stream_completion() {
    let provider = Arc::new(StreamingProvider {
        response: LlmOutput {
            content: "streamed".to_string(),
            tool_calls: vec![],
            reasoning: None,
            usage: None,
            finish_reason: Default::default(),
        },
        per_delta_delay: Duration::from_millis(20),
    });
    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, empty_capabilities());
    let state = make_state("stream please");

    let events: Arc<Mutex<Vec<StorageEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events_for_task = events.clone();

    let run_task = tokio::spawn(async move {
        let mut on_event = move |event: StorageEvent| {
            events_for_task.lock().expect("lock").push(event);
            Ok(())
        };

        loop_runner
            .run_turn(&state, "turn-3", &mut on_event, CancelToken::new())
            .await
            .expect("turn should complete");
    });

    tokio::time::timeout(Duration::from_millis(50), async {
        loop {
            if events
                .lock()
                .expect("lock")
                .iter()
                .any(|event| matches!(event, StorageEvent::AssistantDelta { .. }))
            {
                break;
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("delta should be emitted before streaming completes");

    let snapshot = events.lock().expect("lock").clone();
    assert!(
        snapshot
            .iter()
            .any(|event| matches!(event, StorageEvent::AssistantDelta { .. }))
    );
    assert!(
        !snapshot
            .iter()
            .any(|event| matches!(event, StorageEvent::TurnDone { .. })),
        "turn should still be in progress when first delta arrives"
    );

    run_task.await.expect("run task should join");
}
