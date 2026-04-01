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
    pub duration_ms: u64,
    /// Indicates whether the output was truncated due to size limit
    #[serde(default)]
    pub truncated: bool,
}

impl ToolExecutionResult {
    pub fn model_content(&self) -> String {
        if self.ok {
            return self.output.clone();
        }

        match self.error.as_deref() {
            Some(error) if self.output.is_empty() => format!("tool execution failed: {error}"),
            Some(error) => format!("tool execution failed: {error}\n{}", self.output),
            None => self.output.clone(),
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

/// 将 LLM 原始输出文本拆分为「可见内容」和「推理内容」两部分。
///
/// ## 为什么需要这个函数
///
/// 某些 LLM（如 Anthropic Claude）使用 `<think＞...＜/think＞` 标签包裹推理过程。
/// 但 LLM 可能在不同位置以不同方式输出这些标签：
/// - 作为独立的 reasoning_content 字段（由 LLM API 返回）
/// - 内联在文本内容中（某些模型/提供商的输出风格）
///
/// 此函数统一处理这两种情况，提取出推理内容并清理可见文本。
///
/// ## 算法要点
///
/// 1. 用游标扫描全文，查找大小写不敏感的 `<think＞...＜/think＞` 标签对
/// 2. 空的 think 块（`<think＞＜/think＞`）保留原样不动——避免破坏无推理内容时的输出
/// 3. 非空 think 块的内容被提取到 `inline_blocks`，标签从可见文本中移除
/// 4. 移除标签后，连续超过两个空行的位置会被折叠为两个空行（`collapse_extra_blank_lines`），
///    因为标签移除可能留下大片空白
/// 5. 如果同时存在显式 reasoning（API 返回的）和内联 reasoning（从标签提取的），
///    合并两者；内容相同时去重
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
            truncated: false,
        };

        assert_eq!(
            result.model_content(),
            "tool execution failed: boom\ntool output"
        );
    }

    #[test]
    fn model_content_avoids_trailing_newline_for_failed_tools_without_output() {
        let result = ToolExecutionResult {
            tool_call_id: "call-1".to_string(),
            tool_name: "demo".to_string(),
            ok: false,
            output: String::new(),
            error: Some("blocked".to_string()),
            metadata: None,
            duration_ms: 12,
            truncated: false,
        };

        assert_eq!(result.model_content(), "tool execution failed: blocked");
    }

    #[test]
    fn model_content_preserves_legacy_failed_output_without_error_field() {
        let result = ToolExecutionResult {
            tool_call_id: "call-1".to_string(),
            tool_name: "demo".to_string(),
            ok: false,
            output: "tool execution blocked: policy".to_string(),
            error: None,
            metadata: None,
            duration_ms: 12,
            truncated: false,
        };

        assert_eq!(result.model_content(), "tool execution blocked: policy");
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
