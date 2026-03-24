use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PhaseDto {
    Idle,
    Thinking,
    CallingTool,
    Streaming,
    Interrupted,
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallResultDto {
    pub tool_call_id: String,
    pub tool_name: String,
    pub ok: bool,
    pub output: String,
    pub error: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub duration_ms: u128,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "event", content = "data", rename_all = "camelCase")]
pub enum AgentEventPayload {
    SessionStarted {
        session_id: String,
    },
    PhaseChanged {
        turn_id: Option<String>,
        phase: PhaseDto,
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
        input: serde_json::Value,
    },
    ToolCallResult {
        turn_id: String,
        result: ToolCallResultDto,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentEventEnvelope {
    pub protocol_version: u32,
    #[serde(flatten)]
    pub event: AgentEventPayload,
}
