use astrcode_core::{LlmMessage, SystemPromptBlock, SystemPromptLayer, ToolDefinition};
use serde_json::{Value, json};

use super::dto::{
    AnthropicCacheControl, AnthropicContentBlock, AnthropicMessage, AnthropicRequest,
    AnthropicSystemBlock, AnthropicSystemPrompt, AnthropicThinking, AnthropicTool, cacheable_text,
};

pub(super) const ANTHROPIC_CACHE_BREAKPOINT_LIMIT: usize = 4;

/// 将 `LlmMessage` 转换为 Anthropic 格式的消息结构。
///
/// Anthropic 使用内容块数组（而非纯文本），因此需要按消息类型分派：
/// - User 消息 → 单个 `text` 内容块
/// - Assistant 消息 → 可能包含 `thinking`、`text`、`tool_use` 多个块
/// - Tool 消息 → 单个 `tool_result` 内容块
#[derive(Clone, Copy)]
pub(super) struct MessageBuildOptions {
    pub(super) include_reasoning_blocks: bool,
}

pub(super) fn summarize_request_for_diagnostics(request: &AnthropicRequest) -> Value {
    let messages = request
        .messages
        .iter()
        .map(|message| {
            let block_types = message
                .content
                .iter()
                .map(AnthropicContentBlock::block_type)
                .collect::<Vec<_>>();
            json!({
                "role": message.role,
                "blockTypes": block_types,
                "blockCount": message.content.len(),
                "cacheControlCount": message
                    .content
                    .iter()
                    .filter(|block| block.has_cache_control())
                    .count(),
            })
        })
        .collect::<Vec<_>>();
    let system = match &request.system {
        None => Value::Null,
        Some(AnthropicSystemPrompt::Text(text)) => json!({
            "kind": "text",
            "chars": text.chars().count(),
        }),
        Some(AnthropicSystemPrompt::Blocks(blocks)) => json!({
            "kind": "blocks",
            "count": blocks.len(),
            "cacheControlCount": blocks
                .iter()
                .filter(|block| block.cache_control.is_some())
                .count(),
            "chars": blocks.iter().map(|block| block.text.chars().count()).sum::<usize>(),
        }),
    };
    let tools = request.tools.as_ref().map(|tools| {
        json!({
            "count": tools.len(),
            "names": tools.iter().map(|tool| tool.name.clone()).collect::<Vec<_>>(),
            "cacheControlCount": tools
                .iter()
                .filter(|tool| tool.cache_control.is_some())
                .count(),
        })
    });

    json!({
        "model": request.model,
        "maxTokens": request.max_tokens,
        "topLevelCacheControl": request.cache_control.is_some(),
        "hasThinking": request.thinking.is_some(),
        "stream": request.stream.unwrap_or(false),
        "system": system,
        "messages": messages,
        "tools": tools,
    })
}

