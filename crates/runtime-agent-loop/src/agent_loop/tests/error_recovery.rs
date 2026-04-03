//! 错误恢复测试（P4）。
//!
//! 覆盖：
//! - max_tokens 截断时自动注入 nudge 继续生成

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;

use astrcode_core::{CancelToken, StorageEvent};
use astrcode_runtime_llm::{FinishReason, LlmOutput};

use super::fixtures::*;
use super::test_support::empty_capabilities;
use crate::agent_loop::TurnOutcome;
use crate::AgentLoop;

/// P4.2: max_tokens 截断时自动注入 nudge 继续生成。
///
/// 场景：
/// 1. 首次 LLM 调用返回 finish_reason = max_tokens
/// 2. 自动注入 nudge 消息，再次调用 LLM
/// 3. 第二次调用正常返回 finish_reason = stop
#[tokio::test]
async fn p4_2_max_tokens_triggers_auto_continue() {
    let _guard = super::test_support::TestEnvGuard::new();

    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([
            // 第一次调用被 max_tokens 截断
            LlmOutput {
                content: "this is a partial response that got cut off".to_string(),
                tool_calls: vec![],
                reasoning: None,
                usage: None,
                finish_reason: FinishReason::MaxTokens,
            },
            // nudge 后继续生成完成
            LlmOutput {
                content: " and this is the rest of the response.".to_string(),
                tool_calls: vec![],
                reasoning: None,
                usage: None,
                finish_reason: FinishReason::Stop,
            },
        ])),
        delay: std::time::Duration::from_millis(0),
    });

    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, empty_capabilities());

    let state = make_state("write something long");

    let (events, mut on_event) = collect_events();

    let outcome = loop_runner
        .run_turn(
            &state,
            "turn-max-tokens-continue",
            &mut on_event,
            CancelToken::new(),
        )
        .await
        .expect("turn should complete after auto-continue");

    assert!(
        matches!(outcome, TurnOutcome::Completed),
        "turn should complete after max_tokens auto-continue"
    );

    let events = events.lock().expect("events lock");
    // 应该有两次 AssistantFinal（第一次截断 + 第二次继续）
    let assistant_finals: Vec<_> = events
        .iter()
        .filter_map(|event| {
            if let StorageEvent::AssistantFinal { content, .. } = event {
                Some(content.clone())
            } else {
                None
            }
        })
        .collect();

    assert_eq!(
        assistant_finals.len(),
        2,
        "should have two AssistantFinal events (truncated + continued)"
    );
    assert!(
        assistant_finals[0].contains("partial response"),
        "first response should be the truncated content"
    );
    assert!(
        assistant_finals[1].contains("rest of the response"),
        "second response should be the continued content"
    );
}
