use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ToolExecutionResult;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Idle,
    Thinking,
    CallingTool,
    Streaming,
    Interrupted,
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "event", content = "data", rename_all = "camelCase")]
pub enum AgentEvent {
    SessionStarted {
        session_id: String,
    },
    PhaseChanged {
        turn_id: Option<String>,
        phase: Phase,
    },
    ModelDelta {
        turn_id: String,
        delta: String,
    },
    ThinkingDelta {
        turn_id: String,
        delta: String,
    },
    AssistantMessage {
        turn_id: String,
        content: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning_content: Option<String>,
    },
    ToolCallStart {
        turn_id: String,
        tool_call_id: String,
        tool_name: String,
        #[serde(rename = "args")]
        input: Value,
    },
    ToolCallResult {
        turn_id: String,
        result: ToolExecutionResult,
    },
    TurnDone {
        turn_id: String,
    },
    Error {
        turn_id: Option<String>,
        code: String,
        message: String,
    },
}