pub(super) fn to_anthropic_messages(
    messages: &[LlmMessage],
    options: MessageBuildOptions,
) -> Vec<AnthropicMessage> {
    let mut anthropic_messages = Vec::with_capacity(messages.len());
    let mut pending_user_blocks = Vec::new();

    let flush_pending_user_blocks =
        |anthropic_messages: &mut Vec<AnthropicMessage>,
         pending_user_blocks: &mut Vec<AnthropicContentBlock>| {
            if pending_user_blocks.is_empty() {
                return;
            }

            anthropic_messages.push(AnthropicMessage {
                role: "user".to_string(),
                content: std::mem::take(pending_user_blocks),
            });
        };

    for message in messages {
        match message {
            LlmMessage::User { content, .. } => {
                pending_user_blocks.push(AnthropicContentBlock::Text {
                    text: content.clone(),
                    cache_control: None,
                });
            },
            LlmMessage::Assistant {
                content,
                tool_calls,
                reasoning,
            } => {
                flush_pending_user_blocks(&mut anthropic_messages, &mut pending_user_blocks);

                let mut blocks = Vec::new();
                if options.include_reasoning_blocks {
                    if let Some(reasoning) = reasoning {
                        blocks.push(AnthropicContentBlock::Thinking {
                            thinking: reasoning.content.clone(),
                            signature: reasoning.signature.clone(),
                            cache_control: None,
                        });
                    }
                }
                // Anthropic assistant 消息可以直接包含 tool_use 块，不要求前置 text 块。
                // 仅在确实有文本时写入 text 块，避免向兼容网关发送空 text 导致参数校验失败。
                if !content.is_empty() {
                    blocks.push(AnthropicContentBlock::Text {
                        text: content.clone(),
                        cache_control: None,
                    });
                }
                blocks.extend(
                    tool_calls
                        .iter()
                        .map(|call| AnthropicContentBlock::ToolUse {
                            id: call.id.clone(),
                            name: call.name.clone(),
                            input: call.args.clone(),
                            cache_control: None,
                        }),
                );
                if blocks.is_empty() {
                    blocks.push(AnthropicContentBlock::Text {
                        text: String::new(),
                        cache_control: None,
                    });
                }

                anthropic_messages.push(AnthropicMessage {
                    role: "assistant".to_string(),
                    content: blocks,
                });
            },
            LlmMessage::Tool {
                tool_call_id,
                content,
            } => {
                pending_user_blocks.push(AnthropicContentBlock::ToolResult {
                    tool_use_id: tool_call_id.clone(),
                    content: content.clone(),
                    cache_control: None,
                });
            },
        }
    }

    flush_pending_user_blocks(&mut anthropic_messages, &mut pending_user_blocks);
    anthropic_messages
}

/// 在最近的消息内容块上启用显式 prompt caching。
///
/// 只有在自定义 Anthropic 网关上才需要这条兜底路径。官方 Anthropic endpoint 使用顶层
/// 自动缓存来追踪不断增长的对话尾部，避免显式断点超过 4 个 slot。
pub(super) fn enable_message_caching(
    messages: &mut [AnthropicMessage],
    max_breakpoints: usize,
) -> usize {
    if messages.is_empty() || max_breakpoints == 0 {
        return 0;
    }

    let mut used = 0;
    for msg in messages.iter_mut().rev() {
        if used >= max_breakpoints {
            break;
        }

        let Some(block) = msg
            .content
            .iter_mut()
            .rev()
            .find(|block| block.can_use_explicit_cache_control())
        else {
            continue;
        };

        if block.set_cache_control_if_allowed(true) {
            used += 1;
        }
    }

    used
}

fn consume_cache_breakpoint(remaining: &mut usize) -> bool {
    if *remaining == 0 {
        return false;
    }

    *remaining -= 1;
    true
}

pub(super) fn is_official_anthropic_api_url(url: &str) -> bool {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|url| {
            url.host_str()
                .map(|host| host.eq_ignore_ascii_case("api.anthropic.com"))
        })
        .unwrap_or(false)
}

fn cache_control_if_allowed(remaining: &mut usize) -> Option<AnthropicCacheControl> {
    consume_cache_breakpoint(remaining).then(AnthropicCacheControl::ephemeral)
}

// Dynamic 层不参与缓存，动态内容每轮都变
fn cacheable_system_layer(layer: SystemPromptLayer) -> bool {
    !matches!(layer, SystemPromptLayer::Dynamic)
}

/// 将 `ToolDefinition` 转换为 Anthropic 工具定义格式。
pub(super) fn to_anthropic_tools(
    tools: &[ToolDefinition],
    remaining_cache_breakpoints: &mut usize,
) -> Vec<AnthropicTool> {
    if tools.is_empty() {
        return Vec::new();
    }

    let last_cacheable_index = tools
        .iter()
        .rposition(|tool| cacheable_text(&tool.name) || cacheable_text(&tool.description));

    tools
        .iter()
        .enumerate()
        .map(|(index, tool)| {
            let cache_control = if Some(index) == last_cacheable_index {
                cache_control_if_allowed(remaining_cache_breakpoints)
            } else {
                None
            };

            AnthropicTool {
                name: tool.name.clone(),
                description: tool.description.clone(),
                input_schema: tool.parameters.clone(),
                cache_control,
            }
        })
        .collect()
}

