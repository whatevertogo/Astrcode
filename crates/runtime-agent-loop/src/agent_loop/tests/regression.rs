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
    AgentLifecycleStatus, ApprovalDefault, ApprovalResolution, CancelToken, ChildAgentRef,
    ChildSessionLineageKind, ChildSessionNotification, ChildSessionNotificationKind, LlmMessage,
    Phase, StorageEventPayload, ToolCallRequest, UserMessageOrigin,
};
use astrcode_runtime_llm::LlmOutput;
use serde_json::json;

use super::{
    fixtures::*,
    test_support::{boxed_tool, capabilities_from_tools, empty_capabilities},
};
use crate::{AgentLoop, agent_loop::TurnOutcome, child_delivery_prompt_declaration};

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
                capabilities_from_tools(vec![boxed_tool(QuickTool)]),
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
                .any(|event| matches!(&event.payload, StorageEventPayload::TurnDone { .. })),
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
            last_assistant_at: None,
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
                &event.payload,
                StorageEventPayload::CompactApplied { summary, .. } if summary == "condensed history"
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
            last_assistant_at: None,
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
                .any(|event| matches!(&event.payload, StorageEventPayload::CompactApplied { .. }))
        );
        assert!(events.iter().any(|event| {
            matches!(
                &event.payload,
                StorageEventPayload::AssistantFinal { content, .. } if content == "recovered answer"
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
        let tools = vec![boxed_tool(CountingTool {
            executions: executions.clone(),
        })];
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
                &event.payload,
                StorageEventPayload::ToolResult {
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
        let tools = vec![boxed_tool(CountingTool {
            executions: executions.clone(),
        })];
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

#[test]
fn child_delivery_prompt_regression_keeps_delivery_identity_and_duplicate_guidance() {
    let notification = ChildSessionNotification {
        notification_id: "delivery-regression-1".to_string(),
        child_ref: ChildAgentRef {
            agent_id: "agent-child".to_string(),
            session_id: "session-parent".to_string(),
            sub_run_id: "subrun-child".to_string(),
            parent_agent_id: Some("agent-parent".to_string()),
            parent_sub_run_id: Some("subrun-parent".to_string()),
            lineage_kind: ChildSessionLineageKind::Spawn,
            status: AgentLifecycleStatus::Idle,
            open_session_id: "session-child".to_string(),
        },
        kind: ChildSessionNotificationKind::Delivered,
        summary: "child summary".to_string(),
        status: AgentLifecycleStatus::Idle,
        source_tool_call_id: None,
        final_reply_excerpt: Some("child final excerpt".to_string()),
    };

    let prompt = child_delivery_prompt_declaration(&notification);

    assert!(prompt.contains("deliveryId: delivery-regression-1"));
    assert!(prompt.contains("相同的 deliveryId"));
    assert!(prompt.contains("不能把它当作新任务重复处理"));
}

/// 构造一个 413 prompt too long 错误。
fn make_prompt_too_long_error() -> astrcode_core::AstrError {
    astrcode_core::AstrError::LlmRequestFailed {
        status: 413,
        body: "prompt too long for this model".to_string(),
    }
}

// ---------------------------------------------------------------------------
// T036: close-or-keep 决策回归测试
// ---------------------------------------------------------------------------

/// 验证 agent loop 在收到子 Agent 交付结果后仍然能正常完成 turn。
/// 模型在收到 close 工具结果后选择关闭子 Agent，
/// turn 应正常完成（Completed），不应因子交付的存在而阻塞。
#[tokio::test]
async fn parent_turn_completes_after_deciding_to_close_child() {
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([
            // 第一轮：模型调用 close 关闭子 Agent
            LlmOutput {
                content: String::new(),
                tool_calls: vec![ToolCallRequest {
                    id: "call-close-1".to_string(),
                    name: "close".to_string(),
                    args: json!({
                        "agentId": "agent-child-1"
                    }),
                }],
                reasoning: None,
                usage: None,
                finish_reason: Default::default(),
            },
            // 第二轮：模型输出最终回复
            LlmOutput {
                content: "子 Agent 已关闭，任务完成".to_string(),
                tool_calls: vec![],
                reasoning: None,
                usage: None,
                finish_reason: Default::default(),
            },
        ])),
        delay: std::time::Duration::from_millis(0),
    });

    let tools = vec![];
    let loop_runner = AgentLoop::from_capabilities(
        Arc::new(StaticProviderFactory { provider }),
        capabilities_from_tools(tools),
    );
    let state = make_state("close-or-keep close decision");

    let (events, mut on_event) = collect_events();

    let outcome = loop_runner
        .run_turn(
            &state,
            "turn-close-decision",
            &mut on_event,
            CancelToken::new(),
        )
        .await
        .expect("turn should complete");
    assert_eq!(outcome, TurnOutcome::Completed);

    // 验证事件序列中包含 ToolCall、ToolResult 和 TurnDone
    let events = events.lock().expect("events lock").clone();
    assert!(events.iter().any(
        |e| matches!(&e.payload, StorageEventPayload::ToolCall { tool_name, .. } if tool_name == "close")
    ));
    assert!(events.iter().any(
        |e| matches!(&e.payload, StorageEventPayload::ToolResult { tool_name, .. } if tool_name == "close")
    ));
    assert!(events.iter().any(|e| matches!(
        &e.payload,
        StorageEventPayload::TurnDone { reason, .. } if reason.as_deref() == Some("completed")
    )));
}

/// 验证 agent loop 在选择保留子 Agent 时能正常继续执行后续工具调用。
/// 模型先调用 send 追加消息，然后输出最终回复，
/// turn 应正常完成，子 Agent 保留继续运行。
#[tokio::test]
async fn parent_turn_completes_after_deciding_to_keep_child() {
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([
            // 第一轮：模型调用 send 向子 Agent 发送消息
            LlmOutput {
                content: String::new(),
                tool_calls: vec![ToolCallRequest {
                    id: "call-send-1".to_string(),
                    name: "send".to_string(),
                    args: json!({
                        "agentId": "agent-child-1",
                        "message": "继续执行"
                    }),
                }],
                reasoning: None,
                usage: None,
                finish_reason: Default::default(),
            },
            // 第二轮：模型输出最终回复
            LlmOutput {
                content: "已向子 Agent 发送追加指令，继续等待结果".to_string(),
                tool_calls: vec![],
                reasoning: None,
                usage: None,
                finish_reason: Default::default(),
            },
        ])),
        delay: std::time::Duration::from_millis(0),
    });

    let tools = vec![];
    let loop_runner = AgentLoop::from_capabilities(
        Arc::new(StaticProviderFactory { provider }),
        capabilities_from_tools(tools),
    );
    let state = make_state("close-or-keep keep decision");

    let (events, mut on_event) = collect_events();

    let outcome = loop_runner
        .run_turn(
            &state,
            "turn-keep-decision",
            &mut on_event,
            CancelToken::new(),
        )
        .await
        .expect("turn should complete");
    assert_eq!(outcome, TurnOutcome::Completed);

    // 验证事件序列中包含 send 工具调用
    let events = events.lock().expect("events lock").clone();
    assert!(events.iter().any(
        |e| matches!(&e.payload, StorageEventPayload::ToolCall { tool_name, .. } if tool_name == "send")
    ));
    assert!(events.iter().any(|e| matches!(
        &e.payload,
        StorageEventPayload::TurnDone { reason, .. } if reason.as_deref() == Some("completed")
    )));
}

