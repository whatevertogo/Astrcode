use std::path::PathBuf;

use crate::Phase;

use crate::event::StorageEvent;
use crate::{split_assistant_content, LlmMessage, ReasoningContent, ToolCallRequest};

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

#[derive(Debug, Clone, Default)]
pub struct AgentStateProjector {
    state: AgentState,
    pending_content: Option<String>,
    pending_reasoning: Option<ReasoningContent>,
    pending_tool_calls: Vec<ToolCallRequest>,
}

impl AgentStateProjector {
    pub fn from_events(events: &[StorageEvent]) -> Self {
        let mut projector = Self::default();
        for event in events {
            projector.apply(event);
        }
        projector
    }

    pub fn apply(&mut self, event: &StorageEvent) {
        match event {
            StorageEvent::SessionStart {
                session_id,
                working_dir,
                ..
            } => {
                self.state.session_id = session_id.clone();
                self.state.working_dir = PathBuf::from(working_dir);
            }

            StorageEvent::UserMessage { content, .. } => {
                self.flush_pending_assistant();
                self.state.messages.push(LlmMessage::User {
                    content: content.clone(),
                });
                self.state.phase = Phase::Thinking;
            }

            StorageEvent::AssistantFinal {
                content,
                reasoning_content,
                reasoning_signature,
                ..
            } => {
                self.flush_pending_assistant();
                let parts = split_assistant_content(content, reasoning_content.as_deref());
                self.pending_content = Some(parts.visible_content);
                self.pending_reasoning = parts.reasoning_content.map(|content| ReasoningContent {
                    content,
                    signature: reasoning_signature.clone(),
                });
            }

            StorageEvent::ToolCall {
                tool_call_id,
                tool_name,
                args,
                ..
            } => {
                self.pending_tool_calls.push(ToolCallRequest {
                    id: tool_call_id.clone(),
                    name: tool_name.clone(),
                    args: args.clone(),
                });
            }

            StorageEvent::ToolResult {
                tool_call_id,
                tool_name,
                output,
                success,
                error,
                metadata,
                duration_ms,
                ..
            } => {
                self.flush_pending_assistant();
                let result = crate::ToolExecutionResult {
                    tool_call_id: tool_call_id.clone(),
                    tool_name: tool_name.clone(),
                    ok: *success,
                    output: output.clone(),
                    error: error.clone(),
                    metadata: metadata.clone(),
                    duration_ms: *duration_ms,
                    truncated: false,
                };
                self.state.messages.push(LlmMessage::Tool {
                    tool_call_id: tool_call_id.clone(),
                    content: result.model_content(),
                });
            }

            StorageEvent::TurnDone { .. } => {
                self.flush_pending_assistant();
                self.state.phase = Phase::Idle;
                self.state.turn_count += 1;
            }

            StorageEvent::AssistantDelta { .. }
            | StorageEvent::ThinkingDelta { .. }
            | StorageEvent::Error { .. } => {}
        }
    }

    pub fn snapshot(&self) -> AgentState {
        let mut clone = self.clone();
        clone.flush_pending_assistant();
        clone.state
    }

    fn flush_pending_assistant(&mut self) {
        if self.pending_content.is_some() || !self.pending_tool_calls.is_empty() {
            let content = self.pending_content.take().unwrap_or_default();
            self.state.messages.push(LlmMessage::Assistant {
                content,
                tool_calls: std::mem::take(&mut self.pending_tool_calls),
                reasoning: self.pending_reasoning.take(),
            });
        }
    }
}

