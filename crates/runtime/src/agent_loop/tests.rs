use std::collections::VecDeque;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use astrcode_core::{AstrError, CancelToken, Phase, Result, Tool, ToolContext};
use async_trait::async_trait;
use serde_json::json;
use tokio::time::{sleep, Duration};

use crate::llm::{EventSink, LlmEvent, LlmOutput, LlmProvider, LlmRequest};
use crate::prompt::{
    BlockKind, BlockSpec, PromptComposer, PromptComposerOptions, PromptContext, PromptContribution,
    PromptContributor,
};
use crate::provider_factory::ProviderFactory;
use crate::test_support::TestEnvGuard;
use astrcode_core::AgentState;
use astrcode_core::StorageEvent;
use astrcode_core::ToolRegistry;
use astrcode_core::{LlmMessage, ToolCallRequest, ToolDefinition, ToolExecutionResult};

use super::AgentLoop;

fn make_state(user_text: &str) -> AgentState {
    AgentState {
        session_id: "test".into(),
        working_dir: std::env::temp_dir(),
        messages: vec![LlmMessage::User {
            content: user_text.into(),
        }],
        phase: Phase::Thinking,
        turn_count: 0,
    }
}

struct ScriptedProvider {
    responses: Mutex<VecDeque<LlmOutput>>,
    delay: Duration,
}

#[async_trait]
impl LlmProvider for ScriptedProvider {
    async fn generate(&self, request: LlmRequest, sink: Option<EventSink>) -> Result<LlmOutput> {
        if self.delay > Duration::from_millis(0) {
            tokio::select! {
                _ = crate::cancel::cancelled(request.cancel.clone()) => return Err(AstrError::LlmInterrupted),
                _ = sleep(self.delay) => {}
            }
        }
        let response = self
            .responses
            .lock()
            .expect("lock should work")
            .pop_front()
            .ok_or_else(|| AstrError::Internal("no scripted response".to_string()))?;

        if let Some(sink) = sink {
            for delta in response.content.chars() {
                sink(LlmEvent::TextDelta(delta.to_string()));
            }
        }

        Ok(response)
    }
}

struct SlowTool;

struct QuickTool;

struct StreamingProvider {
    response: LlmOutput,
    per_delta_delay: Duration,
}

struct RecordingProvider {
    responses: Mutex<VecDeque<LlmOutput>>,
    requests: Arc<Mutex<Vec<LlmRequest>>>,
}

struct StaticProviderFactory {
    provider: Arc<dyn LlmProvider>,
}

struct CountingPromptContributor {
    calls: Arc<AtomicUsize>,
}

impl ProviderFactory for StaticProviderFactory {
    fn build(&self) -> Result<Arc<dyn LlmProvider>> {
        Ok(self.provider.clone())
    }
}

#[async_trait]
impl PromptContributor for CountingPromptContributor {
    fn contributor_id(&self) -> &'static str {
        "counting-prompt"
    }

    async fn contribute(&self, _ctx: &PromptContext) -> PromptContribution {
        self.calls.fetch_add(1, Ordering::SeqCst);
        PromptContribution {
            blocks: vec![BlockSpec::system_text(
                "cached-block",
                BlockKind::Identity,
                "Cached",
                "cached",
            )],
            ..PromptContribution::default()
        }
    }
}

#[async_trait]
impl LlmProvider for StreamingProvider {
    async fn generate(&self, request: LlmRequest, sink: Option<EventSink>) -> Result<LlmOutput> {
        let Some(sink) = sink else {
            return Ok(self.response.clone());
        };

        for delta in self.response.content.chars() {
            sink(LlmEvent::TextDelta(delta.to_string()));

            tokio::select! {
                _ = crate::cancel::cancelled(request.cancel.clone()) => return Err(AstrError::LlmInterrupted),
                _ = sleep(self.per_delta_delay) => {}
            }
        }

        Ok(self.response.clone())
    }
}

