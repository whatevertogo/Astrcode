use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use ipc::{AgentEvent, AgentEventKind, Phase};
use tokio_util::sync::CancellationToken;

use crate::action::LlmMessage;
use crate::llm::LlmProvider;
use crate::tools::registry::ToolRegistry;

#[async_trait]
pub trait EventSink: Send + Sync {
    async fn emit(&self, event: AgentEvent) -> Result<()>;
}

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

    pub async fn run_turn(
        &self,
        turn_id: String,
        user_text: String,
        sink: Arc<dyn EventSink>,
        cancel: CancellationToken,
    ) -> Result<()> {
        sink.emit(AgentEvent::new(AgentEventKind::PhaseChanged {
            turn_id: Some(turn_id.clone()),
            phase: Phase::Thinking,
        }))
        .await?;

        let mut messages = vec![LlmMessage::User { content: user_text }];

        for _ in 0..self.max_steps {
            if cancel.is_cancelled() {
                self.finish_interrupted(&turn_id, sink.as_ref()).await?;
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
                        self.finish_interrupted(&turn_id, sink.as_ref()).await?;
                        return Ok(());
                    }

                    sink.emit(AgentEvent::new(AgentEventKind::Error {
                        turn_id: Some(turn_id.clone()),
                        code: "llm_error".to_string(),
                        message: error.to_string(),
                    }))
                    .await?;
                    self.finish_done(&turn_id, sink.as_ref()).await?;
                    return Ok(());
                }
            };

            if !response.content.is_empty() {
                sink.emit(AgentEvent::new(AgentEventKind::PhaseChanged {
                    turn_id: Some(turn_id.clone()),
                    phase: Phase::Streaming,
                }))
                .await?;

                for chunk in chunk_text(&response.content, 24) {
                    if cancel.is_cancelled() {
                        self.finish_interrupted(&turn_id, sink.as_ref()).await?;
                        return Ok(());
                    }
                    sink.emit(AgentEvent::new(AgentEventKind::ModelDelta {
                        turn_id: turn_id.clone(),
                        delta: chunk,
                    }))
                    .await?;
                }
            }

            let tool_calls = response.tool_calls.clone();
            messages.push(LlmMessage::Assistant {
                content: response.content,
                tool_calls: response.tool_calls,
            });

            if tool_calls.is_empty() {
                self.finish_done(&turn_id, sink.as_ref()).await?;
                return Ok(());
            }

            for call in tool_calls {
                if cancel.is_cancelled() {
                    self.finish_interrupted(&turn_id, sink.as_ref()).await?;
                    return Ok(());
                }

                sink.emit(AgentEvent::new(AgentEventKind::PhaseChanged {
                    turn_id: Some(turn_id.clone()),
                    phase: Phase::CallingTool,
                }))
                .await?;
                sink.emit(AgentEvent::new(AgentEventKind::ToolCallStart {
                    turn_id: turn_id.clone(),
                    tool_call_id: call.id.clone(),
                    tool_name: call.name.clone(),
                    args: call.args.clone(),
                }))
                .await?;

                let result = self
                    .tools
                    .execute(&call, cancel.child_token())
                    .await;

                sink.emit(AgentEvent::new(AgentEventKind::ToolCallResult {
                    turn_id: turn_id.clone(),
                    result: result.clone().into_envelope(),
                }))
                .await?;

                messages.push(LlmMessage::Tool {
                    tool_call_id: call.id,
                    content: result.model_content(),
                });
            }
        }

        sink.emit(AgentEvent::new(AgentEventKind::Error {
            turn_id: Some(turn_id.clone()),
            code: "max_steps_exceeded".to_string(),
            message: "turn exceeded max tool iteration steps".to_string(),
        }))
        .await?;
        self.finish_done(&turn_id, sink.as_ref()).await?;

        Ok(())
    }

    async fn finish_done(&self, turn_id: &str, sink: &dyn EventSink) -> Result<()> {
        sink.emit(AgentEvent::new(AgentEventKind::PhaseChanged {
            turn_id: Some(turn_id.to_string()),
            phase: Phase::Done,
        }))
        .await?;
        sink.emit(AgentEvent::new(AgentEventKind::TurnDone {
            turn_id: turn_id.to_string(),
        }))
        .await?;
        sink.emit(AgentEvent::new(AgentEventKind::PhaseChanged {
            turn_id: None,
            phase: Phase::Idle,
        }))
        .await?;
        Ok(())
    }

    async fn finish_interrupted(&self, turn_id: &str, sink: &dyn EventSink) -> Result<()> {
        sink.emit(AgentEvent::new(AgentEventKind::PhaseChanged {
            turn_id: Some(turn_id.to_string()),
            phase: Phase::Interrupted,
        }))
        .await?;
        sink.emit(AgentEvent::new(AgentEventKind::TurnDone {
            turn_id: turn_id.to_string(),
        }))
        .await?;
        sink.emit(AgentEvent::new(AgentEventKind::PhaseChanged {
            turn_id: None,
            phase: Phase::Idle,
        }))
        .await?;
        Ok(())
    }
}

