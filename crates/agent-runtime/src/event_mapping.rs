//! Runtime 事件到应用层 AgentEvent 的映射。
//!
//! 将 `RuntimeTurnEvent`（agent-runtime 内部事件）和 `StorageEvent`（持久化事件）
//! 转换为 `AgentEvent`（应用层 SSE 广播事件）。

use astrcode_core::{
    AgentEvent, AgentEventContext, StorageEvent, StorageEventPayload, ToolExecutionResult,
    llm::LlmEvent,
};
use astrcode_runtime_contract::RuntimeTurnEvent;

/// 将 `RuntimeTurnEvent` 转换为 0..N 个 `AgentEvent`。
pub fn runtime_turn_events_to_agent_events(
    event: &RuntimeTurnEvent,
    agent: &AgentEventContext,
) -> Vec<AgentEvent> {
    match event {
        RuntimeTurnEvent::ProviderStream { identity, event } => match event {
            LlmEvent::TextDelta(delta) if !delta.is_empty() => vec![AgentEvent::ModelDelta {
                turn_id: identity.turn_id.clone(),
                agent: agent.clone(),
                delta: delta.clone(),
            }],
            LlmEvent::ThinkingDelta(delta) if !delta.is_empty() => {
                vec![AgentEvent::ThinkingDelta {
                    turn_id: identity.turn_id.clone(),
                    agent: agent.clone(),
                    delta: delta.clone(),
                }]
            },
            LlmEvent::StreamRetryStarted {
                attempt,
                max_attempts,
                reason,
            } => vec![AgentEvent::StreamRetryStarted {
                turn_id: identity.turn_id.clone(),
                agent: agent.clone(),
                attempt: *attempt,
                max_attempts: *max_attempts,
                reason: reason.clone(),
            }],
            _ => Vec::new(),
        },
        RuntimeTurnEvent::TurnCompleted { identity, .. } => vec![AgentEvent::TurnDone {
            turn_id: identity.turn_id.clone(),
            agent: agent.clone(),
        }],
        RuntimeTurnEvent::TurnErrored { identity, message } => vec![AgentEvent::Error {
            turn_id: Some(identity.turn_id.clone()),
            agent: agent.clone(),
            code: "agent_error".to_string(),
            message: message.clone(),
        }],
        RuntimeTurnEvent::StorageEvent { event } => storage_event_to_agent_events(event, agent),
        _ => Vec::new(),
    }
}

/// 将 `StorageEvent` 转换为 0..N 个 `AgentEvent`。
pub fn storage_event_to_agent_events(
    event: &StorageEvent,
    fallback_agent: &AgentEventContext,
) -> Vec<AgentEvent> {
    let Some(turn_id) = event.turn_id.clone() else {
        return Vec::new();
    };
    let agent = if event.agent.is_empty() {
        fallback_agent.clone()
    } else {
        event.agent.clone()
    };
    match &event.payload {
        StorageEventPayload::ToolCall {
            tool_call_id,
            tool_name,
            args,
        } => vec![AgentEvent::ToolCallStart {
            turn_id,
            agent,
            tool_call_id: tool_call_id.clone(),
            tool_name: tool_name.clone(),
            input: args.clone(),
        }],
        StorageEventPayload::ToolCallDelta {
            tool_call_id,
            tool_name,
            stream,
            delta,
        } if !delta.is_empty() => vec![AgentEvent::ToolCallDelta {
            turn_id,
            agent,
            tool_call_id: tool_call_id.clone(),
            tool_name: tool_name.clone(),
            stream: *stream,
            delta: delta.clone(),
        }],
        StorageEventPayload::ToolResult {
            tool_call_id,
            tool_name,
            output,
            success,
            error,
            metadata,
            continuation,
            duration_ms,
        } => vec![AgentEvent::ToolCallResult {
            turn_id,
            agent,
            result: ToolExecutionResult {
                tool_call_id: tool_call_id.clone(),
                tool_name: tool_name.clone(),
                ok: *success,
                output: output.clone(),
                error: error.clone(),
                metadata: metadata.clone(),
                continuation: continuation.clone(),
                duration_ms: *duration_ms,
                truncated: false,
            },
        }],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{
        AgentEvent, AgentEventContext, StorageEvent, StorageEventPayload, ToolOutputStream,
    };

    use super::runtime_turn_events_to_agent_events;
    use crate::RuntimeTurnEvent;

    #[test]
    fn storage_tool_delta_maps_to_agent_event() {
        let agent = AgentEventContext::root_execution("agent-root", "default");
        let events = runtime_turn_events_to_agent_events(
            &RuntimeTurnEvent::StorageEvent {
                event: Box::new(StorageEvent {
                    turn_id: Some("turn-1".to_string()),
                    agent: agent.clone(),
                    payload: StorageEventPayload::ToolCallDelta {
                        tool_call_id: "call-1".to_string(),
                        tool_name: "shell_command".to_string(),
                        stream: ToolOutputStream::Stdout,
                        delta: "live\n".to_string(),
                    },
                }),
            },
            &agent,
        );

        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            AgentEvent::ToolCallDelta {
                turn_id,
                tool_call_id,
                tool_name,
                stream,
                delta,
                ..
            } if turn_id == "turn-1"
                && tool_call_id == "call-1"
                && tool_name == "shell_command"
                && *stream == ToolOutputStream::Stdout
                && delta == "live\n"
        ));
    }
}