#[async_trait]
impl LlmProvider for RecordingProvider {
    async fn generate(&self, request: LlmRequest, sink: Option<EventSink>) -> Result<LlmOutput> {
        self.requests
            .lock()
            .expect("lock should work")
            .push(request.clone());

        let response = self
            .responses
            .lock()
            .expect("lock should work")
            .pop_front()
            .ok_or_else(|| AstrError::Internal("no scripted response".to_string()))?;

        if request.cancel.is_cancelled() {
            return Err(AstrError::LlmInterrupted);
        }

        if let Some(sink) = sink {
            for delta in response.content.chars() {
                sink(LlmEvent::TextDelta(delta.to_string()));
            }
        }

        Ok(response)
    }
}

#[async_trait]
impl Tool for SlowTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "slowTool".to_string(),
            description: "slow test tool".to_string(),
            parameters: json!({"type":"object"}),
        }
    }

    async fn execute(
        &self,
        tool_call_id: String,
        _args: serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<ToolExecutionResult> {
        tokio::select! {
            _ = crate::cancel::cancelled(ctx.cancel.clone()) => Err(AstrError::Cancelled),
            _ = sleep(Duration::from_millis(250)) => Ok(ToolExecutionResult {
                tool_call_id,
                tool_name: "slowTool".to_string(),
                ok: true,
                output: "ok".to_string(),
                error: None,
                metadata: None,
                duration_ms: 250,
                truncated: false,
            })
        }
    }
}

#[async_trait]
impl Tool for QuickTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "quickTool".to_string(),
            description: "quick test tool".to_string(),
            parameters: json!({"type":"object"}),
        }
    }

    async fn execute(
        &self,
        tool_call_id: String,
        _args: serde_json::Value,
        _ctx: &ToolContext,
    ) -> Result<ToolExecutionResult> {
        Ok(ToolExecutionResult {
            tool_call_id,
            tool_name: "quickTool".to_string(),
            ok: true,
            output: "ok".to_string(),
            error: None,
            metadata: None,
            duration_ms: 1,
            truncated: false,
        })
    }
}

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
            },
            LlmOutput {
                content: "done".to_string(),
                tool_calls: vec![],
                reasoning: None,
            },
        ])),
        delay: Duration::from_millis(0),
    });

    let tools = ToolRegistry::builder()
        .register(Box::new(QuickTool))
        .build();
    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::new(factory, tools).with_max_steps(8);
    let state = make_state("list files");

    let events: Arc<Mutex<Vec<StorageEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events_clone = events.clone();
    let mut on_event = move |e: StorageEvent| {
        events_clone.lock().expect("lock").push(e);
        Ok(())
    };

    loop_runner
        .run_turn(&state, "turn-1", &mut on_event, CancelToken::new())
        .await
        .expect("turn should complete");

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
}

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
        }])),
        delay: Duration::from_millis(0),
    });

    let tools = ToolRegistry::builder().register(Box::new(SlowTool)).build();

    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::new(factory, tools).with_max_steps(8);
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
    loop_runner
        .run_turn(&state, "turn-2", &mut on_event, cancel)
        .await
        .expect("turn should end cleanly");
    cancel_task.await.expect("cancel task should join");

    let events = events.lock().expect("lock").clone();
    let has_error = events
        .iter()
        .any(|e| matches!(e, StorageEvent::Error { message, .. } if message == "interrupted"));
    let has_done = events
        .iter()
        .any(|e| matches!(e, StorageEvent::TurnDone { .. }));

    assert!(has_error, "should have Error(interrupted)");
    assert!(has_done, "should have TurnDone");
}

#[tokio::test]
async fn deltas_emit_before_stream_completion() {
    let provider = Arc::new(StreamingProvider {
        response: LlmOutput {
            content: "streamed".to_string(),
            tool_calls: vec![],
            reasoning: None,
        },
        per_delta_delay: Duration::from_millis(20),
    });
    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::new(factory, ToolRegistry::builder().build());
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
    assert!(snapshot
        .iter()
        .any(|event| matches!(event, StorageEvent::AssistantDelta { .. })));
    assert!(
        !snapshot
            .iter()
            .any(|event| matches!(event, StorageEvent::TurnDone { .. })),
        "turn should still be in progress when first delta arrives"
    );

    run_task.await.expect("run task should join");
}

