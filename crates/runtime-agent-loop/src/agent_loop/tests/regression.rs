//! Phase 0：行为基线锁定测试。
//!
//! 在正式重构前锁定现有 `turn_runner` 的语义，确保后续重构不引入回退。
//! 覆盖场景：
//! - 普通单轮无工具
//! - tool call 后继续一轮
//! - cancel / interrupted
//! - auto compact
//! - reactive compact
//! - policy deny
//! - policy ask

use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use astrcode_core::{
    ApprovalDefault, ApprovalResolution, CancelToken, LlmMessage, Phase, StorageEvent,
    ToolCallRequest, UserMessageOrigin,
};
use astrcode_runtime_llm::LlmOutput;
use serde_json::json;

use super::{
    fixtures::*,
    test_support::{capabilities_from_tools, empty_capabilities},
};
use crate::{AgentLoop, agent_loop::TurnOutcome};

// ---------------------------------------------------------------------------
// 基础场景矩阵
// ---------------------------------------------------------------------------

enum Scenario {
    PlainText,
    ToolRoundTrip,
    Cancelled,
}

#[tokio::test]
async fn phase0_behavior_regression_matrix_keeps_core_turn_outcomes_stable() {
    let cases = vec![
        ("plain-text", Scenario::PlainText, TurnOutcome::Completed),
        (
            "tool-round-trip",
            Scenario::ToolRoundTrip,
            TurnOutcome::Completed,
        ),
        ("cancelled", Scenario::Cancelled, TurnOutcome::Cancelled),
    ];

    for (name, scenario, expected_outcome) in cases {
        let state = make_state("phase0 baseline");
        let (provider, capabilities, cancel) = match scenario {
            Scenario::PlainText => (
                Arc::new(ScriptedProvider {
                    responses: Mutex::new(VecDeque::from([LlmOutput {
                        content: "done".to_string(),
                        tool_calls: vec![],
                        reasoning: None,
                        usage: None,
                        finish_reason: Default::default(),
                    }])),
                    delay: std::time::Duration::from_millis(0),
                }) as Arc<dyn astrcode_runtime_llm::LlmProvider>,
                empty_capabilities(),
                CancelToken::new(),
            ),
            Scenario::ToolRoundTrip => (
                Arc::new(ScriptedProvider {
                    responses: Mutex::new(VecDeque::from([
                        LlmOutput {
                            content: String::new(),
                            tool_calls: vec![ToolCallRequest {
                                id: "phase0-call".to_string(),
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
                }) as Arc<dyn astrcode_runtime_llm::LlmProvider>,
                capabilities_from_tools(
                    astrcode_runtime_registry::ToolRegistry::builder()
                        .register(Box::new(QuickTool))
                        .build(),
                ),
                CancelToken::new(),
            ),
            Scenario::Cancelled => {
                let cancel = CancelToken::new();
                cancel.cancel();
                (
                    Arc::new(ScriptedProvider {
                        responses: Mutex::new(VecDeque::from([LlmOutput {
                            content: "never happens".to_string(),
                            tool_calls: vec![],
                            reasoning: None,
                            usage: None,
                            finish_reason: Default::default(),
                        }])),
                        delay: std::time::Duration::from_millis(0),
                    }) as Arc<dyn astrcode_runtime_llm::LlmProvider>,
                    empty_capabilities(),
                    cancel,
                )
            },
        };

        let loop_runner = AgentLoop::from_capabilities(
            Arc::new(StaticProviderFactory { provider }),
            capabilities,
        );

        let (events, mut on_event) = collect_events();

        let outcome = loop_runner
            .run_turn(&state, name, &mut on_event, cancel)
            .await
            .expect("phase0 scenario should return an outcome");

        assert_eq!(outcome, expected_outcome, "scenario {name} drifted");
        assert!(
            events
                .lock()
                .expect("events lock")
                .iter()
                .any(|event| matches!(event, StorageEvent::TurnDone { .. })),
            "scenario {name} should still emit TurnDone"
        );
    }
}

// ---------------------------------------------------------------------------
// Compaction 和 Policy 边缘场景
// ---------------------------------------------------------------------------

#[tokio::test]
async fn phase0_behavior_regression_covers_compaction_and_policy_edges() {
    let _guard = super::test_support::TestEnvGuard::new();

    // Auto compact
    {
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
        let loop_runner = AgentLoop::from_capabilities(
            Arc::new(StaticProviderFactory { provider }),
            empty_capabilities(),
        )
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
                "phase0-auto-compact",
                &mut on_event,
                CancelToken::new(),
            )
            .await
            .expect("auto compact baseline should complete");

        assert!(matches!(outcome, TurnOutcome::Completed));
        assert!(events.lock().expect("events lock").iter().any(|event| {
            matches!(
                event,
                StorageEvent::CompactApplied { summary, .. } if summary == "condensed history"
            )
        }));
    }

    // Reactive compact
    {
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
        let loop_runner = AgentLoop::from_capabilities(
            Arc::new(StaticProviderFactory { provider }),
            empty_capabilities(),
        )
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
                "phase0-reactive-compact",
                &mut on_event,
                CancelToken::new(),
            )
            .await
            .expect("reactive compact baseline should complete");

        assert!(matches!(outcome, TurnOutcome::Completed));
        let events = events.lock().expect("events lock");
        assert!(
            events
                .iter()
                .any(|event| matches!(event, StorageEvent::CompactApplied { .. }))
        );
        assert!(events.iter().any(|event| {
            matches!(
                event,
                StorageEvent::AssistantFinal { content, .. } if content == "recovered answer"
            )
        }));
    }

    // Policy deny
    {
        let executions = Arc::new(AtomicUsize::new(0));
        let provider = Arc::new(ScriptedProvider {
            responses: Mutex::new(VecDeque::from([
                LlmOutput {
                    content: String::new(),
                    tool_calls: vec![ToolCallRequest {
                        id: "phase0-call-policy-deny".to_string(),
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
        let tools = astrcode_runtime_registry::ToolRegistry::builder()
            .register(Box::new(CountingTool {
                executions: Arc::clone(&executions),
            }))
            .build();
        let loop_runner = AgentLoop::from_capabilities(
            Arc::new(StaticProviderFactory { provider }),
            capabilities_from_tools(tools),
        )
        .with_policy_engine(Arc::new(DenyCapabilityPolicy {
            capability_name: "policyTool".to_string(),
            reason: "policy blocked tool".to_string(),
        }));
        let (events, mut on_event) = collect_events();

        let outcome = loop_runner
            .run_turn(
                &make_state("phase0 deny tool"),
                "phase0-policy-deny",
                &mut on_event,
                CancelToken::new(),
            )
            .await
            .expect("policy deny baseline should complete");

        assert!(matches!(outcome, TurnOutcome::Completed));
        assert_eq!(executions.load(Ordering::SeqCst), 0);
        assert!(events.lock().expect("events lock").iter().any(|event| {
            matches!(
                event,
                StorageEvent::ToolResult {
                    tool_name,
                    success,
                    error,
                    ..
                } if tool_name == "policyTool"
                    && !success
                    && error.as_deref() == Some("policy blocked tool")
            )
        }));
    }

    // Policy ask
    {
        let executions = Arc::new(AtomicUsize::new(0));
        let approval_requests = Arc::new(Mutex::new(Vec::new()));
        let provider = Arc::new(ScriptedProvider {
            responses: Mutex::new(VecDeque::from([
                LlmOutput {
                    content: String::new(),
                    tool_calls: vec![ToolCallRequest {
                        id: "phase0-call-policy-ask".to_string(),
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
        let broker = Arc::new(RecordingApprovalBroker {
            requests: Arc::clone(&approval_requests),
            resolutions: Mutex::new(VecDeque::from([ApprovalResolution::approved()])),
        });
        let tools = astrcode_runtime_registry::ToolRegistry::builder()
            .register(Box::new(CountingTool {
                executions: Arc::clone(&executions),
            }))
            .build();
        let loop_runner = AgentLoop::from_capabilities(
            Arc::new(StaticProviderFactory { provider }),
            capabilities_from_tools(tools),
        )
        .with_policy_engine(Arc::new(AskCapabilityPolicy {
            capability_name: "policyTool".to_string(),
            prompt: "Allow policyTool?".to_string(),
            default: ApprovalDefault::Deny,
        }))
        .with_approval_broker(broker);

        let outcome = loop_runner
            .run_turn(
                &make_state("phase0 ask tool"),
                "phase0-policy-ask",
                &mut |_event| Ok(()),
                CancelToken::new(),
            )
            .await
            .expect("policy ask baseline should complete");

        assert!(matches!(outcome, TurnOutcome::Completed));
        assert_eq!(executions.load(Ordering::SeqCst), 1);
        assert_eq!(
            approval_requests
                .lock()
                .expect("approval requests lock")
                .len(),
            1
        );
    }
}

/// 构造一个 413 prompt too long 错误。
fn make_prompt_too_long_error() -> astrcode_core::AstrError {
    astrcode_core::AstrError::LlmRequestFailed {
        status: 413,
        body: "prompt too long for this model".to_string(),
    }
}