pub(super) fn to_anthropic_system(
    system_prompt: Option<&str>,
    system_prompt_blocks: &[SystemPromptBlock],
    remaining_cache_breakpoints: &mut usize,
) -> Option<AnthropicSystemPrompt> {
    if !system_prompt_blocks.is_empty() {
        return Some(AnthropicSystemPrompt::Blocks(
            system_prompt_blocks
                .iter()
                .map(|block| {
                    let text = block.render();
                    let cache_control = if block.cache_boundary
                        && cacheable_system_layer(block.layer)
                        && cacheable_text(&text)
                    {
                        cache_control_if_allowed(remaining_cache_breakpoints)
                    } else {
                        None
                    };

                    AnthropicSystemBlock {
                        type_: "text".to_string(),
                        text,
                        cache_control,
                    }
                })
                .collect(),
        ));
    }

    system_prompt.map(|value| AnthropicSystemPrompt::Text(value.to_string()))
}

/// 为模型生成 extended thinking 配置。
///
/// 当 max_tokens >= 2 时启用 thinking 模式，预算 token 数为 max_tokens 的 75%（向下取整）。
///
/// ## 设计动机
///
/// Extended thinking 让模型在输出前进行深度推理，提升复杂任务的回答质量。
/// 预算设为 75% 是为了保留至少 25% 的 token 给实际输出内容。
/// 如果预算为 0 或等于 max_tokens，则不启用（避免无意义配置）。
///
/// 默认为所有模型启用此功能。如果模型不支持，API 会忽略此参数。
pub(super) fn thinking_config_for_model(
    _model: &str,
    max_tokens: u32,
) -> Option<AnthropicThinking> {
    if max_tokens < 2 {
        return None;
    }

    let budget_tokens = max_tokens.saturating_mul(3) / 4;
    if budget_tokens == 0 || budget_tokens >= max_tokens {
        return None;
    }

    Some(AnthropicThinking {
        type_: "enabled".to_string(),
        budget_tokens,
    })
}

#[cfg(test)]
mod tests {
    use astrcode_core::{
        LlmMessage, ReasoningContent, SystemPromptBlock, SystemPromptLayer, ToolCallRequest,
        ToolDefinition, UserMessageOrigin,
    };
    use serde_json::{Value, json};

    use super::{ANTHROPIC_CACHE_BREAKPOINT_LIMIT, MessageBuildOptions, to_anthropic_messages};
    use crate::{
        LlmClientConfig, ModelLimits,
        anthropic::{dto::AnthropicContentBlock, provider::AnthropicProvider},
    };

    #[test]
    fn to_anthropic_messages_does_not_inject_empty_text_block_for_tool_use() {
        let messages = vec![LlmMessage::Assistant {
            content: "".to_string(),
            tool_calls: vec![ToolCallRequest {
                id: "call_123".to_string(),
                name: "test_tool".to_string(),
                args: json!({"arg": "value"}),
            }],
            reasoning: None,
        }];

        let anthropic_messages = to_anthropic_messages(
            &messages,
            MessageBuildOptions {
                include_reasoning_blocks: true,
            },
        );
        assert_eq!(anthropic_messages.len(), 1);

        let msg = &anthropic_messages[0];
        assert_eq!(msg.role, "assistant");
        assert_eq!(msg.content.len(), 1);

        match &msg.content[0] {
            AnthropicContentBlock::ToolUse { id, name, .. } => {
                assert_eq!(id, "call_123");
                assert_eq!(name, "test_tool");
            },
            _ => panic!("Expected ToolUse block"),
        }
    }

