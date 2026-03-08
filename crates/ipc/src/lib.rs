use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
#[serde(rename_all = "camelCase")]
pub struct ToolCallResultEnvelope {
    pub tool_call_id: String,
    pub tool_name: String,
    pub ok: bool,
    pub output: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    pub duration_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "event", content = "data", rename_all = "camelCase")]
pub enum AgentEventKind {
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
    ToolCallStart {
        turn_id: String,
        tool_call_id: String,
        tool_name: String,
        args: Value,
    },
    ToolCallResult {
        turn_id: String,
        result: ToolCallResultEnvelope,
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
pub struct AgentEvent {
    pub protocol_version: u32,
    #[serde(flatten)]
    pub kind: AgentEventKind,
}

impl AgentEvent {
    pub fn new(kind: AgentEventKind) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            kind,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "command", content = "data", rename_all = "camelCase")]
pub enum TuiCommandKind {
    SubmitPrompt {
        text: String,
    },
    Interrupt,
    Exit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TuiCommand {
    pub protocol_version: u32,
    #[serde(flatten)]
    pub kind: TuiCommandKind,
}

impl TuiCommand {
    pub fn new(kind: TuiCommandKind) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            kind,
        }
    }
}