fn chunk_text(input: &str, chunk_size: usize) -> Vec<String> {
    if input.is_empty() {
        return Vec::new();
    }

    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut count = 0;

    for ch in input.chars() {
        current.push(ch);
        count += 1;
        if count >= chunk_size {
            chunks.push(current.clone());
            current.clear();
            count = 0;
        }
    }

    if !current.is_empty() {
        chunks.push(current);
    }

    chunks
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use anyhow::{anyhow, Result};
    use async_trait::async_trait;
    use ipc::AgentEventKind;
    use serde_json::json;
    use tokio::time::{sleep, Duration};
    use tokio_util::sync::CancellationToken;

    use crate::action::{LlmResponse, ToolCallRequest, ToolDefinition, ToolExecutionResult};
    use crate::llm::LlmProvider;
    use crate::tools::registry::ToolRegistry;
    use crate::tools::Tool;

    use super::{AgentLoop, EventSink};

    #[derive(Default)]
    struct MemorySink {
        events: Mutex<Vec<ipc::AgentEvent>>,
    }

    #[async_trait]
    impl EventSink for MemorySink {
        async fn emit(&self, event: ipc::AgentEvent) -> Result<()> {
            self.events.lock().expect("lock should work").push(event);
            Ok(())
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
            _messages: &[crate::action::LlmMessage],
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
        let sink = Arc::new(MemorySink::default());

        loop_runner
            .run_turn(
                "turn1".to_string(),
                "list files".to_string(),
                sink.clone(),
                CancellationToken::new(),
            )
            .await
            .expect("turn should complete");

        let events = sink.events.lock().expect("lock should work").clone();
        let start_idx = events
            .iter()
            .position(|event| matches!(event.kind, AgentEventKind::ToolCallStart { .. }))
            .expect("tool start event expected");
        let result_idx = events
            .iter()
            .position(|event| matches!(event.kind, AgentEventKind::ToolCallResult { .. }))
            .expect("tool result event expected");
        let done_idx = events
            .iter()
            .position(|event| matches!(event.kind, AgentEventKind::TurnDone { .. }))
            .expect("turn done event expected");

        assert!(start_idx < result_idx);
        assert!(result_idx < done_idx);
    }

    #[tokio::test]
    async fn interrupt_moves_turn_to_interrupted_without_extra_deltas() {
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
        let sink = Arc::new(MemorySink::default());
        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();
        let sink_for_task = sink.clone();

        let handle = tokio::spawn(async move {
            loop_runner
                .run_turn(
                    "turn-interrupt".to_string(),
                    "run slow".to_string(),
                    sink_for_task,
                    cancel,
                )
                .await
                .expect("turn should end cleanly")
        });

        sleep(Duration::from_millis(40)).await;
        cancel_clone.cancel();
        handle.await.expect("task should join");

        let events = sink.events.lock().expect("lock should work").clone();
        let interrupted = events.iter().any(|event| {
            matches!(
                event.kind,
                AgentEventKind::PhaseChanged {
                    phase: ipc::Phase::Interrupted,
                    ..
                }
            )
        });

        assert!(interrupted);
    }
}
