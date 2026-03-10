use std::time::Instant;

use anyhow::Result;
use chrono::Utc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::action::LlmMessage;
use crate::events::StorageEvent;
use crate::llm::{EventSink, LlmEvent, LlmRequest};
use crate::projection::AgentState;
use crate::provider_factory::DynProviderFactory;
use crate::tools::registry::ToolRegistry;

pub struct AgentLoop {
    factory: DynProviderFactory,
    tools: ToolRegistry,
    max_steps: Option<usize>,
}

impl AgentLoop {
    pub fn new(factory: DynProviderFactory, tools: ToolRegistry) -> Self {
        Self {
            factory,
            tools,
            max_steps: None,
        }
    }

    pub fn with_max_steps(mut self, max_steps: usize) -> Self {
        self.max_steps = Some(max_steps);
        self
    }

    /// Execute one turn of the agent loop.
    ///
    /// `state` provides the conversation history (messages) reconstructed from events.
    /// Every significant result is emitted as a `StorageEvent` via `on_event`.
    /// The loop itself performs no IO besides LLM calls and tool execution.
    pub async fn run_turn(
        &self,
        state: &AgentState,
        on_event: &mut impl FnMut(StorageEvent),
        cancel: CancellationToken,
    ) -> Result<()> {
        let mut messages = state.messages.clone();
        // Use spawn_blocking to avoid blocking the Tokio executor with sync IO
        // (config file reading, env lookups, etc.)
        let factory = self.factory.clone();
        let provider = tokio::task::spawn_blocking(move || factory.build()).await??;

        let mut step_index = 0usize;
        loop {
            if let Some(max_steps) = self.max_steps {
                if step_index >= max_steps {
                    eprintln!(
                        "[agent_loop] reached max tool iteration steps ({}), finishing turn gracefully",
                        max_steps
                    );
                    on_event(StorageEvent::TurnDone {
                        timestamp: Utc::now(),
                    });
                    return Ok(());
                }
            }
            step_index += 1;

            if cancel.is_cancelled() {
                on_event(StorageEvent::Error {
                    message: "interrupted".to_string(),
                });
                on_event(StorageEvent::TurnDone {
                    timestamp: Utc::now(),
                });
                return Ok(());
            }

            let tool_definitions = self.tools.definitions();
            let (event_tx, mut event_rx) = mpsc::unbounded_channel::<LlmEvent>();
            let sink: EventSink = std::sync::Arc::new(move |event| {
                let _ = event_tx.send(event);
            });
            let request = LlmRequest::new(messages.clone(), tool_definitions, cancel.child_token());

            let output = {
                let generate_future = provider.generate(request, Some(sink));
                tokio::pin!(generate_future);

                // Track if channel is still open to avoid spinning when sender is dropped
                let mut event_rx_open = true;

                let output = loop {
                    tokio::select! {
                        result = &mut generate_future => break result,
                        maybe_event = event_rx.recv(), if event_rx_open => {
                            match maybe_event {
                                Some(LlmEvent::TextDelta(text)) => {
                                    eprintln!("[delta] {}", text);
                                    on_event(StorageEvent::AssistantDelta { token: text });
                                }
                                Some(LlmEvent::ToolCallDelta { .. }) => {}
                                None => {
                                    // Sender dropped, disable this branch to avoid spinning
                                    event_rx_open = false;
                                }
                            }
                        }
                    }
                };

                while let Ok(event) = event_rx.try_recv() {
                    if let LlmEvent::TextDelta(text) = event {
                        on_event(StorageEvent::AssistantDelta { token: text });
                    }
                }

                output
            };

            let output = match output {
                Ok(response) => response,
                Err(error) => {
                    if cancel.is_cancelled() {
                        on_event(StorageEvent::Error {
                            message: "interrupted".to_string(),
                        });
                        on_event(StorageEvent::TurnDone {
                            timestamp: Utc::now(),
                        });
                        return Ok(());
                    }

                    on_event(StorageEvent::Error {
                        message: error.to_string(),
                    });
                    on_event(StorageEvent::TurnDone {
                        timestamp: Utc::now(),
                    });
                    return Ok(());
                }
            };

            if !output.content.is_empty() || !output.tool_calls.is_empty() {
                on_event(StorageEvent::AssistantFinal {
                    content: output.content.clone(),
                });
            }

            let tool_calls = output.tool_calls.clone();
            messages.push(LlmMessage::Assistant {
                content: output.content,
                tool_calls: output.tool_calls,
            });

            if tool_calls.is_empty() {
                on_event(StorageEvent::TurnDone {
                    timestamp: Utc::now(),
                });
                return Ok(());
            }

            for call in tool_calls {
                if cancel.is_cancelled() {
                    on_event(StorageEvent::Error {
                        message: "interrupted".to_string(),
                    });
                    on_event(StorageEvent::TurnDone {
                        timestamp: Utc::now(),
                    });
                    return Ok(());
                }

                on_event(StorageEvent::ToolCall {
                    tool_call_id: call.id.clone(),
                    tool_name: call.name.clone(),
                    args: call.args.clone(),
                });

                let start = Instant::now();
                let result = self.tools.execute(&call, cancel.child_token()).await;
                let duration_ms = start.elapsed().as_millis() as u64;

                on_event(StorageEvent::ToolResult {
                    tool_call_id: call.id.clone(),
                    output: if result.ok {
                        result.output.clone()
                    } else {
                        format!(
                            "tool execution failed: {}\n{}",
                            result.error.as_deref().unwrap_or("unknown error"),
                            result.output
                        )
                    },
                    success: result.ok,
                    duration_ms,
                });

                messages.push(LlmMessage::Tool {
                    tool_call_id: call.id,
                    content: result.model_content(),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use anyhow::{anyhow, Result};
    use async_trait::async_trait;
    use ipc::Phase;
    use serde_json::json;
    use tokio::time::{sleep, Duration};
    use tokio_util::sync::CancellationToken;

    use crate::action::{LlmMessage, ToolCallRequest, ToolDefinition, ToolExecutionResult};
    use crate::events::StorageEvent;
    use crate::llm::{EventSink, LlmEvent, LlmOutput, LlmProvider, LlmRequest};
    use crate::projection::AgentState;
    use crate::provider_factory::ProviderFactory;
    use crate::tools::registry::ToolRegistry;
    use crate::tools::Tool;

    use super::AgentLoop;

    fn make_state(user_text: &str) -> AgentState {
        AgentState {
            session_id: "test".into(),
            working_dir: PathBuf::from("/tmp"),
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
        async fn generate(
            &self,
            request: LlmRequest,
            sink: Option<EventSink>,
        ) -> Result<LlmOutput> {
            if self.delay > Duration::from_millis(0) {
                tokio::select! {
                    _ = request.cancel.cancelled() => return Err(anyhow!("cancelled")),
                    _ = sleep(self.delay) => {}
                }
            }
            let response = self
                .responses
                .lock()
                .expect("lock should work")
                .pop_front()
                .ok_or_else(|| anyhow!("no scripted response"))?;

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

    struct StaticProviderFactory {
        provider: Arc<dyn LlmProvider>,
    }

    impl ProviderFactory for StaticProviderFactory {
        fn build(&self) -> Result<Arc<dyn LlmProvider>> {
            Ok(self.provider.clone())
        }
    }

    #[async_trait]
    impl LlmProvider for StreamingProvider {
        async fn generate(
            &self,
            request: LlmRequest,
            sink: Option<EventSink>,
        ) -> Result<LlmOutput> {
            let Some(sink) = sink else {
                return Ok(self.response.clone());
            };

            for delta in self.response.content.chars() {
                sink(LlmEvent::TextDelta(delta.to_string()));

                tokio::select! {
                    _ = request.cancel.cancelled() => return Err(anyhow!("cancelled")),
                    _ = sleep(self.per_delta_delay) => {}
                }
            }

            Ok(self.response.clone())
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
            cancel: CancellationToken,
        ) -> Result<ToolExecutionResult> {
            tokio::select! {
                _ = cancel.cancelled() => Err(anyhow!("tool cancelled")),
                _ = sleep(Duration::from_millis(250)) => Ok(ToolExecutionResult {
                    tool_call_id,
                    tool_name: "slowTool".to_string(),
                    ok: true,
                    output: "ok".to_string(),
                    error: None,
                    metadata: None,
                    duration_ms: 250,
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
            _cancel: CancellationToken,
        ) -> Result<ToolExecutionResult> {
            Ok(ToolExecutionResult {
                tool_call_id,
                tool_name: "quickTool".to_string(),
                ok: true,
                output: "ok".to_string(),
                error: None,
                metadata: None,
                duration_ms: 1,
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
                        name: "listDir".to_string(),
                        args: json!({}),
                    }],
                },
                LlmOutput {
                    content: "done".to_string(),
                    tool_calls: vec![],
                },
            ])),
            delay: Duration::from_millis(0),
        });

        let tools = ToolRegistry::with_v1_defaults();
        let factory = Arc::new(StaticProviderFactory { provider });
        let loop_runner = AgentLoop::new(factory, tools).with_max_steps(8);
        let state = make_state("list files");

        let events: Arc<Mutex<Vec<StorageEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let events_clone = events.clone();
        let mut on_event = move |e: StorageEvent| {
            events_clone.lock().expect("lock").push(e);
        };

        loop_runner
            .run_turn(&state, &mut on_event, CancellationToken::new())
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
            }])),
            delay: Duration::from_millis(0),
        });

        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(SlowTool));