#[tokio::test]
async fn reaching_max_steps_does_not_emit_error_event() {
    let scripted = (0..8)
        .map(|idx| LlmOutput {
            content: format!("step-{idx}"),
            tool_calls: vec![ToolCallRequest {
                id: format!("call-{idx}"),
                name: "quickTool".to_string(),
                args: json!({}),
            }],
            reasoning: None,
        })
        .collect::<Vec<_>>();

    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from(scripted)),
        delay: Duration::from_millis(0),
    });

    let tools = ToolRegistry::builder()
        .register(Box::new(QuickTool))
        .build();

    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::new(factory, tools);
    let state = make_state("loop test");

    let events: Arc<Mutex<Vec<StorageEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events_clone = events.clone();
    let mut on_event = move |e: StorageEvent| {
        events_clone.lock().expect("lock").push(e);
        Ok(())
    };

    loop_runner
        .run_turn(&state, "turn-4", &mut on_event, CancelToken::new())
        .await
        .expect("turn should complete");

    let events = events.lock().expect("lock").clone();
    let has_max_error = events.iter().any(|event| {
        matches!(
            event,
            StorageEvent::Error { message, .. }
            if message.contains("max tool iteration")
        )
    });
    let has_turn_done = events
        .iter()
        .any(|event| matches!(event, StorageEvent::TurnDone { .. }));

    assert!(has_turn_done, "should always emit TurnDone at max steps");
    assert!(
        !has_max_error,
        "max step exhaustion should not be surfaced as user-visible error"
    );
}

#[tokio::test]
async fn rebuilds_system_prompt_for_every_step_and_keeps_agents_rules_active() {
    let guard = TestEnvGuard::new();
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
            },
            LlmOutput {
                content: "done".to_string(),
                tool_calls: vec![],
                reasoning: None,
            },
        ])),
        requests: requests.clone(),
    });

    let tools = ToolRegistry::builder()
        .register(Box::new(QuickTool))
        .build();

    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::new(factory, tools).with_max_steps(8);
    let state = AgentState {
        session_id: "test".into(),
        working_dir: project.path().to_path_buf(),
        messages: vec![LlmMessage::User {
            content: "run quick tool".into(),
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

    assert_eq!(requests[0].messages.len(), 3);
    assert_eq!(requests[1].messages.len(), 3);
    assert!(matches!(
        &requests[0].messages[0],
        LlmMessage::User { content } if content == "Before changing code, inspect the relevant files and gather context first."
    ));
    assert!(matches!(
        &requests[0].messages[1],
        LlmMessage::Assistant { content, tool_calls, .. } if content == "I will inspect the relevant files and gather context before making changes." && tool_calls.is_empty()
    ));
    assert!(matches!(
        &requests[0].messages[2],
        LlmMessage::User { content } if content == "run quick tool"
    ));
    assert!(matches!(
        &requests[1].messages[1],
        LlmMessage::Assistant { tool_calls, .. } if tool_calls.len() == 1 && tool_calls[0].name == "quickTool"
    ));
    assert!(matches!(
        &requests[1].messages[2],
        LlmMessage::Tool { tool_call_id, content } if tool_call_id == "call-1" && content == "ok"
    ));
}

#[tokio::test]
async fn reuses_prompt_contributor_cache_across_llm_steps() {
    let calls = Arc::new(AtomicUsize::new(0));
    let composer = PromptComposer::with_options(PromptComposerOptions {
        cache_ttl: Duration::from_secs(60),
        ..PromptComposerOptions::default()
    })
    .add(Arc::new(CountingPromptContributor {
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
            },
            LlmOutput {
                content: "done".to_string(),
                tool_calls: vec![],
                reasoning: None,
            },
        ])),
        delay: Duration::from_millis(0),
    });
    let tools = ToolRegistry::builder()
        .register(Box::new(QuickTool))
        .build();
    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::new(factory, tools)
        .with_prompt_composer(composer)
        .with_max_steps(8);
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
        }])),
        delay: Duration::from_millis(0),
    });
    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::new(factory, ToolRegistry::builder().build());
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
    assert!(result
        .err()
        .expect("result should be error")
        .to_string()
        .contains("event sink failed"));
}
