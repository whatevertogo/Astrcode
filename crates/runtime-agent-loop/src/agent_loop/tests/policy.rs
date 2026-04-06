//! Policy 相关测试。
//!
//! 覆盖：
//! - Policy 重写模型请求
//! - 拒绝工具调用
//! - 需要审批的工具调用
//! - 审批拒绝

use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use astrcode_core::{
    ApprovalDefault, ApprovalResolution, CancelToken, StorageEvent, ToolCallRequest,
};
use astrcode_runtime_llm::LlmOutput;
use serde_json::json;

use super::{
    fixtures::*,
    test_support::{capabilities_from_tools, empty_capabilities},
};
use crate::AgentLoop;

#[tokio::test]
async fn policy_can_rewrite_model_request_before_provider_execution() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(RecordingProvider {
        responses: Mutex::new(VecDeque::from([LlmOutput {
            content: "done".to_string(),
            tool_calls: vec![],
            reasoning: None,
            usage: None,
            finish_reason: Default::default(),
        }])),
        requests: Arc::clone(&requests),
    });
    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, empty_capabilities())
        .with_policy_engine(Arc::new(RewriteSystemPromptPolicy {
            suffix: "[Policy Guardrail]".to_string(),
        }));

    loop_runner
        .run_turn(
            &make_state("rewrite prompt"),
            "turn-policy-rewrite",
            &mut |_event| Ok(()),
            CancelToken::new(),
        )
        .await
        .expect("turn should complete");

    let requests = requests.lock().expect("recorded requests lock");
    let prompt = requests[0]
        .system_prompt
        .as_deref()
        .expect("system prompt should exist");
    assert!(prompt.contains("[Policy Guardrail]"));
}

#[tokio::test]
async fn denied_tool_calls_emit_failure_without_executing_tool() {
    let executions = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([
            LlmOutput {
                content: String::new(),
                tool_calls: vec![ToolCallRequest {
                    id: "call-policy-deny".to_string(),
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
    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, capabilities_from_tools(tools))
        .with_policy_engine(Arc::new(DenyCapabilityPolicy {
            capability_name: "policyTool".to_string(),
            reason: "policy blocked tool".to_string(),
        }));
    let (events, mut on_event) = collect_events();

    loop_runner
        .run_turn(
            &make_state("deny tool"),
            "turn-policy-deny",
            &mut on_event,
            CancelToken::new(),
        )
        .await
        .expect("turn should complete");

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

#[tokio::test]
async fn ask_policy_uses_approval_broker_before_tool_execution() {
    let executions = Arc::new(AtomicUsize::new(0));
    let approval_requests = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([
            LlmOutput {
                content: String::new(),
                tool_calls: vec![ToolCallRequest {
                    id: "call-policy-ask".to_string(),
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
    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, capabilities_from_tools(tools))
        .with_policy_engine(Arc::new(AskCapabilityPolicy {
            capability_name: "policyTool".to_string(),
            prompt: "Allow policyTool?".to_string(),
            default: ApprovalDefault::Deny,
        }))
        .with_approval_broker(broker);

    loop_runner
        .run_turn(
            &make_state("ask tool"),
            "turn-policy-ask",
            &mut |_event| Ok(()),
            CancelToken::new(),
        )
        .await
        .expect("turn should complete");

    assert_eq!(executions.load(Ordering::SeqCst), 1);
    let requests = approval_requests.lock().expect("approval requests lock");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].capability_name(), "policyTool");
    assert_eq!(requests[0].prompt, "Allow policyTool?");
}

#[tokio::test]
async fn denied_approval_returns_failed_tool_result_without_execution() {
    let executions = Arc::new(AtomicUsize::new(0));
    let approval_requests = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([
            LlmOutput {
                content: String::new(),
                tool_calls: vec![ToolCallRequest {
                    id: "call-policy-ask-denied".to_string(),
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
        resolutions: Mutex::new(VecDeque::from([ApprovalResolution::denied(
            "approval rejected in test",
        )])),
    });
    let tools = astrcode_runtime_registry::ToolRegistry::builder()
        .register(Box::new(CountingTool {
            executions: Arc::clone(&executions),
        }))
        .build();
    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, capabilities_from_tools(tools))
        .with_policy_engine(Arc::new(AskCapabilityPolicy {
            capability_name: "policyTool".to_string(),
            prompt: "Allow policyTool?".to_string(),
            default: ApprovalDefault::Allow,
        }))
        .with_approval_broker(broker);
    let (events, mut on_event) = collect_events();

    loop_runner
        .run_turn(
            &make_state("deny approval"),
            "turn-policy-approval-deny",
            &mut on_event,
            CancelToken::new(),
        )
        .await
        .expect("turn should complete");

    assert_eq!(executions.load(Ordering::SeqCst), 0);
    assert_eq!(
        approval_requests
            .lock()
            .expect("approval requests lock")
            .len(),
        1
    );
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
                && error.as_deref() == Some("approval rejected in test")
        )
    }));
}