        let factory = Arc::new(StaticProviderFactory { provider });
        let loop_runner = AgentLoop::new(factory, tools).with_max_steps(8);
        let state = make_state("run slow");

        let events: Arc<Mutex<Vec<StorageEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();
        let events_clone = events.clone();

        let cancel_task = tokio::spawn(async move {
            sleep(Duration::from_millis(40)).await;
            cancel_clone.cancel();
        });

        let mut on_event = move |e: StorageEvent| {
            events_clone.lock().expect("lock").push(e);
        };
        loop_runner
            .run_turn(&state, &mut on_event, cancel)
            .await
            .expect("turn should end cleanly");
        cancel_task.await.expect("cancel task should join");

        let events = events.lock().expect("lock").clone();
        let has_error = events
            .iter()
            .any(|e| matches!(e, StorageEvent::Error { message } if message == "interrupted"));
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
            },
            per_delta_delay: Duration::from_millis(20),
        });
        let factory = Arc::new(StaticProviderFactory { provider });
        let loop_runner = AgentLoop::new(factory, ToolRegistry::new());
        let state = make_state("stream please");

        let events: Arc<Mutex<Vec<StorageEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let events_for_task = events.clone();

        let run_task = tokio::spawn(async move {
            let mut on_event = move |event: StorageEvent| {
                events_for_task.lock().expect("lock").push(event);
            };

            loop_runner
                .run_turn(&state, &mut on_event, CancellationToken::new())
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
            })
            .collect::<Vec<_>>();

        let provider = Arc::new(ScriptedProvider {
            responses: Mutex::new(VecDeque::from(scripted)),
            delay: Duration::from_millis(0),
        });

        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(QuickTool));

        let factory = Arc::new(StaticProviderFactory { provider });
        let loop_runner = AgentLoop::new(factory, tools);
        let state = make_state("loop test");

        let events: Arc<Mutex<Vec<StorageEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let events_clone = events.clone();
        let mut on_event = move |e: StorageEvent| {
            events_clone.lock().expect("lock").push(e);
        };

        loop_runner
            .run_turn(&state, &mut on_event, CancellationToken::new())
            .await
            .expect("turn should complete");

        let events = events.lock().expect("lock").clone();
        let has_max_error = events.iter().any(|event| {
            matches!(
                event,
                StorageEvent::Error { message }
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
}