    #[test]
    fn to_anthropic_messages_groups_consecutive_tool_results_into_one_user_message() {
        let messages = vec![
            LlmMessage::Assistant {
                content: String::new(),
                tool_calls: vec![
                    ToolCallRequest {
                        id: "call_1".to_string(),
                        name: "read_file".to_string(),
                        args: json!({"path": "a.rs"}),
                    },
                    ToolCallRequest {
                        id: "call_2".to_string(),
                        name: "grep".to_string(),
                        args: json!({"pattern": "spawn"}),
                    },
                ],
                reasoning: None,
            },
            LlmMessage::Tool {
                tool_call_id: "call_1".to_string(),
                content: "file content".to_string(),
            },
            LlmMessage::Tool {
                tool_call_id: "call_2".to_string(),
                content: "grep result".to_string(),
            },
        ];

        let anthropic_messages = to_anthropic_messages(
            &messages,
            MessageBuildOptions {
                include_reasoning_blocks: true,
            },
        );

        assert_eq!(anthropic_messages.len(), 2);
        assert_eq!(anthropic_messages[0].role, "assistant");
        assert_eq!(anthropic_messages[1].role, "user");
        assert_eq!(anthropic_messages[1].content.len(), 2);
        assert!(matches!(
            &anthropic_messages[1].content[0],
            AnthropicContentBlock::ToolResult { tool_use_id, content, .. }
            if tool_use_id == "call_1" && content == "file content"
        ));
        assert!(matches!(
            &anthropic_messages[1].content[1],
            AnthropicContentBlock::ToolResult { tool_use_id, content, .. }
            if tool_use_id == "call_2" && content == "grep result"
        ));
    }

    #[test]
    fn to_anthropic_messages_keeps_user_text_after_tool_results_in_same_message() {
        let messages = vec![
            LlmMessage::Assistant {
                content: String::new(),
                tool_calls: vec![ToolCallRequest {
                    id: "call_1".to_string(),
                    name: "read_file".to_string(),
                    args: json!({"path": "a.rs"}),
                }],
                reasoning: None,
            },
            LlmMessage::Tool {
                tool_call_id: "call_1".to_string(),
                content: "file content".to_string(),
            },
            LlmMessage::User {
                content: "请继续总结发现。".to_string(),
                origin: UserMessageOrigin::User,
            },
        ];

        let anthropic_messages = to_anthropic_messages(
            &messages,
            MessageBuildOptions {
                include_reasoning_blocks: true,
            },
        );

        assert_eq!(anthropic_messages.len(), 2);
        assert_eq!(anthropic_messages[1].role, "user");
        assert_eq!(anthropic_messages[1].content.len(), 2);
        assert!(matches!(
            &anthropic_messages[1].content[0],
            AnthropicContentBlock::ToolResult { tool_use_id, content, .. }
            if tool_use_id == "call_1" && content == "file content"
        ));
        assert!(matches!(
            &anthropic_messages[1].content[1],
            AnthropicContentBlock::Text { text, .. } if text == "请继续总结发现。"
        ));
    }

    #[test]
    fn build_request_serializes_system_and_thinking_when_applicable() {
        let provider = AnthropicProvider::new(
            "https://api.anthropic.com/v1/messages".to_string(),
            "sk-ant-test".to_string(),
            "claude-sonnet-4-5".to_string(),
            ModelLimits {
                context_window: 200_000,
                max_output_tokens: 8096,
            },
            LlmClientConfig::default(),
        )
        .expect("provider should build");
        let request = provider.build_request(
            &[LlmMessage::User {
                content: "hi".to_string(),
                origin: UserMessageOrigin::User,
            }],
            &[],
            Some("Follow the rules"),
            &[],
            true,
        );
        let body = serde_json::to_value(&request).expect("request should serialize");

        assert_eq!(body["cache_control"]["type"], json!("ephemeral"));
        assert_eq!(
            body.get("system").and_then(Value::as_str),
            Some("Follow the rules")
        );
        assert_eq!(
            body.get("thinking")
                .and_then(|value| value.get("type"))
                .and_then(Value::as_str),
            Some("enabled")
        );
    }

    fn count_cache_control_fields(value: &Value) -> usize {
        match value {
            Value::Object(map) => {
                usize::from(map.contains_key("cache_control"))
                    + map.values().map(count_cache_control_fields).sum::<usize>()
            },
            Value::Array(values) => values.iter().map(count_cache_control_fields).sum(),
            _ => 0,
        }
    }