#[tokio::test]
async fn replayed_child_state_is_consumed_as_resume_history_instead_of_empty_spawn_state() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(RecordingProvider {
        responses: Mutex::new(VecDeque::from([LlmOutput {
            content: "继续完成".to_string(),
            tool_calls: vec![],
            reasoning: None,
            usage: None,
            finish_reason: Default::default(),
        }])),
        requests: Arc::clone(&requests),
    });
    let loop_runner = AgentLoop::from_capabilities(
        Arc::new(StaticProviderFactory { provider }),
        empty_capabilities(),
    );
    let state = replayed_child_state_fixture("resume");

    let outcome = loop_runner
        .run_turn(
            &state,
            "turn-resume-replayed-state",
            &mut |_event| Ok(()),
            CancelToken::new(),
        )
        .await
        .expect("resume turn should complete");

    assert_eq!(outcome, TurnOutcome::Completed);
    let requests = requests.lock().expect("requests lock");
    let request = requests
        .first()
        .expect("provider should receive one request");
    assert!(
        request.messages.len() >= 3,
        "replayed resume history should contribute multiple prior messages"
    );
    assert!(request.messages.iter().any(|message| {
        matches!(
            message,
            astrcode_core::LlmMessage::User { content, .. } if content == "任务 resume"
        )
    }));
    assert!(request.messages.iter().any(|message| {
        matches!(
            message,
            astrcode_core::LlmMessage::Assistant { content, .. } if content == "阶段性总结 resume"
        )
    }));
    assert!(request.messages.iter().any(|message| {
        matches!(
            message,
            astrcode_core::LlmMessage::User { content, .. } if content == "继续处理 resume"
        )
    }));
}
