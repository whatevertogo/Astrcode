use std::path::PathBuf;

use ipc::Phase;

use crate::action::{LlmMessage, ToolCallRequest};
use crate::events::StorageEvent;

#[derive(Debug, Clone)]
pub struct AgentState {
    pub session_id: String,
    pub working_dir: PathBuf,
    pub messages: Vec<LlmMessage>,
    pub phase: Phase,
    pub turn_count: usize,
}

impl Default for AgentState {
    fn default() -> Self {
        Self {
            session_id: String::new(),
            working_dir: PathBuf::new(),
            messages: Vec::new(),
            phase: Phase::Idle,
            turn_count: 0,
        }
    }
}

/// Pure function: project an event sequence into an AgentState.
/// No IO, no side effects.
pub fn project(events: &[StorageEvent]) -> AgentState {
    let mut state = AgentState::default();

    // Buffer for assembling LlmMessage::Assistant with tool_calls.
    // When we see AssistantFinal we store the content; subsequent ToolCall
    // events accumulate tool_calls.  The buffer is flushed when we encounter
    // a ToolResult (after all calls in a step), TurnDone, or the next
    // UserMessage — whichever comes first.
    let mut pending_content: Option<String> = None;
    let mut pending_tool_calls: Vec<ToolCallRequest> = Vec::new();

    let flush = |state: &mut AgentState,
                 pending_content: &mut Option<String>,
                 pending_tool_calls: &mut Vec<ToolCallRequest>| {
        if pending_content.is_some() || !pending_tool_calls.is_empty() {
            let content = pending_content.take().unwrap_or_default();
            state.messages.push(LlmMessage::Assistant {
                content,
                tool_calls: std::mem::take(pending_tool_calls),
            });
        }
    };

    for event in events {
        match event {
            StorageEvent::SessionStart {
                session_id,
                working_dir,
                ..
            } => {
                state.session_id = session_id.clone();
                state.working_dir = PathBuf::from(working_dir);
            }

            StorageEvent::UserMessage { content, .. } => {
                flush(&mut state, &mut pending_content, &mut pending_tool_calls);
                state.messages.push(LlmMessage::User {
                    content: content.clone(),
                });
                state.phase = Phase::Thinking;
            }

            StorageEvent::AssistantFinal { content } => {
                // If there's already a pending assistant (from a previous step
                // in the same turn that wasn't flushed), flush it first.
                flush(&mut state, &mut pending_content, &mut pending_tool_calls);
                pending_content = Some(content.clone());
            }

            StorageEvent::ToolCall {
                tool_call_id,
                tool_name,
                args,
                ..
            } => {
                pending_tool_calls.push(ToolCallRequest {
                    id: tool_call_id.clone(),
                    name: tool_name.clone(),
                    args: args.clone(),
                });
            }

            StorageEvent::ToolResult {
                tool_call_id,
                output,
                success,
                ..
            } => {
                // Flush the assistant message that triggered these tool calls.
                flush(&mut state, &mut pending_content, &mut pending_tool_calls);

                let content = if *success {
                    output.clone()
                } else {
                    format!("tool execution failed:\n{output}")
                };
                state.messages.push(LlmMessage::Tool {
                    tool_call_id: tool_call_id.clone(),
                    content,
                });
            }

            StorageEvent::TurnDone { .. } => {
                flush(&mut state, &mut pending_content, &mut pending_tool_calls);
                state.phase = Phase::Idle;
                state.turn_count += 1;
            }

            // AssistantDelta and Error don't participate in state rebuilding.
            StorageEvent::AssistantDelta { .. } | StorageEvent::Error { .. } => {}
        }
    }

    // Flush any trailing pending content (e.g. replay stops mid-turn).
    flush(&mut state, &mut pending_content, &mut pending_tool_calls);

    state
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::events::StorageEvent;

    fn ts() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc::now()
    }

    #[test]
    fn empty_events_produce_default_state() {
        let state = project(&[]);
        assert_eq!(state.session_id, "");
        assert!(state.messages.is_empty());
        assert_eq!(state.phase, Phase::Idle);
        assert_eq!(state.turn_count, 0);
    }

    #[test]
    fn session_start_and_user_message() {
        let events = vec![
            StorageEvent::SessionStart {
                session_id: "s1".into(),
                timestamp: ts(),
                working_dir: "/tmp".into(),
            },
            StorageEvent::UserMessage {
                content: "hello".into(),
                timestamp: ts(),
            },
        ];
        let state = project(&events);
        assert_eq!(state.session_id, "s1");
        assert_eq!(state.working_dir, PathBuf::from("/tmp"));
        assert_eq!(state.messages.len(), 1);
        assert!(matches!(&state.messages[0], LlmMessage::User { content } if content == "hello"));
        assert_eq!(state.phase, Phase::Thinking);
    }

    #[test]
    fn turn_done_sets_idle_and_increments_count() {
        let events = vec![
            StorageEvent::SessionStart {
                session_id: "s1".into(),
                timestamp: ts(),
                working_dir: "/tmp".into(),
            },
            StorageEvent::UserMessage {
                content: "hi".into(),
                timestamp: ts(),
            },
            StorageEvent::AssistantFinal {
                content: "hello!".into(),
            },
            StorageEvent::TurnDone { timestamp: ts() },
        ];
        let state = project(&events);
        assert_eq!(state.phase, Phase::Idle);
        assert_eq!(state.turn_count, 1);
        assert_eq!(state.messages.len(), 2); // User + Assistant
    }

    #[test]
    fn multi_turn_with_tool_calls_rebuilds_correctly() {
        let events = vec![
            StorageEvent::SessionStart {
                session_id: "s1".into(),
                timestamp: ts(),
                working_dir: "/tmp".into(),
            },
            // Turn 1: user → assistant with tool call → tool result → final answer
            StorageEvent::UserMessage {
                content: "list files".into(),
                timestamp: ts(),
            },
            StorageEvent::AssistantFinal {
                content: "".into(),
            },
            StorageEvent::ToolCall {
                tool_call_id: "tc1".into(),
                tool_name: "listDir".into(),
                args: json!({"path": "."}),
            },
            StorageEvent::ToolResult {
                tool_call_id: "tc1".into(),
                output: "file1.txt\nfile2.txt".into(),
                success: true,
                duration_ms: 10,
            },
            StorageEvent::AssistantFinal {
                content: "Here are the files".into(),
            },
            StorageEvent::TurnDone { timestamp: ts() },
            // Turn 2: simple user → assistant
            StorageEvent::UserMessage {
                content: "thanks".into(),
                timestamp: ts(),
            },
            StorageEvent::AssistantFinal {
                content: "You're welcome!".into(),
            },
            StorageEvent::TurnDone { timestamp: ts() },
        ];
        let state = project(&events);

        assert_eq!(state.turn_count, 2);
        assert_eq!(state.phase, Phase::Idle);

        // Turn 1: User, Assistant(empty + tool_calls), Tool, Assistant(final)
        // Turn 2: User, Assistant
        // Total: 6 messages
        assert_eq!(state.messages.len(), 6);

        // First assistant should have one tool_call
        match &state.messages[1] {
            LlmMessage::Assistant {
                content,
                tool_calls,
            } => {
                assert_eq!(content, "");
                assert_eq!(tool_calls.len(), 1);
                assert_eq!(tool_calls[0].name, "listDir");
            }
            other => panic!("expected Assistant, got {:?}", other),
        }

        // Tool result
        match &state.messages[2] {
            LlmMessage::Tool {
                tool_call_id,
                content,
            } => {
                assert_eq!(tool_call_id, "tc1");
                assert!(content.contains("file1.txt"));
            }
            other => panic!("expected Tool, got {:?}", other),
        }
    }

    #[test]
    fn assistant_delta_and_error_are_ignored() {
        let events = vec![
            StorageEvent::SessionStart {
                session_id: "s1".into(),
                timestamp: ts(),
                working_dir: "/tmp".into(),
            },
            StorageEvent::UserMessage {
                content: "hi".into(),
                timestamp: ts(),
            },
            StorageEvent::AssistantDelta {
                token: "hel".into(),
            },
            StorageEvent::AssistantDelta { token: "lo".into() },
            StorageEvent::AssistantFinal {
                content: "hello".into(),
            },
            StorageEvent::Error {
                message: "some error".into(),
            },
            StorageEvent::TurnDone { timestamp: ts() },
        ];
        let state = project(&events);
        assert_eq!(state.messages.len(), 2); // User + Assistant only
        assert_eq!(state.turn_count, 1);
    }

    #[test]
    fn tool_messages_require_synthetic_assistant_when_content_is_empty() {
        let events = vec![
            StorageEvent::SessionStart {
                session_id: "s1".into(),
                timestamp: ts(),
                working_dir: "/tmp".into(),
            },
            StorageEvent::UserMessage {
                content: "run tool".into(),
                timestamp: ts(),
            },
            StorageEvent::ToolCall {
                tool_call_id: "tc1".into(),
                tool_name: "listDir".into(),
                args: json!({"path": "."}),
            },
            StorageEvent::ToolResult {
                tool_call_id: "tc1".into(),
                output: "[]".into(),
                success: true,
                duration_ms: 2,
            },
            StorageEvent::TurnDone { timestamp: ts() },
        ];

        let state = project(&events);
        assert_eq!(state.messages.len(), 3, "expected user + assistant + tool");

        match &state.messages[1] {
            LlmMessage::Assistant { content, tool_calls } => {
                assert_eq!(content, "");
                assert_eq!(tool_calls.len(), 1);
                assert_eq!(tool_calls[0].id, "tc1");
            }
            other => panic!("expected assistant before tool message, got {:?}", other),
        }

        assert!(matches!(&state.messages[2], LlmMessage::Tool { tool_call_id, .. } if tool_call_id == "tc1"));
    }
}