    #[test]
    fn official_anthropic_uses_automatic_cache_and_caps_explicit_breakpoints() {
        let provider = AnthropicProvider::new(
            "https://api.anthropic.com/v1/messages".to_string(),
            "sk-ant-test".to_string(),
            "claude-sonnet-4-5".to_string(),
            ModelLimits {
                context_window: 200_000,
                max_output_tokens: 8096,
            },
            LlmClientConfig::default(),
        )
        .expect("provider should build");
        let system_blocks = (0..5)
            .map(|index| SystemPromptBlock {
                title: format!("Stable {index}"),
                content: format!("stable content {index}"),
                cache_boundary: true,
                layer: SystemPromptLayer::Stable,
            })
            .collect::<Vec<_>>();
        let tools = vec![ToolDefinition {
            name: "search".to_string(),
            description: "Search indexed data.".to_string(),
            parameters: json!({ "type": "object" }),
        }];
        let request = provider.build_request(
            &[LlmMessage::User {
                content: "hi".to_string(),
                origin: UserMessageOrigin::User,
            }],
            &tools,
            None,
            &system_blocks,
            false,
        );
        let body = serde_json::to_value(&request).expect("request should serialize");

        assert_eq!(body["cache_control"]["type"], json!("ephemeral"));
        assert!(
            count_cache_control_fields(&body) <= ANTHROPIC_CACHE_BREAKPOINT_LIMIT,
            "official request should keep automatic + explicit cache controls within the provider \
             limit"
        );
        assert!(
            body["messages"][0]["content"][0]
                .get("cache_control")
                .is_none(),
            "official endpoint uses top-level automatic cache for the message tail"
        );
    }

    #[test]
    fn custom_anthropic_gateway_uses_explicit_message_tail_without_top_level_cache() {
        let provider = AnthropicProvider::new(
            "https://gateway.example.com/anthropic/v1/messages".to_string(),
            "sk-ant-test".to_string(),
            "claude-sonnet-4-5".to_string(),
            ModelLimits {
                context_window: 200_000,
                max_output_tokens: 8096,
            },
            LlmClientConfig::default(),
        )
        .expect("provider should build");
        let request = provider.build_request(
            &[
                LlmMessage::User {
                    content: "first".to_string(),
                    origin: UserMessageOrigin::User,
                },
                LlmMessage::User {
                    content: "second".to_string(),
                    origin: UserMessageOrigin::User,
                },
            ],
            &[],
            None,
            &[],
            false,
        );
        let body = serde_json::to_value(&request).expect("request should serialize");

        assert!(body.get("cache_control").is_none());
        assert_eq!(body["messages"].as_array().map(Vec::len), Some(1));
        assert_eq!(
            body["messages"][0]["content"][1]["cache_control"]["type"],
            json!("ephemeral")
        );
        assert!(
            count_cache_control_fields(&body) <= ANTHROPIC_CACHE_BREAKPOINT_LIMIT,
            "custom gateways only receive explicit cache controls within the provider limit"
        );
    }

    #[test]
    fn custom_gateway_request_disables_extended_thinking_payloads() {
        let provider = AnthropicProvider::new(
            "https://gateway.example.com/anthropic/v1/messages".to_string(),
            "sk-ant-test".to_string(),
            "claude-sonnet-4-5".to_string(),
            ModelLimits {
                context_window: 200_000,
                max_output_tokens: 8096,
            },
            LlmClientConfig::default(),
        )
        .expect("provider should build");
        let request = provider.build_request(
            &[LlmMessage::Assistant {
                content: "".to_string(),
                tool_calls: vec![],
                reasoning: Some(ReasoningContent {
                    content: "thinking".to_string(),
                    signature: Some("sig".to_string()),
                }),
            }],
            &[],
            None,
            &[],
            false,
        );
        let body = serde_json::to_value(&request).expect("request should serialize");

        assert!(body.get("thinking").is_none());
        assert_eq!(body["messages"][0]["content"][0]["type"], json!("text"));
        assert_eq!(body["messages"][0]["content"][0]["text"], json!(""));
    }

    #[test]
    fn build_request_serializes_system_blocks_with_cache_boundaries() {
        let provider = AnthropicProvider::new(
            "https://api.anthropic.com/v1/messages".to_string(),
            "sk-ant-test".to_string(),
            "claude-sonnet-4-5".to_string(),
            ModelLimits {
                context_window: 200_000,
                max_output_tokens: 8096,
            },
            LlmClientConfig::default(),
        )
        .expect("provider should build");
        let request = provider.build_request(
            &[LlmMessage::User {
                content: "hi".to_string(),
                origin: UserMessageOrigin::User,
            }],
            &[],
            Some("ignored fallback"),
            &[SystemPromptBlock {
                title: "Stable".to_string(),
                content: "stable".to_string(),
                cache_boundary: true,
                layer: SystemPromptLayer::Stable,
            }],
            false,
        );
        let body = serde_json::to_value(&request).expect("request should serialize");

        assert!(body.get("system").is_some_and(Value::is_array));
        assert_eq!(
            body["system"][0]["cache_control"]["type"],
            json!("ephemeral")
        );
    }

