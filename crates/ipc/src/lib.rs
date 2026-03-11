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
    #[serde(alias = "tool_call_id")]
    pub tool_call_id: String,
    #[serde(alias = "tool_name")]
    pub tool_name: String,
    pub ok: bool,
    pub output: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    #[serde(alias = "duration_ms")]
    pub duration_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "event", content = "data", rename_all = "camelCase")]
pub enum AgentEventKind {
    SessionStarted {
        #[serde(rename = "sessionId", alias = "session_id")]
        session_id: String,
    },
    PhaseChanged {
        #[serde(rename = "turnId", alias = "turn_id")]
        turn_id: Option<String>,
        phase: Phase,
    },
    ModelDelta {
        #[serde(rename = "turnId", alias = "turn_id")]
        turn_id: String,
        delta: String,
    },
    ThinkingDelta {
        #[serde(rename = "turnId", alias = "turn_id")]
        turn_id: String,
        delta: String,
    },
    AssistantMessage {
        #[serde(rename = "turnId", alias = "turn_id")]
        turn_id: String,
        content: String,
    },
    ToolCallStart {
        #[serde(rename = "turnId", alias = "turn_id")]
        turn_id: String,
        #[serde(rename = "toolCallId", alias = "tool_call_id")]
        tool_call_id: String,
        #[serde(rename = "toolName", alias = "tool_name")]
        tool_name: String,
        args: Value,
    },
    ToolCallResult {
        #[serde(rename = "turnId", alias = "turn_id")]
        turn_id: String,
        result: ToolCallResultEnvelope,
    },
    TurnDone {
        #[serde(rename = "turnId", alias = "turn_id")]
        turn_id: String,
    },
    Error {
        #[serde(rename = "turnId", alias = "turn_id")]
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
    SubmitPrompt { text: String },
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn tool_call_start_serializes_camel_case_fields() {
        let event = AgentEvent::new(AgentEventKind::ToolCallStart {
            turn_id: "turn-1".to_string(),
            tool_call_id: "call-123".to_string(),
            tool_name: "listDir".to_string(),
            args: json!({ "path": "." }),
        });

        let payload = serde_json::to_value(event).expect("event should serialize");
        let data = payload
            .get("data")
            .expect("toolCallStart should contain data");

        assert_eq!(
            payload.get("event"),
            Some(&Value::String("toolCallStart".to_string()))
        );
        assert_eq!(
            data.get("turnId"),
            Some(&Value::String("turn-1".to_string()))
        );
        assert_eq!(
            data.get("toolCallId"),
            Some(&Value::String("call-123".to_string()))
        );
        assert_eq!(
            data.get("toolName"),
            Some(&Value::String("listDir".to_string()))
        );
        assert!(data.get("turn_id").is_none());
        assert!(data.get("tool_call_id").is_none());
        assert!(data.get("tool_name").is_none());
    }

    #[test]
    fn tool_call_result_serializes_camel_case_envelope() {
        let event = AgentEvent::new(AgentEventKind::ToolCallResult {
            turn_id: "turn-2".to_string(),
            result: ToolCallResultEnvelope {
                tool_call_id: "call-456".to_string(),
                tool_name: "readFile".to_string(),
                ok: true,
                output: "ok".to_string(),
                error: None,
                metadata: None,
                duration_ms: 12,
            },
        });

        let payload = serde_json::to_value(event).expect("event should serialize");
        let data = payload
            .get("data")
            .expect("toolCallResult should contain data");
        let result = data
            .get("result")
            .expect("toolCallResult should contain result");

        assert_eq!(
            data.get("turnId"),
            Some(&Value::String("turn-2".to_string()))
        );
        assert_eq!(
            result.get("toolCallId"),
            Some(&Value::String("call-456".to_string()))
        );
        assert_eq!(
            result.get("toolName"),
            Some(&Value::String("readFile".to_string()))
        );
        assert_eq!(result.get("durationMs"), Some(&Value::Number(12u64.into())));
        assert!(result.get("tool_call_id").is_none());
        assert!(result.get("tool_name").is_none());
        assert!(result.get("duration_ms").is_none());
    }

    #[test]
    fn tool_call_start_deserializes_legacy_snake_case_fields() {
        let raw = json!({
            "protocolVersion": 1,
            "event": "toolCallStart",
            "data": {
                "turn_id": "turn-legacy",
                "tool_call_id": "legacy-call",
                "tool_name": "listDir",
                "args": { "path": "." }
            }
        });

        let parsed: AgentEvent =
            serde_json::from_value(raw).expect("legacy payload should deserialize");
        match parsed.kind {
            AgentEventKind::ToolCallStart {
                turn_id,
                tool_call_id,
                tool_name,
                ..
            } => {
                assert_eq!(turn_id, "turn-legacy");
                assert_eq!(tool_call_id, "legacy-call");
                assert_eq!(tool_name, "listDir");
            }
            other => panic!("expected ToolCallStart, got {other:?}"),
        }
    }

    #[test]
    fn thinking_delta_serializes_camel_case_fields() {
        let event = AgentEvent::new(AgentEventKind::ThinkingDelta {
            turn_id: "turn-1".to_string(),
            delta: "pondering".to_string(),
        });

        let payload = serde_json::to_value(event).expect("event should serialize");
        let data = payload.get("data").expect("thinkingDelta should contain data");

        assert_eq!(
            payload.get("event"),
            Some(&Value::String("thinkingDelta".to_string()))
        );
        assert_eq!(
            data.get("turnId"),
            Some(&Value::String("turn-1".to_string()))
        );
        assert_eq!(
            data.get("delta"),
            Some(&Value::String("pondering".to_string()))
        );
        assert!(data.get("turn_id").is_none());
    }
}
