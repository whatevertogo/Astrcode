use crate::{event, AgentEvent};

pub use event::{DeleteProjectResult, SessionMeta};

#[derive(Clone, Debug)]
pub enum SessionMessage {
    User {
        content: String,
        timestamp: String,
    },
    Assistant {
        content: String,
        timestamp: String,
        reasoning_content: Option<String>,
    },
    ToolCall {
        tool_call_id: String,
        tool_name: String,
        args: serde_json::Value,
        output: Option<String>,
        ok: Option<bool>,
        duration_ms: Option<u64>,
    },
}

#[derive(Clone, Debug)]
pub struct SessionEventRecord {
    pub event_id: String,
    pub event: AgentEvent,
}