    #[test]
    fn build_request_only_marks_cache_boundaries_at_layer_transitions() {
        let provider = AnthropicProvider::new(
            "https://api.anthropic.com/v1/messages".to_string(),
            "sk-ant-test".to_string(),
            "claude-sonnet-4-5".to_string(),
            ModelLimits {
                context_window: 200_000,
                max_output_tokens: 8096,
            },
            LlmClientConfig::default(),
        )
        .expect("provider should build");
        let request = provider.build_request(
            &[LlmMessage::User {
                content: "hi".to_string(),
                origin: UserMessageOrigin::User,
            }],
            &[],
            Some("ignored fallback"),
            &[
                SystemPromptBlock {
                    title: "Stable 1".to_string(),
                    content: "stable content 1".to_string(),
                    cache_boundary: false,
                    layer: SystemPromptLayer::Stable,
                },
                SystemPromptBlock {
                    title: "Stable 2".to_string(),
                    content: "stable content 2".to_string(),
                    cache_boundary: false,
                    layer: SystemPromptLayer::Stable,
                },
                SystemPromptBlock {
                    title: "Stable 3".to_string(),
                    content: "stable content 3".to_string(),
                    cache_boundary: true,
                    layer: SystemPromptLayer::Stable,
                },
                SystemPromptBlock {
                    title: "Semi 1".to_string(),
                    content: "semi content 1".to_string(),
                    cache_boundary: false,
                    layer: SystemPromptLayer::SemiStable,
                },
                SystemPromptBlock {
                    title: "Semi 2".to_string(),
                    content: "semi content 2".to_string(),
                    cache_boundary: true,
                    layer: SystemPromptLayer::SemiStable,
                },
                SystemPromptBlock {
                    title: "Inherited 1".to_string(),
                    content: "inherited content 1".to_string(),
                    cache_boundary: true,
                    layer: SystemPromptLayer::Inherited,
                },
                SystemPromptBlock {
                    title: "Dynamic 1".to_string(),
                    content: "dynamic content 1".to_string(),
                    cache_boundary: true,
                    layer: SystemPromptLayer::Dynamic,
                },
            ],
            false,
        );
        let body = serde_json::to_value(&request).expect("request should serialize");

        assert!(body.get("system").is_some_and(Value::is_array));
        assert_eq!(
            body["system"]
                .as_array()
                .expect("system should be an array")
                .len(),
            7
        );

        // Stable 层内的前两个 block 不应该有 cache_control
        assert!(
            body["system"][0].get("cache_control").is_none(),
            "stable1 should not have cache_control"
        );
        assert!(
            body["system"][1].get("cache_control").is_none(),
            "stable2 should not have cache_control"
        );

        // Stable 层的最后一个 block 应该有 cache_control
        assert_eq!(
            body["system"][2]["cache_control"]["type"],
            json!("ephemeral"),
            "stable3 should have cache_control"
        );

        // SemiStable 层的第一个 block 不应该有 cache_control
        assert!(
            body["system"][3].get("cache_control").is_none(),
            "semi1 should not have cache_control"
        );

        // SemiStable 层的最后一个 block 应该有 cache_control
        assert_eq!(
            body["system"][4]["cache_control"]["type"],
            json!("ephemeral"),
            "semi2 should have cache_control"
        );

        // Inherited 层允许独立缓存
        assert_eq!(
            body["system"][5]["cache_control"]["type"],
            json!("ephemeral"),
            "inherited1 should have cache_control"
        );

        // Dynamic 层不缓存（避免浪费，因为内容变化频繁）
        // TODO: 更好的做法？实现更好的kv缓存？
        assert!(
            body["system"][6].get("cache_control").is_none(),
            "dynamic1 should not have cache_control (Dynamic layer is not cached)"
        );
    }
}
