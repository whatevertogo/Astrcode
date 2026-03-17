use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCallRequest {
    pub id: String,
    pub name: String,
    pub args: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolExecutionResult {
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

impl ToolExecutionResult {
    pub fn model_content(&self) -> String {
        if self.ok {
            self.output.clone()
        } else {
            format!(
                "tool execution failed: {}\n{}",
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

#[cfg(test)]
mod tests {
    use super::ToolExecutionResult;

    #[test]
    fn model_content_uses_real_newline_for_failed_tools() {
        let result = ToolExecutionResult {
            tool_call_id: "call-1".to_string(),
            tool_name: "demo".to_string(),
            ok: false,
            output: "tool output".to_string(),
            error: Some("boom".to_string()),
            metadata: None,
            duration_ms: 12,
        };

        assert_eq!(
            result.model_content(),
            "tool execution failed: boom\ntool output"
        );
    }
}
