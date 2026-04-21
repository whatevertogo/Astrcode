use serde::Serialize;
use serde_json::Value;

use crate::LlmUsage;

pub(crate) fn cacheable_text(text: &str) -> bool {
    !text.is_empty()
}

/// Anthropic Messages API 请求体。
///
/// 注意：`stream` 字段为 `Option<bool>`，`None` 时表示非流式模式，
/// 这样可以在序列化时省略该字段（Anthropic API 默认非流式）。
#[derive(Debug, Clone, Serialize)]
pub(crate) struct AnthropicRequest {
    pub(crate) model: String,
    pub(crate) max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cache_control: Option<AnthropicCacheControl>,
    pub(crate) messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) system: Option<AnthropicSystemPrompt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tools: Option<Vec<AnthropicTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) thinking: Option<AnthropicThinking>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub(crate) enum AnthropicSystemPrompt {
    Text(String),
    Blocks(Vec<AnthropicSystemBlock>),
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AnthropicSystemBlock {
    #[serde(rename = "type")]
    pub(crate) type_: String,
    pub(crate) text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cache_control: Option<AnthropicCacheControl>,
}

/// Anthropic extended thinking 配置。
///
/// `budget_tokens` 指定推理过程可使用的最大 token 数，
/// 不计入最终输出的 `max_tokens` 限制。
///
/// ## 设计动机
///
/// Extended thinking 让 Claude 在输出前进行深度推理，提升复杂任务的回答质量。
/// 预算设为 75% 是为了保留至少 25% 的 token 给实际输出内容。
#[derive(Debug, Clone, Serialize)]
pub(crate) struct AnthropicThinking {
    #[serde(rename = "type")]
    pub(crate) type_: String,
    pub(crate) budget_tokens: u32,
}

/// Anthropic 消息（包含角色和内容块数组）。
///
/// Anthropic 的消息结构与 OpenAI 不同：`content` 是内容块数组而非纯文本，
/// 这使得单条消息可以混合文本、推理、工具调用等多种内容类型。
#[derive(Debug, Clone, Serialize)]
pub(crate) struct AnthropicMessage {
    pub(crate) role: String,
    pub(crate) content: Vec<AnthropicContentBlock>,
}

/// Anthropic 内容块——消息内容由多个块组成。
///
/// 使用 `#[serde(tag = "type")]` 实现内部标记序列化，
/// 每个变体对应一个 `type` 值（`text`、`thinking`、`tool_use`、`tool_result`）。
///
/// ## 缓存控制
///
/// 每个块可选携带 `cache_control` 字段，标记为 `ephemeral` 类型时，
/// Anthropic 后端会将该块作为缓存前缀的一部分，用于 KV cache 复用。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum AnthropicContentBlock {
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<AnthropicCacheControl>,
    },
    Thinking {
        thinking: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<AnthropicCacheControl>,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<AnthropicCacheControl>,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_reference: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<AnthropicCacheControl>,
    },
}

/// Anthropic prompt caching 控制标记。
///
/// `type: "ephemeral"` 告诉 Anthropic 后端该块可作为缓存前缀的一部分。
/// 缓存是临时的（ephemeral），不保证长期有效，但在短时间内重复请求可以显著减少延迟。
#[derive(Debug, Clone, Serialize)]
pub(crate) struct AnthropicCacheControl {
    #[serde(rename = "type")]
    type_: String,
}

impl AnthropicCacheControl {
    /// 创建 ephemeral 类型的缓存控制标记。
    pub(crate) fn ephemeral() -> Self {
        Self {
            type_: "ephemeral".to_string(),
        }
    }
}

impl AnthropicContentBlock {
    pub(crate) fn block_type(&self) -> &'static str {
        match self {
            AnthropicContentBlock::Text { .. } => "text",
            AnthropicContentBlock::Thinking { .. } => "thinking",
            AnthropicContentBlock::ToolUse { .. } => "tool_use",
            AnthropicContentBlock::ToolResult { .. } => "tool_result",
        }
    }

    pub(crate) fn has_cache_control(&self) -> bool {
        match self {
            AnthropicContentBlock::Text { cache_control, .. }
            | AnthropicContentBlock::Thinking { cache_control, .. }
            | AnthropicContentBlock::ToolUse { cache_control, .. }
            | AnthropicContentBlock::ToolResult { cache_control, .. } => cache_control.is_some(),
        }
    }

    /// 判断内容块是否适合显式 `cache_control`。
    pub(crate) fn can_use_explicit_cache_control(&self) -> bool {
        match self {
            AnthropicContentBlock::Text { text, .. } => cacheable_text(text),
            AnthropicContentBlock::Thinking { thinking, .. } => cacheable_text(thinking),
            AnthropicContentBlock::ToolUse { id, name, .. } => {
                cacheable_text(id) && cacheable_text(name)
            },
            AnthropicContentBlock::ToolResult { tool_use_id, .. } => cacheable_text(tool_use_id),
        }
    }

    /// 为允许显式缓存的内容块设置或清除 `cache_control` 标记。
    pub(crate) fn set_cache_control_if_allowed(&mut self, enabled: bool) -> bool {
        if enabled && !self.can_use_explicit_cache_control() {
            return false;
        }

        let control = if enabled {
            Some(AnthropicCacheControl::ephemeral())
        } else {
            None
        };
        match self {
            AnthropicContentBlock::Text { cache_control, .. }
            | AnthropicContentBlock::Thinking { cache_control, .. }
            | AnthropicContentBlock::ToolUse { cache_control, .. }
            | AnthropicContentBlock::ToolResult { cache_control, .. } => *cache_control = control,
        }
        true
    }

    pub(crate) fn set_cache_reference_to_tool_use_id(&mut self) -> bool {
        let AnthropicContentBlock::ToolResult {
            tool_use_id,
            cache_reference,
            ..
        } = self
        else {
            return false;
        };

        *cache_reference = Some(tool_use_id.clone());
        true
    }
}

