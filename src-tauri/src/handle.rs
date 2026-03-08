use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use astrcode_core::{AgentRuntime, StorageEvent};
use ipc::{AgentEvent, AgentEventKind, Phase, ToolCallResultEnvelope};

pub struct AgentHandle {
    runtime: Mutex<AgentRuntime>,
    cancel: Mutex<Option<CancellationToken>>,
}

impl AgentHandle {
    pub fn new() -> anyhow::Result<Self> {
        let runtime = AgentRuntime::new_session()?;
        Ok(Self {
            runtime: Mutex::new(runtime),
            cancel: Mutex::new(None),
        })
    }

    pub async fn submit_prompt(&self, text: String, app: AppHandle) -> Result<(), String> {
        // Cancel any previous in-flight turn.
        {
            let mut guard = self.cancel.lock().await;
            if let Some(prev) = guard.take() {
                prev.cancel();
            }
        }

        let turn_id = uuid::Uuid::new_v4().to_string();
        let cancel_token = CancellationToken::new();

        {
            let mut guard = self.cancel.lock().await;
            *guard = Some(cancel_token.clone());
        }

        // Emit PhaseChanged(Thinking) before starting the turn.
        let _ = emit_agent_event(&app, &turn_id, AgentEventKind::PhaseChanged {
            turn_id: Some(turn_id.clone()),
            phase: Phase::Thinking,
        });

        let mut runtime = self.runtime.lock().await;
        let cancel = cancel_token;
        let tid = turn_id.clone();

        let result = runtime
            .submit(text, cancel, |event| {
                map_and_emit(&app, &tid, event);
            })
            .await;

        if let Err(e) = result {
            eprintln!("agent turn error: {e}");
            return Err(e.to_string());
        }

        Ok(())
    }

    pub async fn interrupt(&self) -> Result<(), String> {
        let mut guard = self.cancel.lock().await;
        if let Some(token) = guard.take() {
            token.cancel();
        }
        Ok(())
    }
}

/// Convert a StorageEvent into one or more AgentEvents and emit to the frontend.
fn map_and_emit(app: &AppHandle, turn_id: &str, event: &StorageEvent) {
    match event {
        StorageEvent::UserMessage { .. } => {
            // No direct AgentEvent for the user message itself.
        }

        StorageEvent::AssistantDelta { token } => {
            emit_agent_event(app, turn_id, AgentEventKind::PhaseChanged {
                turn_id: Some(turn_id.to_string()),
                phase: Phase::Streaming,
            });
            emit_agent_event(app, turn_id, AgentEventKind::ModelDelta {
                turn_id: turn_id.to_string(),
                delta: token.clone(),
            });
        }

        StorageEvent::AssistantFinal { content } => {
            if !content.is_empty() {
                // Emit as ModelDelta chunks so the frontend streaming path still works.
                emit_agent_event(app, turn_id, AgentEventKind::PhaseChanged {
                    turn_id: Some(turn_id.to_string()),
                    phase: Phase::Streaming,
                });
                for chunk in chunk_text(content, 24) {
                    emit_agent_event(app, turn_id, AgentEventKind::ModelDelta {
                        turn_id: turn_id.to_string(),
                        delta: chunk,
                    });
                }
            }
        }

        StorageEvent::ToolCall {
            tool_call_id,
            tool_name,
            args,
        } => {
            emit_agent_event(app, turn_id, AgentEventKind::PhaseChanged {
                turn_id: Some(turn_id.to_string()),
                phase: Phase::CallingTool,
            });
            emit_agent_event(app, turn_id, AgentEventKind::ToolCallStart {
                turn_id: turn_id.to_string(),
                tool_call_id: tool_call_id.clone(),
                tool_name: tool_name.clone(),
                args: args.clone(),
            });
        }

        StorageEvent::ToolResult {
            tool_call_id,
            output,
            success,
            duration_ms,
        } => {
            emit_agent_event(app, turn_id, AgentEventKind::ToolCallResult {
                turn_id: turn_id.to_string(),
                result: ToolCallResultEnvelope {
                    tool_call_id: tool_call_id.clone(),
                    tool_name: String::new(), // tool_name not in ToolResult event
                    ok: *success,
                    output: output.clone(),
                    error: if *success { None } else { Some(output.clone()) },
                    metadata: None,
                    duration_ms: *duration_ms as u128,
                },
            });
        }

        StorageEvent::TurnDone { .. } => {
            emit_agent_event(app, turn_id, AgentEventKind::PhaseChanged {
                turn_id: Some(turn_id.to_string()),
                phase: Phase::Done,
            });
            emit_agent_event(app, turn_id, AgentEventKind::TurnDone {
                turn_id: turn_id.to_string(),
            });
            emit_agent_event(app, turn_id, AgentEventKind::PhaseChanged {
                turn_id: None,
                phase: Phase::Idle,
            });
        }

        StorageEvent::Error { message } => {
            if message == "interrupted" {
                emit_agent_event(app, turn_id, AgentEventKind::PhaseChanged {
                    turn_id: Some(turn_id.to_string()),
                    phase: Phase::Interrupted,
                });
            } else {
                emit_agent_event(app, turn_id, AgentEventKind::Error {
                    turn_id: Some(turn_id.to_string()),
                    code: "agent_error".to_string(),
                    message: message.clone(),
                });
            }
        }

        StorageEvent::SessionStart { .. } => {}
    }
}

fn emit_agent_event(app: &AppHandle, _turn_id: &str, kind: AgentEventKind) {
    let event = AgentEvent::new(kind);
    if let Err(e) = app.emit("agent-event", &event) {
        eprintln!("failed to emit agent-event: {e}");
    }
}

fn chunk_text(input: &str, chunk_size: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut count = 0;
    for ch in input.chars() {
        current.push(ch);
        count += 1;
        if count >= chunk_size {
            chunks.push(std::mem::take(&mut current));
            count = 0;
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}