/// Pure function: project an event sequence into an AgentState.
/// No IO, no side effects.
pub fn project(events: &[StorageEvent]) -> AgentState {
    AgentStateProjector::from_events(events).snapshot()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::event::StorageEvent;

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
                turn_id: None,
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
                turn_id: None,
                content: "hi".into(),
                timestamp: ts(),
            },
            StorageEvent::AssistantFinal {
                turn_id: None,
                content: "hello!".into(),
                reasoning_content: None,
                reasoning_signature: None,
                timestamp: None,
            },
            StorageEvent::TurnDone {
                turn_id: None,
                timestamp: ts(),
            },
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
                turn_id: None,
                content: "list files".into(),
                timestamp: ts(),
            },
            StorageEvent::AssistantFinal {
                turn_id: None,
                content: "".into(),
                reasoning_content: None,
                reasoning_signature: None,
                timestamp: None,
            },
            StorageEvent::ToolCall {
                turn_id: None,
                tool_call_id: "tc1".into(),
                tool_name: "listDir".into(),
                args: json!({"path": "."}),
            },
            StorageEvent::ToolResult {
                turn_id: None,
                tool_call_id: "tc1".into(),
                tool_name: "listDir".into(),
                output: "file1.txt\nfile2.txt".into(),
                success: true,
                error: None,
                metadata: None,
                duration_ms: 10,
            },
            StorageEvent::AssistantFinal {
                turn_id: None,
                content: "Here are the files".into(),
                reasoning_content: None,
                reasoning_signature: None,
                timestamp: None,
            },
            StorageEvent::TurnDone {
                turn_id: None,
                timestamp: ts(),
            },
            // Turn 2: simple user → assistant
            StorageEvent::UserMessage {
                turn_id: None,
                content: "thanks".into(),
                timestamp: ts(),
            },
            StorageEvent::AssistantFinal {
                turn_id: None,
                content: "You're welcome!".into(),
                reasoning_content: None,
                reasoning_signature: None,
                timestamp: None,
            },
            StorageEvent::TurnDone {
                turn_id: None,
                timestamp: ts(),
            },
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
                ..
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
                turn_id: None,
                content: "hi".into(),
                timestamp: ts(),
            },
            StorageEvent::AssistantDelta {
                turn_id: None,
                token: "hel".into(),
            },
            StorageEvent::AssistantDelta {
                turn_id: None,
                token: "lo".into(),
            },
            StorageEvent::AssistantFinal {
                turn_id: None,
                content: "hello".into(),
                reasoning_content: None,
                reasoning_signature: None,
                timestamp: None,
            },
            StorageEvent::Error {
                turn_id: None,
                message: "some error".into(),
                timestamp: Some(ts()),
            },
            StorageEvent::TurnDone {
                turn_id: None,
                timestamp: ts(),
            },
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
                turn_id: None,
                content: "run tool".into(),
                timestamp: ts(),
            },
            StorageEvent::ToolCall {
                turn_id: None,
                tool_call_id: "tc1".into(),
                tool_name: "listDir".into(),
                args: json!({"path": "."}),
            },
            StorageEvent::ToolResult {
                turn_id: None,
                tool_call_id: "tc1".into(),
                tool_name: "listDir".into(),
                output: "[]".into(),
                success: true,
                error: None,
                metadata: None,
                duration_ms: 2,
            },
            StorageEvent::TurnDone {
                turn_id: None,
                timestamp: ts(),
            },
        ];

        let state = project(&events);
        assert_eq!(state.messages.len(), 3, "expected user + assistant + tool");

        match &state.messages[1] {
            LlmMessage::Assistant {
                content,
                tool_calls,
                ..
            } => {
                assert_eq!(content, "");
                assert_eq!(tool_calls.len(), 1);
                assert_eq!(tool_calls[0].id, "tc1");
            }
            other => panic!("expected assistant before tool message, got {:?}", other),
        }

        assert!(
            matches!(&state.messages[2], LlmMessage::Tool { tool_call_id, .. } if tool_call_id == "tc1")
        );
    }

    #[test]
    fn incremental_projector_matches_batch_projection() {
        let events = vec![
            StorageEvent::SessionStart {
                session_id: "s1".into(),
                timestamp: ts(),
                working_dir: "/tmp".into(),
            },
            StorageEvent::UserMessage {
                turn_id: None,
                content: "hello".into(),
                timestamp: ts(),
            },
            StorageEvent::AssistantFinal {
                turn_id: None,
                content: "hi".into(),
                reasoning_content: Some("thinking".into()),
                reasoning_signature: Some("sig".into()),
                timestamp: None,
            },
            StorageEvent::TurnDone {
                turn_id: None,
                timestamp: ts(),
            },
        ];

        let batch = project(&events);
        let mut projector = AgentStateProjector::default();
        for event in &events {
            projector.apply(event);
        }

        let incremental = projector.snapshot();
        assert_eq!(incremental.session_id, batch.session_id);
        assert_eq!(incremental.working_dir, batch.working_dir);
        assert_eq!(incremental.phase, batch.phase);
        assert_eq!(incremental.turn_count, batch.turn_count);
        assert_eq!(incremental.messages.len(), batch.messages.len());
    }
}