/// Anthropic 工具定义。
///
/// 与 OpenAI 不同，Anthropic 工具定义不需要 `type` 字段，
/// 直接使用 `name`、`description`、`input_schema` 三个字段。
#[derive(Debug, Clone, Serialize)]
pub(crate) struct AnthropicTool {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) input_schema: Value,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cache_control: Option<AnthropicCacheControl>,
}

/// Anthropic Messages API 非流式响应体。
///
/// NOTE: `content` 使用 `Vec<Value>` 而非强类型结构体，
/// 因为 Anthropic 响应可能包含多种内容块类型（text / tool_use / thinking），
/// 使用 `Value` 可以灵活处理未知或新增的块类型，避免每次 API 更新都要修改 DTO。
#[derive(Debug, serde::Deserialize)]
pub(super) struct AnthropicResponse {
    pub(super) content: Vec<Value>,
    #[allow(dead_code)]
    pub(super) stop_reason: Option<String>,
    #[serde(default)]
    pub(super) usage: Option<AnthropicUsage>,
}

/// Anthropic 响应中的 token 用量统计。
///
/// 两个字段均为 `Option` 且带 `#[serde(default)]`，
/// 因为某些旧版 API 或特殊响应可能不包含用量信息。
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub(super) struct AnthropicUsage {
    #[serde(default)]
    pub(super) input_tokens: Option<u64>,
    #[serde(default)]
    pub(super) output_tokens: Option<u64>,
    #[serde(default)]
    pub(super) cache_creation_input_tokens: Option<u64>,
    #[serde(default)]
    pub(super) cache_read_input_tokens: Option<u64>,
    #[serde(default)]
    pub(super) cache_creation: Option<AnthropicCacheCreationUsage>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub(super) struct AnthropicCacheCreationUsage {
    #[serde(default)]
    pub(super) ephemeral_5m_input_tokens: Option<u64>,
    #[serde(default)]
    pub(super) ephemeral_1h_input_tokens: Option<u64>,
}

impl AnthropicUsage {
    pub(super) fn merge_from(&mut self, other: Self) {
        self.input_tokens = other.input_tokens.or(self.input_tokens);
        self.cache_creation_input_tokens = other
            .cache_creation_input_tokens
            .or(self.cache_creation_input_tokens);
        self.cache_read_input_tokens = other
            .cache_read_input_tokens
            .or(self.cache_read_input_tokens);
        self.cache_creation = other.cache_creation.or_else(|| self.cache_creation.take());
        // output_tokens 在流式事件里通常是累计值，优先保留最新的非空值。
        self.output_tokens = other.output_tokens.or(self.output_tokens);
    }

    pub(super) fn into_llm_usage(self) -> Option<LlmUsage> {
        let cache_creation_input_tokens = self.cache_creation_input_tokens.or_else(|| {
            self.cache_creation
                .as_ref()
                .map(AnthropicCacheCreationUsage::total_input_tokens)
        });

        if self.input_tokens.is_none()
            && self.output_tokens.is_none()
            && cache_creation_input_tokens.is_none()
            && self.cache_read_input_tokens.is_none()
        {
            return None;
        }

        Some(LlmUsage {
            input_tokens: self.input_tokens.unwrap_or_default() as usize,
            output_tokens: self.output_tokens.unwrap_or_default() as usize,
            cache_creation_input_tokens: cache_creation_input_tokens.unwrap_or_default() as usize,
            cache_read_input_tokens: self.cache_read_input_tokens.unwrap_or_default() as usize,
        })
    }
}

impl AnthropicCacheCreationUsage {
    fn total_input_tokens(&self) -> u64 {
        self.ephemeral_5m_input_tokens
            .unwrap_or_default()
            .saturating_add(self.ephemeral_1h_input_tokens.unwrap_or_default())
    }
}

#[derive(Debug, Default)]
pub(super) struct SseProcessResult {
    pub(super) done: bool,
    pub(super) stop_reason: Option<String>,
    pub(super) usage: Option<AnthropicUsage>,
}

pub(super) fn extract_usage_from_payload(
    event_type: &str,
    payload: &Value,
) -> Option<AnthropicUsage> {
    match event_type {
        "message_start" => payload
            .get("message")
            .and_then(|message| message.get("usage"))
            .and_then(parse_usage_value),
        "message_delta" => payload
            .get("usage")
            .or_else(|| payload.get("delta").and_then(|delta| delta.get("usage")))
            .and_then(parse_usage_value),
        _ => None,
    }
}

fn parse_usage_value(value: &Value) -> Option<AnthropicUsage> {
    serde_json::from_value::<AnthropicUsage>(value.clone()).ok()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{AnthropicCacheControl, AnthropicContentBlock};

    #[test]
    fn clearing_cache_control_reports_success_for_non_text_blocks() {
        let mut block = AnthropicContentBlock::Thinking {
            thinking: "reasoning".to_string(),
            signature: None,
            cache_control: Some(AnthropicCacheControl::ephemeral()),
        };

        assert!(block.set_cache_control_if_allowed(false));
        assert!(!block.has_cache_control());
    }

    #[test]
    fn enabling_cache_control_supports_tool_use_blocks() {
        let mut block = AnthropicContentBlock::ToolUse {
            id: "call_1".to_string(),
            name: "search".to_string(),
            input: json!({ "q": "rust" }),
            cache_control: None,
        };

        assert!(block.set_cache_control_if_allowed(true));
        assert!(block.has_cache_control());
    }
}
