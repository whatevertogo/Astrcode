use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum StorageEvent {
    SessionStart {
        session_id: String,
        timestamp: DateTime<Utc>,
        working_dir: String,
    },
    UserMessage {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        content: String,
        timestamp: DateTime<Utc>,
    },
    AssistantDelta {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        token: String,
    },
    ThinkingDelta {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        token: String,
    },
    AssistantFinal {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        content: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning_content: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning_signature: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timestamp: Option<DateTime<Utc>>,
    },
    ToolCall {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        tool_call_id: String,
        tool_name: String,
        args: Value,
    },
    ToolResult {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        tool_call_id: String,
        #[serde(default)]
        tool_name: String,
        output: String,
        success: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        metadata: Option<Value>,
        duration_ms: u64,
    },
    TurnDone {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        timestamp: DateTime<Utc>,
    },
    Error {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timestamp: Option<DateTime<Utc>>,
    },
}

impl StorageEvent {
    pub fn turn_id(&self) -> Option<&str> {
        match self {
            Self::UserMessage { turn_id, .. }
            | Self::AssistantDelta { turn_id, .. }
            | Self::ThinkingDelta { turn_id, .. }
            | Self::AssistantFinal { turn_id, .. }
            | Self::ToolCall { turn_id, .. }
            | Self::ToolResult { turn_id, .. }
            | Self::TurnDone { turn_id, .. }
            | Self::Error { turn_id, .. } => turn_id.as_deref(),
            Self::SessionStart { .. } => None,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StoredEvent {
    pub storage_seq: u64,
    #[serde(flatten)]
    pub event: StorageEvent,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum StoredEventLine {
    Stored(StoredEvent),
    Legacy(StorageEvent),
}

impl StoredEventLine {
    pub fn into_stored(self, fallback_seq: u64) -> StoredEvent {
        match self {
            Self::Stored(stored) => stored,
            Self::Legacy(event) => StoredEvent {
                storage_seq: fallback_seq,
                event,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::StorageEvent;

    #[test]
    fn tool_result_deserializes_legacy_lines_without_error_or_metadata() {
        let event: StorageEvent = serde_json::from_str(
            r#"{"type":"toolResult","turn_id":"turn-1","tool_call_id":"call-1","tool_name":"readFile","output":"hello","success":true,"duration_ms":12}"#,
        )
        .expect("legacy tool result should deserialize");

        match event {
            StorageEvent::ToolResult {
                error, metadata, ..
            } => {
                assert_eq!(error, None);
                assert_eq!(metadata, None);
            }
            other => panic!("expected tool result, got {other:?}"),
        }
    }
}
