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
    sync::{Arc, Mutex},
};

use astrcode_core::{AstrError, CancelToken, LlmMessage, Phase, StorageEvent, UserMessageOrigin};
use astrcode_runtime_llm::LlmOutput;

use super::{fixtures::*, test_support::empty_capabilities};
use crate::{AgentLoop, agent_loop::TurnOutcome, compaction_runtime::CompactionTailSnapshot};

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
        .manual_compact_event(&state, CompactionTailSnapshot::default())
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
