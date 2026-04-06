//! Prompt 相关测试。
//!
//! 覆盖：
//! - 每步重建 system prompt
//! - Prompt contributor 缓存复用
//! - 事件 sink 失败中止 turn

use std::{
    collections::VecDeque,
    fs,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use astrcode_core::{
    AstrError, CancelToken, LlmMessage, Phase, ToolCallRequest, UserMessageOrigin,
};
use astrcode_runtime_llm::LlmOutput;
use astrcode_runtime_prompt::{PromptComposer, PromptComposerOptions};
use serde_json::json;
use tokio::time::Duration;

use super::{
    fixtures::*,
    test_support::{capabilities_from_tools, empty_capabilities},
};
use crate::AgentLoop;

#[tokio::test]
async fn rebuilds_system_prompt_for_every_step_and_keeps_agents_rules_active() {
    let guard = super::test_support::TestEnvGuard::new();
    let project = tempfile::tempdir().expect("tempdir should be created");
    let user_agents_path = guard.home_dir().join(".astrcode").join("AGENTS.md");
    fs::create_dir_all(user_agents_path.parent().expect("parent should exist"))
        .expect("user agents dir should be created");
    fs::write(&user_agents_path, "Follow user rule").expect("user agents file should be written");
    fs::write(project.path().join("AGENTS.md"), "Follow project rule")
        .expect("project agents file should be written");

    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(RecordingProvider {
        responses: Mutex::new(VecDeque::from([
            LlmOutput {
                content: String::new(),
                tool_calls: vec![ToolCallRequest {
                    id: "call-1".to_string(),
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
        requests: requests.clone(),
    });

    let tools = astrcode_core::ToolRegistry::builder()
        .register(Box::new(QuickTool))
        .build();

    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, capabilities_from_tools(tools));
    let state = astrcode_core::AgentState {
        session_id: "test".into(),
        working_dir: project.path().to_path_buf(),
        messages: vec![LlmMessage::User {
            content: "run quick tool".into(),
            origin: UserMessageOrigin::User,
        }],
        phase: Phase::Thinking,
        turn_count: 0,
    };

    loop_runner
        .run_turn(&state, "turn-5", &mut |_event| Ok(()), CancelToken::new())
        .await
        .expect("turn should complete");

    let requests = requests.lock().expect("lock should work").clone();
    assert_eq!(requests.len(), 2, "expected one request per llm step");

    for request in &requests {
        let prompt = request
            .system_prompt
            .as_deref()
            .expect("system prompt should be present for every step");
        assert!(prompt.contains("[Identity]"));
        assert!(prompt.contains("[Environment]"));
        assert!(prompt.contains(&format!(
            "User-wide instructions from {}:\nFollow user rule",
            user_agents_path.display()
        )));
        assert!(prompt.contains(&format!(
            "Project-specific instructions from {}:\nFollow project rule",
            project.path().join("AGENTS.md").display()
        )));
        assert!(prompt.contains(&format!(
            "Working directory: {}",
            project.path().to_string_lossy()
        )));
        assert!(request.tools.iter().any(|tool| tool.name == "quickTool"));
    }

    // RequestAssembler 现在会在真实会话消息前编码 structured workset，所以消息数比
    // 旧基线多 1。第二轮仍沿用历史基线：只保留继续执行所需的 assistant/tool 记录，
    // 不再重复首轮用户消息。
    assert_eq!(requests[0].messages.len(), 4);
    assert_eq!(requests[1].messages.len(), 4);
    assert!(matches!(
        &requests[0].messages[0],
        LlmMessage::User { content, .. } if content.starts_with("Before changing code, inspect the relevant files and gather context first.")
    ));
    assert!(matches!(
        &requests[0].messages[1],
        LlmMessage::Assistant { content, tool_calls, .. } if content.starts_with("I will inspect the relevant files and gather context before making changes.") && tool_calls.is_empty()
    ));
    assert!(matches!(
        &requests[0].messages[2],
        LlmMessage::User { content, .. } if content.contains("[Structured workset: working-dir]")
    ));
    assert!(matches!(
        &requests[0].messages[3],
        LlmMessage::User { content, .. } if content == "run quick tool"
    ));
    assert!(
        requests[1].messages.iter().any(|message| {
            matches!(
                message,
                LlmMessage::Assistant { tool_calls, .. }
                    if tool_calls.len() == 1 && tool_calls[0].name == "quickTool"
            )
        }),
        "assistant tool call should remain visible on later steps"
    );
    assert!(
        requests[1].messages.iter().any(|message| {
            matches!(
                message,
                LlmMessage::User { content, .. }
                    if content.contains("[Structured workset: working-dir]")
            )
        }),
        "structured workset should remain present on later steps"
    );
    assert!(
        requests[1].messages.iter().any(|message| {
            matches!(
                message,
                LlmMessage::Tool { tool_call_id, content }
                    if tool_call_id == "call-1" && content == "ok"
            )
        }),
        "tool result should remain visible on later steps"
    );
}

#[tokio::test]
async fn reuses_prompt_contributor_cache_across_llm_steps() {
    let _guard = super::test_support::TestEnvGuard::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let composer = PromptComposer::with_options(PromptComposerOptions {
        cache_ttl: Duration::from_secs(60),
        ..PromptComposerOptions::default()
    })
    .with_contributor(Arc::new(CountingPromptContributor {
        calls: calls.clone(),
    }));
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([
            LlmOutput {
                content: String::new(),
                tool_calls: vec![ToolCallRequest {
                    id: "call-cache".to_string(),
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
    let tools = astrcode_core::ToolRegistry::builder()
        .register(Box::new(QuickTool))
        .build();
    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, capabilities_from_tools(tools))
        .with_prompt_composer(composer);
    let state = make_state("cache prompt");

    loop_runner
        .run_turn(
            &state,
            "turn-cache",
            &mut |_event| Ok(()),
            CancelToken::new(),
        )
        .await
        .expect("turn should complete");

    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn event_sink_failures_abort_the_turn() {
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([LlmOutput {
            content: "done".to_string(),
            tool_calls: vec![],
            reasoning: None,
            usage: None,
            finish_reason: Default::default(),
        }])),
        delay: std::time::Duration::from_millis(0),
    });
    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, empty_capabilities());
    let state = make_state("fail event sink");

    let result = loop_runner
        .run_turn(
            &state,
            "turn-6",
            &mut |_event| Err(AstrError::Internal("event sink failed".to_string())),
            CancelToken::new(),
        )
        .await;

    assert!(result.is_err());
    assert!(
        result
            .expect_err("result should be error")
            .to_string()
            .contains("event sink failed")
    );
}
