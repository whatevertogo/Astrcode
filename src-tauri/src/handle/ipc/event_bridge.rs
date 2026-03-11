use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::ipc::Channel;

use astrcode_core::{llm::LlmEvent, StorageEvent};
use ipc::{AgentEvent, AgentEventKind, Phase, ToolCallResultEnvelope};

pub(crate) struct TurnEventBridge {
    turn_id: String,
    pending_tool_names: HashMap<String, String>,
    streaming_phase_emitted: AtomicBool,
}

impl TurnEventBridge {
    pub(crate) fn new(turn_id: String) -> Self {
        Self {
            turn_id,
            pending_tool_names: HashMap::new(),
            streaming_phase_emitted: AtomicBool::new(false),
        }
    }

    pub(crate) fn emit_thinking(&self, channel: &Channel<AgentEvent>) {
        send_agent_event(
            channel,
            AgentEventKind::PhaseChanged {
                turn_id: Some(self.turn_id.clone()),
                phase: Phase::Thinking,
            },
        );
    }

    pub(crate) fn forward_storage_event(
        &mut self,
        channel: &Channel<AgentEvent>,
        event: &StorageEvent,
    ) {
        if matches!(event, StorageEvent::AssistantDelta { .. }) {
            if !self.streaming_phase_emitted.swap(true, Ordering::Relaxed) {
                send_agent_event(
                    channel,
                    AgentEventKind::PhaseChanged {
                        turn_id: Some(self.turn_id.clone()),
                        phase: Phase::Streaming,
                    },
                );
            }
        } else {
            self.streaming_phase_emitted.store(false, Ordering::Relaxed);
        }

        for kind in collect_event_kinds(&self.turn_id, event, &mut self.pending_tool_names) {
            send_agent_event(channel, kind);
        }
    }

    pub(crate) fn forward_llm_event(&mut self, channel: &Channel<AgentEvent>, event: &LlmEvent) {
        if let LlmEvent::ThinkingDelta(delta) = event {
            send_agent_event(
                channel,
                AgentEventKind::ThinkingDelta {
                    turn_id: self.turn_id.clone(),
                    delta: delta.clone(),
                },
            );
        }
    }
}

fn collect_event_kinds(
    turn_id: &str,
    event: &StorageEvent,
    pending_tool_names: &mut HashMap<String, String>,
) -> Vec<AgentEventKind> {
    match event {
        StorageEvent::UserMessage { .. } => Vec::new(),
        StorageEvent::AssistantDelta { token } => {
            vec![AgentEventKind::ModelDelta {
                turn_id: turn_id.to_string(),
                delta: token.clone(),
            }]
        }
        StorageEvent::AssistantFinal { .. } => Vec::new(),
        StorageEvent::ToolCall {
            tool_call_id,
            tool_name,
            args,
        } => {
            pending_tool_names.insert(tool_call_id.clone(), tool_name.clone());
            vec![
                AgentEventKind::PhaseChanged {
                    turn_id: Some(turn_id.to_string()),
                    phase: Phase::CallingTool,
                },
                AgentEventKind::ToolCallStart {
                    turn_id: turn_id.to_string(),
                    tool_call_id: tool_call_id.clone(),
                    tool_name: tool_name.clone(),
                    args: args.clone(),
                },
            ]
        }
        StorageEvent::ToolResult {
            tool_call_id,
            tool_name,
            output,
            success,
            duration_ms,
        } => {
            // Clean up pending_tool_names (may not exist if already cleaned)
            let _ = pending_tool_names.remove(tool_call_id);
            vec![AgentEventKind::ToolCallResult {
                turn_id: turn_id.to_string(),
                result: ToolCallResultEnvelope {
                    tool_call_id: tool_call_id.clone(),
                    tool_name: tool_name.clone(),
                    ok: *success,
                    output: output.clone(),
                    error: if *success { None } else { Some(output.clone()) },
                    metadata: None,
                    duration_ms: *duration_ms as u128,
                },
            }]
        }
        StorageEvent::TurnDone { .. } => {
            vec![
                AgentEventKind::PhaseChanged {
                    turn_id: Some(turn_id.to_string()),
                    phase: Phase::Done,
                },
                AgentEventKind::TurnDone {
                    turn_id: turn_id.to_string(),
                },
                AgentEventKind::PhaseChanged {
                    turn_id: None,
                    phase: Phase::Idle,
                },
            ]
        }
        StorageEvent::Error { message } => {
            if message == "interrupted" {
                vec![AgentEventKind::PhaseChanged {
                    turn_id: Some(turn_id.to_string()),
                    phase: Phase::Interrupted,
                }]
            } else {
                vec![AgentEventKind::Error {
                    turn_id: Some(turn_id.to_string()),
                    code: "agent_error".to_string(),
                    message: message.clone(),
                }]
            }
        }
        StorageEvent::SessionStart { session_id, .. } => {
            vec![AgentEventKind::SessionStarted {
                session_id: session_id.clone(),
            }]
        }
    }
}

fn send_agent_event(channel: &Channel<AgentEvent>, kind: AgentEventKind) {
    let event = AgentEvent::new(kind);
    if let Err(error) = channel.send(event) {
        eprintln!("failed to send agent-event over channel: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assistant_final_produces_no_events() {
        let mut pending = HashMap::new();
        let events = collect_event_kinds(
            "turn-1",
            &StorageEvent::AssistantFinal {
                content: "hello world".to_string(),
                reasoning_content: None,
            },
            &mut pending,
        );

        assert!(
            events.is_empty(),
            "AssistantFinal should not produce IPC events (content arrives via deltas)"
        );
    }

    #[test]
    fn assistant_delta_produces_only_model_delta() {
        let mut pending = HashMap::new();
        let events = collect_event_kinds(
            "turn-2",
            &StorageEvent::AssistantDelta {
                token: "hello".to_string(),
            },
            &mut pending,
        );

        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            AgentEventKind::ModelDelta { turn_id, delta }
            if turn_id == "turn-2" && delta == "hello"
        ));
    }

    #[test]
    fn tool_result_preserves_output_and_failure_state() {
        let mut pending = HashMap::new();
        let _ = collect_event_kinds(
            "turn-3",
            &StorageEvent::ToolCall {
                tool_call_id: "tool-1".to_string(),
                tool_name: "shell".to_string(),
                args: serde_json::json!({ "command": "echo ok" }),
            },
            &mut pending,
        );
        let events = collect_event_kinds(
            "turn-3",
            &StorageEvent::ToolResult {
                tool_call_id: "tool-1".to_string(),
                tool_name: "shell".to_string(),
                output: "boom".to_string(),
                success: false,
                duration_ms: 42,
            },
            &mut pending,
        );

        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            AgentEventKind::ToolCallResult { result, .. }
            if result.tool_call_id == "tool-1"
                && result.tool_name == "shell"
                && result.output == "boom"
                && result.error.as_deref() == Some("boom")
                && !result.ok
                && result.duration_ms == 42
        ));
    }
}
