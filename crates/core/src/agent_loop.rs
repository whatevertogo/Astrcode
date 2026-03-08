use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use chrono::Utc;
use tokio_util::sync::CancellationToken;

use crate::action::LlmMessage;
use crate::events::StorageEvent;
use crate::llm::LlmProvider;
use crate::projection::AgentState;
use crate::tools::registry::ToolRegistry;

pub struct AgentLoop {
    provider: Arc<dyn LlmProvider>,
    tools: ToolRegistry,
    max_steps: usize,
}

impl AgentLoop {
    pub fn new(provider: Arc<dyn LlmProvider>, tools: ToolRegistry) -> Self {
        Self {
            provider,
            tools,
            max_steps: 8,
        }
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

        for _ in 0..self.max_steps {
            if cancel.is_cancelled() {
                on_event(StorageEvent::Error {
                    message: "interrupted".to_string(),
                });
                on_event(StorageEvent::TurnDone {
                    timestamp: Utc::now(),
                });
                return Ok(());
            }

            let response = match self
                .provider
                .complete(&messages, &self.tools.definitions(), cancel.child_token())
                .await
            {
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

            // TODO: streaming - emit AssistantDelta per token
            // Currently LlmProvider::complete() returns the full response at once.
            // When streaming is implemented, emit AssistantDelta { token } for each chunk here.

            if !response.content.is_empty() {
                on_event(StorageEvent::AssistantFinal {
                    content: response.content.clone(),
                });
            }

            let tool_calls = response.tool_calls.clone();
            messages.push(LlmMessage::Assistant {
                content: response.content,
                tool_calls: response.tool_calls,
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

        on_event(StorageEvent::Error {
            message: "turn exceeded max tool iteration steps".to_string(),
        });
        on_event(StorageEvent::TurnDone {
            timestamp: Utc::now(),
        });

        Ok(())
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

    use crate::action::{LlmMessage, LlmResponse, ToolCallRequest, ToolDefinition, ToolExecutionResult};
    use crate::events::StorageEvent;
    use crate::llm::LlmProvider;
    use crate::projection::AgentState;
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
        responses: Mutex<VecDeque<LlmResponse>>,
        delay: Duration,
    }

    #[async_trait]
    impl LlmProvider for ScriptedProvider {
        async fn complete(
            &self,
            _messages: &[LlmMessage],
            _tools: &[ToolDefinition],
            cancel: CancellationToken,
        ) -> Result<LlmResponse> {
            if self.delay > Duration::from_millis(0) {
                tokio::select! {
                    _ = cancel.cancelled() => return Err(anyhow!("cancelled")),
                    _ = sleep(self.delay) => {}
                }
            }
            self.responses
                .lock()
                .expect("lock should work")
                .pop_front()
                .ok_or_else(|| anyhow!("no scripted response"))
        }
    }

    struct SlowTool;

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

    #[tokio::test]
    async fn tool_events_are_ordered_and_turn_finishes() {
        let provider = Arc::new(ScriptedProvider {
            responses: Mutex::new(VecDeque::from([
                LlmResponse {
                    content: "".to_string(),
                    tool_calls: vec![ToolCallRequest {
                        id: "call1".to_string(),
                        name: "listDir".to_string(),
                        args: json!({}),
                    }],
                },
                LlmResponse {
                    content: "done".to_string(),
                    tool_calls: vec![],
                },
            ])),
            delay: Duration::from_millis(0),
        });

        let tools = ToolRegistry::with_v1_defaults();
        let loop_runner = AgentLoop::new(provider, tools);
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
            responses: Mutex::new(VecDeque::from([LlmResponse {
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

        let loop_runner = AgentLoop::new(provider, tools);
        let state = make_state("run slow");

        let events: Arc<Mutex<Vec<StorageEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();
        let events_clone = events.clone();

        let handle = tokio::spawn(async move {
            let mut on_event = move |e: StorageEvent| {
                events_clone.lock().expect("lock").push(e);
            };
            loop_runner
                .run_turn(&state, &mut on_event, cancel)
                .await
                .expect("turn should end cleanly")
        });

        sleep(Duration::from_millis(40)).await;
        cancel_clone.cancel();
        handle.await.expect("task should join");

        let events = events.lock().expect("lock").clone();
        let has_error = events.iter().any(|e| {
            matches!(e, StorageEvent::Error { message } if message == "interrupted")
        });
        let has_done = events
            .iter()
            .any(|e| matches!(e, StorageEvent::TurnDone { .. }));

        assert!(has_error, "should have Error(interrupted)");
        assert!(has_done, "should have TurnDone");
    }
}
