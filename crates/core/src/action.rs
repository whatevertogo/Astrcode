use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningContent {
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

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
        reasoning: Option<ReasoningContent>,
    },
    Tool {
        tool_call_id: String,
        content: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssistantContentParts {
    pub visible_content: String,
    pub reasoning_content: Option<String>,
}

pub fn split_assistant_content(
    content: &str,
    explicit_reasoning: Option<&str>,
) -> AssistantContentParts {
    let mut visible_content = String::new();
    let mut inline_blocks = Vec::new();
    let lower = content.to_ascii_lowercase();
    let mut cursor = 0usize;
    let mut removed_tags = false;

    while let Some(start_rel) = lower[cursor..].find("<think>") {
        let start = cursor + start_rel;
        let block_start = start + "<think>".len();
        let Some(end_rel) = lower[block_start..].find("</think>") else {
            break;
        };
        let end = block_start + end_rel;
        let raw_inner = &content[block_start..end];
        let normalized = raw_inner.trim();

        if normalized.is_empty() {
            visible_content.push_str(&content[cursor..end + "</think>".len()]);
            cursor = end + "</think>".len();
            continue;
        }

        visible_content.push_str(&content[cursor..start]);
        inline_blocks.push(normalized.to_string());
        cursor = end + "</think>".len();
        removed_tags = true;
    }

    visible_content.push_str(&content[cursor..]);

    let explicit_reasoning = explicit_reasoning
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let inline_reasoning = (!inline_blocks.is_empty()).then(|| inline_blocks.join("\n\n"));
    let reasoning_content = match (explicit_reasoning, inline_reasoning) {
        (Some(explicit), Some(inline)) if explicit == inline => Some(explicit),
        (Some(explicit), Some(inline)) => Some(format!("{explicit}\n\n{inline}")),
        (Some(explicit), None) => Some(explicit),
        (None, Some(inline)) => Some(inline),
        (None, None) => None,
    };

    let visible_content = if removed_tags {
        collapse_extra_blank_lines(visible_content.trim())
            .trim()
            .to_string()
    } else {
        content.to_string()
    };

    AssistantContentParts {
        visible_content,
        reasoning_content,
    }
}

fn collapse_extra_blank_lines(input: &str) -> String {
    let mut collapsed = String::with_capacity(input.len());
    let mut newline_run = 0usize;

    for ch in input.chars() {
        if ch == '\n' {
            newline_run += 1;
            if newline_run <= 2 {
                collapsed.push(ch);
            }
            continue;
        }

        newline_run = 0;
        collapsed.push(ch);
    }

    collapsed
}

#[derive(Clone, Debug)]
pub struct LlmResponse {
    pub content: String,
    pub tool_calls: Vec<ToolCallRequest>,
}

#[cfg(test)]
mod tests {
    use super::{split_assistant_content, ToolExecutionResult};

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

    #[test]
    fn split_assistant_content_extracts_inline_thinking_blocks() {
        let parts = split_assistant_content(
            "Answer before\n<think> first step </think>\n<think>second step</think>\nAnswer after",
            None,
        );

        assert_eq!(parts.visible_content, "Answer before\n\nAnswer after");
        assert_eq!(
            parts.reasoning_content.as_deref(),
            Some("first step\n\nsecond step")
        );
    }

    #[test]
    fn split_assistant_content_prefers_explicit_reasoning_and_strips_legacy_tags() {
        let parts = split_assistant_content(
            "<think>legacy</think>\nvisible",
            Some("persisted reasoning"),
        );

        assert_eq!(parts.visible_content, "visible");
        assert_eq!(
            parts.reasoning_content.as_deref(),
            Some("persisted reasoning\n\nlegacy")
        );
    }

    #[test]
    fn split_assistant_content_keeps_empty_think_blocks_literal() {
        let parts = split_assistant_content("<think>   </think>\n\nvisible", None);

        assert_eq!(parts.visible_content, "<think>   </think>\n\nvisible");
        assert_eq!(parts.reasoning_content, None);
    }
}
