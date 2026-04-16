use astrcode_core::{ReasoningContent, ToolCallRequest};
use log::{debug, warn};
use serde_json::Value;

use super::dto::{AnthropicResponse, AnthropicUsage};
use crate::{FinishReason, LlmOutput};

/// 将 Anthropic 非流式响应转换为统一的 `LlmOutput`。
///
/// 遍历内容块数组，根据块类型分派：
/// - `text`: 拼接到输出内容
/// - `tool_use`: 提取 id、name、input 构造工具调用请求
/// - `thinking`: 提取推理内容和签名
/// - 未知类型：记录警告并跳过
///
/// TODO:更好的办法？
/// `stop_reason` 映射到统一的 `FinishReason` (P4.2):
/// - `end_turn` → Stop
/// - `max_tokens` → MaxTokens
/// - `tool_use` → ToolCalls
/// - `stop_sequence` → Stop
pub(super) fn response_to_output(response: AnthropicResponse) -> LlmOutput {
    let usage = response.usage.and_then(AnthropicUsage::into_llm_usage);

    // 记录缓存状态
    if let Some(ref u) = usage {
        let input = u.input_tokens;
        let cache_read = u.cache_read_input_tokens;
        let cache_creation = u.cache_creation_input_tokens;
        let total_prompt_tokens = input.saturating_add(cache_read);

        if cache_read == 0 && cache_creation > 0 {
            debug!(
                "Cache miss: writing {} tokens to cache (total prompt: {}, uncached input: {})",
                cache_creation, total_prompt_tokens, input
            );
        } else if cache_read > 0 {
            let hit_rate = (cache_read as f32 / total_prompt_tokens as f32) * 100.0;
            debug!(
                "Cache hit: {:.1}% ({} / {} prompt tokens, creation: {}, uncached input: {})",
                hit_rate, cache_read, total_prompt_tokens, cache_creation, input
            );
        } else {
            debug!(
                "Cache disabled or unavailable (total prompt: {} tokens)",
                total_prompt_tokens
            );
        }
    }

    let mut content = String::new();
    let mut tool_calls = Vec::new();
    let mut reasoning = None;

    for block in response.content {
        match block_type(&block) {
            Some("text") => {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    content.push_str(text);
                }
            },
            Some("tool_use") => {
                let id = match block.get("id").and_then(Value::as_str) {
                    Some(id) if !id.is_empty() => id.to_string(),
                    _ => {
                        warn!("anthropic: tool_use block missing non-empty id, skipping");
                        continue;
                    },
                };
                let name = match block.get("name").and_then(Value::as_str) {
                    Some(name) if !name.is_empty() => name.to_string(),
                    _ => {
                        warn!("anthropic: tool_use block missing non-empty name, skipping");
                        continue;
                    },
                };
                let args = block.get("input").cloned().unwrap_or(Value::Null);
                tool_calls.push(ToolCallRequest { id, name, args });
            },
            Some("thinking") => {
                if let Some(thinking) = block.get("thinking").and_then(Value::as_str) {
                    reasoning = Some(ReasoningContent {
                        content: thinking.to_string(),
                        signature: block
                            .get("signature")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    });
                }
            },
            Some(other) => {
                warn!("anthropic: unknown content block type: {}", other);
            },
            None => {
                warn!("anthropic: content block missing type");
            },
        }
    }

    // Anthropic stop_reason 映射到统一 FinishReason
    let finish_reason = response
        .stop_reason
        .as_deref()
        .map(|reason| match reason {
            "end_turn" | "stop_sequence" => FinishReason::Stop,
            "max_tokens" => FinishReason::MaxTokens,
            "tool_use" => FinishReason::ToolCalls,
            other => FinishReason::Other(other.to_string()),
        })
        .unwrap_or_else(|| {
            if !tool_calls.is_empty() {
                FinishReason::ToolCalls
            } else {
                FinishReason::Stop
            }
        });

    LlmOutput {
        content,
        tool_calls,
        reasoning,
        usage,
        finish_reason,
    }
}

/// 从 JSON Value 中提取内容块的类型字段。
fn block_type(value: &Value) -> Option<&str> {
    value.get("type").and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use astrcode_core::ReasoningContent;
    use serde_json::json;

    use super::response_to_output;
    use crate::{
        LlmUsage,
        anthropic::dto::{AnthropicCacheCreationUsage, AnthropicResponse, AnthropicUsage},
    };

    #[test]
    fn response_to_output_parses_text_tool_use_and_thinking() {
        let output = response_to_output(AnthropicResponse {
            content: vec![
                json!({ "type": "text", "text": "hello " }),
                json!({
                    "type": "tool_use",
                    "id": "call_1",
                    "name": "search",
                    "input": { "q": "rust" }
                }),
                json!({ "type": "text", "text": "world" }),
                json!({ "type": "thinking", "thinking": "pondering", "signature": "sig-1" }),
            ],
            stop_reason: Some("tool_use".to_string()),
            usage: None,
        });

        assert_eq!(output.content, "hello world");
        assert_eq!(output.tool_calls.len(), 1);
        assert_eq!(output.tool_calls[0].id, "call_1");
        assert_eq!(output.tool_calls[0].args, json!({ "q": "rust" }));
        assert_eq!(
            output.reasoning,
            Some(ReasoningContent {
                content: "pondering".to_string(),
                signature: Some("sig-1".to_string()),
            })
        );
    }

    #[test]
    fn response_to_output_parses_cache_usage_fields() {
        let output = response_to_output(AnthropicResponse {
            content: vec![json!({ "type": "text", "text": "ok" })],
            stop_reason: Some("end_turn".to_string()),
            usage: Some(AnthropicUsage {
                input_tokens: Some(100),
                output_tokens: Some(20),
                cache_creation_input_tokens: Some(80),
                cache_read_input_tokens: Some(60),
                cache_creation: None,
            }),
        });

        assert_eq!(
            output.usage,
            Some(LlmUsage {
                input_tokens: 100,
                output_tokens: 20,
                cache_creation_input_tokens: 80,
                cache_read_input_tokens: 60,
            })
        );
    }

    #[test]
    fn response_to_output_parses_nested_cache_creation_usage_fields() {
        let output = response_to_output(AnthropicResponse {
            content: vec![json!({ "type": "text", "text": "ok" })],
            stop_reason: Some("end_turn".to_string()),
            usage: Some(AnthropicUsage {
                input_tokens: Some(100),
                output_tokens: Some(20),
                cache_creation_input_tokens: None,
                cache_read_input_tokens: Some(60),
                cache_creation: Some(AnthropicCacheCreationUsage {
                    ephemeral_5m_input_tokens: Some(30),
                    ephemeral_1h_input_tokens: Some(50),
                }),
            }),
        });

        assert_eq!(
            output.usage,
            Some(LlmUsage {
                input_tokens: 100,
                output_tokens: 20,
                cache_creation_input_tokens: 80,
                cache_read_input_tokens: 60,
            })
        );
    }
}
