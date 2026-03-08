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
        content: String,
        timestamp: DateTime<Utc>,
    },
    AssistantDelta {
        token: String,
    },
    AssistantFinal {
        content: String,
    },
    ToolCall {
        tool_call_id: String,
        tool_name: String,
        args: Value,
    },
    ToolResult {
        tool_call_id: String,
        output: String,
        success: bool,
        duration_ms: u64,
    },
    TurnDone {
        timestamp: DateTime<Utc>,
    },
    Error {
        message: String,
    },
}
