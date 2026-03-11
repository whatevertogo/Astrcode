use serde_json::Value;

#[derive(Clone, Debug)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Clone, Debug)]
pub struct ToolCallRequest {
    pub id: String,
    pub name: String,
    pub args: Value,
}

#[derive(Clone, Debug)]
pub struct ToolExecutionResult {
    pub tool_call_id: String,
    pub tool_name: String,
    pub ok: bool,
    pub output: String,
    pub error: Option<String>,
    pub metadata: Option<Value>,
    pub duration_ms: u128,
}

impl ToolExecutionResult {
    pub fn model_content(&self) -> String {
        if self.ok {
            self.output.clone()
        } else {
            format!(
                "tool execution failed: {}\\n{}",
                self.error.as_deref().unwrap_or("unknown error"),
                self.output
            )
        }
    }
}

#[derive(Clone, Debug)]
pub enum LlmMessage {
    User {
        content: String,
    },
    Assistant {
        content: String,
        tool_calls: Vec<ToolCallRequest>,
    },
    Tool {
        tool_call_id: String,
        content: String,
    },
}

#[derive(Clone, Debug)]
pub struct LlmResponse {
    pub content: String,
    pub tool_calls: Vec<ToolCallRequest>,
}
