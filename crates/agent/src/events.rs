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
