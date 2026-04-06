//! # Anthropic Messages API 提供者
//!
//! 实现了 [`LlmProvider`] trait，对接 Anthropic Claude 系列模型。
//!
//! ## 协议特性
//!
//! - **Extended Thinking**: 自动为 Claude 模型启用深度推理模式（`thinking` 配置）， 预算 token 设为
//!   `max_tokens` 的 75%，保留至少 25% 给实际输出
//! - **Prompt Caching**: 对最后 2 条消息标记 `ephemeral` 缓存控制，复用 KV cache
//! - **SSE 流式解析**: Anthropic 使用多行 SSE 块格式（`event: ...\ndata: {...}\n\n`）， 与 OpenAI
//!   的单行 `data: {...}` 不同，因此有独立的解析逻辑
//! - **内容块模型**: Anthropic 响应由多种内容块组成（text / tool_use / thinking）， 使用
//!   `Vec<Value>` 灵活处理未知或新增的块类型
//!
//! ## 流式事件分派
//!
//! Anthropic SSE 事件类型：
//! - `content_block_start`: 新内容块开始（文本或工具调用）
//! - `content_block_delta`: 增量内容（text_delta / thinking_delta / signature_delta /
//!   input_json_delta）
//! - `message_stop`: 流结束信号
//! - `message_start / message_delta / content_block_stop / ping`: 元数据事件，静默忽略

use std::fmt;

use astrcode_core::{
    AstrError, CancelToken, LlmMessage, ReasoningContent, Result, ToolCallRequest, ToolDefinition,
};
use async_trait::async_trait;
use futures_util::StreamExt;
use log::{debug, warn};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::select;

use crate::{
    EventSink, FinishReason, LlmAccumulator, LlmEvent, LlmOutput, LlmProvider, LlmRequest,
    LlmUsage, MAX_RETRIES, ModelLimits, Utf8StreamDecoder, build_http_client, classify_http_error,
    emit_event, is_retryable_status, wait_retry_delay,
};
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Anthropic Claude API 提供者实现。
///
/// 封装了 HTTP 客户端、API 密钥和模型配置，提供统一的 [`LlmProvider`] 接口。
///
/// ## 设计要点
///
/// - HTTP 客户端在构造时创建，使用共享的超时策略（连接 10s / 读取 90s）
/// - `limits.max_output_tokens` 同时控制请求体的上限和 extended thinking 的预算计算
/// - Debug 实现会隐藏 API 密钥（显示为 `<redacted>`）
#[derive(Clone)]
pub struct AnthropicProvider {
    client: reqwest::Client,
    messages_api_url: String,
    api_key: String,
    model: String,
    /// 运行时已解析好的模型 limits。
    ///
    /// Anthropic 的上下文窗口来自 Models API，不应该继续在 provider 内写死。
    limits: ModelLimits,
}

impl fmt::Debug for AnthropicProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnthropicProvider")
            .field("client", &self.client)
            .field("messages_api_url", &self.messages_api_url)
            .field("api_key", &"<redacted>")
            .field("model", &self.model)
            .field("limits", &self.limits)
            .finish()
    }
}

impl AnthropicProvider {
    /// 创建新的 Anthropic 提供者实例。
    ///
    /// `limits.max_output_tokens` 同时用于：
    /// 1. 请求体中的 `max_tokens` 字段（输出上限）
    /// 2. Extended thinking 预算计算（75% 的 max_tokens）
    pub fn new(
        messages_api_url: String,
        api_key: String,
        model: String,
        limits: ModelLimits,
    ) -> Result<Self> {
        Ok(Self {
            client: build_http_client()?,
            messages_api_url,
            api_key,
            model,
            limits,
        })
    }

    /// 构建 Anthropic Messages API 请求体。
    ///
    /// - 将 `LlmMessage` 转换为 Anthropic 格式的内容块数组
    /// - 对最后 2 条消息启用 prompt caching（KV cache 复用）
    /// - 如果启用了工具，附加工具定义
    /// - 根据模型名称和 max_tokens 自动配置 extended thinking
    fn build_request(
        &self,
        messages: &[LlmMessage],
        tools: &[ToolDefinition],
        system_prompt: Option<&str>,
        stream: bool,
    ) -> AnthropicRequest {
        let mut anthropic_messages = to_anthropic_messages(messages);
        // Enable prompt caching on the last 2 message blocks for KV cache reuse
        enable_message_caching(&mut anthropic_messages, 2);

        AnthropicRequest {
            model: self.model.clone(),
            max_tokens: self.limits.max_output_tokens.min(u32::MAX as usize) as u32,
            messages: anthropic_messages,
            system: system_prompt.map(str::to_string),
            tools: if tools.is_empty() {
                None
            } else {
                Some(to_anthropic_tools(tools))
            },
            stream: stream.then_some(true),
            thinking: thinking_config_for_model(
                &self.model,
                self.limits.max_output_tokens.min(u32::MAX as usize) as u32,
            ),
        }
    }

    async fn send_request(
        &self,
        request: &AnthropicRequest,
        cancel: CancelToken,
    ) -> Result<reqwest::Response> {
        // 调试日志：打印请求信息（不暴露完整 API Key）
        let api_key_preview = if self.api_key.len() > 8 {
            format!(
                "{}...{}",
                &self.api_key[..4],
                &self.api_key[self.api_key.len() - 4..]
            )
        } else {
            "****".to_string()
        };
        debug!(
            "Anthropic request: url={}, api_key_preview={}, model={}",
            self.messages_api_url, api_key_preview, self.model
        );

        for attempt in 0..=MAX_RETRIES {
            let send_future = self
                .client
                .post(&self.messages_api_url)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .json(request)
                .send();

            let response = select! {
                _ = crate::cancelled(cancel.clone()) => {
                    return Err(AstrError::LlmInterrupted);
                }
                result = send_future => result.map_err(|e| AstrError::http("failed to call anthropic endpoint", e))
            };

            match response {
                Ok(response) => {
                    let status = response.status();
                    if status == reqwest::StatusCode::UNAUTHORIZED {
                        // 读取响应体以便调试
                        let body = response.text().await.unwrap_or_default();
                        warn!(
                            "Anthropic 401 Unauthorized: url={}, api_key_preview={}, response={}",
                            self.messages_api_url,
                            if self.api_key.len() > 8 {
                                format!(
                                    "{}...{}",
                                    &self.api_key[..4],
                                    &self.api_key[self.api_key.len() - 4..]
                                )
                            } else {
                                "****".to_string()
                            },
                            body
                        );
                        return Err(AstrError::InvalidApiKey("Anthropic".to_string()));
                    }
                    if status.is_success() {
                        return Ok(response);
                    }

                    let body = response.text().await.unwrap_or_default();
                    if is_retryable_status(status) && attempt < MAX_RETRIES {
                        wait_retry_delay(attempt, cancel.clone()).await?;
                        continue;
                    }

                    // 使用结构化错误分类 (P4.3)
                    return Err(classify_http_error(status.as_u16(), &body).into());
                },
                Err(error) => {
                    if error.is_retryable() && attempt < MAX_RETRIES {
                        wait_retry_delay(attempt, cancel.clone()).await?;
                        continue;
                    }
                    return Err(error);
                },
            }
        }

        // 所有路径都会通过 return 退出循环；若到达此处说明逻辑有误，
        // 返回 Internal 而非 panic 以保证运行时安全
        Err(AstrError::Internal(
            "retry loop should have returned on all paths".into(),
        ))
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn generate(&self, request: LlmRequest, sink: Option<EventSink>) -> Result<LlmOutput> {
        let cancel = request.cancel;
        let body = self.build_request(
            &request.messages,
            &request.tools,
            request.system_prompt.as_deref(),
            sink.is_some(),
        );
        let response = self.send_request(&body, cancel.clone()).await?;

        match sink {
            None => {
                let payload: AnthropicResponse = response
                    .json()
                    .await
                    .map_err(|e| AstrError::http("failed to parse anthropic response", e))?;
                Ok(response_to_output(payload))
            },
            Some(sink) => {
                let mut stream = response.bytes_stream();
                let mut sse_buffer = String::new();
                let mut utf8_decoder = Utf8StreamDecoder::default();
                let mut accumulator = LlmAccumulator::default();
                // 流式路径下从 message_delta 的 stop_reason 提取 (P4.2)
                let mut stream_stop_reason: Option<String> = None;

                loop {
                    let next_item = select! {
                        _ = crate::cancelled(cancel.clone()) => {
                            return Err(AstrError::LlmInterrupted);
                        }
                        item = stream.next() => item,
                    };

                    let Some(item) = next_item else {
                        break;
                    };

                    let bytes = item.map_err(|e| {
                        AstrError::http("failed to read anthropic response stream", e)
                    })?;
                    let Some(chunk_text) = utf8_decoder
                        .push(&bytes, "anthropic response stream was not valid utf-8")?
                    else {
                        continue;
                    };

                    if consume_sse_text_chunk(
                        &chunk_text,
                        &mut sse_buffer,
                        &mut accumulator,
                        &sink,
                        &mut stream_stop_reason,
                    )? {
                        let mut output = accumulator.finish();
                        // 优先使用 API 返回的 stop_reason，否则使用推断值
                        if let Some(reason) = stream_stop_reason.as_deref() {
                            output.finish_reason = FinishReason::from_api_value(reason);
                        }
                        return Ok(output);
                    }
                }

                if let Some(tail_text) =
                    utf8_decoder.finish("anthropic response stream was not valid utf-8")?
                {
                    let done = consume_sse_text_chunk(
                        &tail_text,
                        &mut sse_buffer,
                        &mut accumulator,
                        &sink,
                        &mut stream_stop_reason,
                    )?;
                    if done {
                        let mut output = accumulator.finish();
                        if let Some(reason) = stream_stop_reason.as_deref() {
                            output.finish_reason = FinishReason::from_api_value(reason);
                        }
                        return Ok(output);
                    }
                }

                flush_sse_buffer(
                    &mut sse_buffer,
                    &mut accumulator,
                    &sink,
                    &mut stream_stop_reason,
                )?;
                let mut output = accumulator.finish();
                if let Some(reason) = stream_stop_reason.as_deref() {
                    output.finish_reason = FinishReason::from_api_value(reason);
                }
                Ok(output)
            },
        }
    }

    fn model_limits(&self) -> ModelLimits {
        self.limits
    }
}

/// 将 `LlmMessage` 转换为 Anthropic 格式的消息结构。
///
/// Anthropic 使用内容块数组（而非纯文本），因此需要按消息类型分派：
/// - User 消息 → 单个 `text` 内容块
/// - Assistant 消息 → 可能包含 `thinking`、`text`、`tool_use` 多个块
/// - Tool 消息 → 单个 `tool_result` 内容块
fn to_anthropic_messages(messages: &[LlmMessage]) -> Vec<AnthropicMessage> {
    messages
        .iter()
        .map(|message| match message {
            LlmMessage::User { content, .. } => AnthropicMessage {
                role: "user".to_string(),
                content: vec![AnthropicContentBlock::Text {
                    text: content.clone(),
                    cache_control: None,
                }],
            },
            LlmMessage::Assistant {
                content,
                tool_calls,
                reasoning,
            } => {
                let mut blocks = Vec::new();
                if let Some(reasoning) = reasoning {
                    blocks.push(AnthropicContentBlock::Thinking {
                        thinking: reasoning.content.clone(),
                        signature: reasoning.signature.clone(),
                        cache_control: None,
                    });
                }
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

                AnthropicMessage {
                    role: "assistant".to_string(),
                    content: blocks,
                }
            },
            LlmMessage::Tool {
                tool_call_id,
                content,
            } => AnthropicMessage {
                role: "user".to_string(),
                content: vec![AnthropicContentBlock::ToolResult {
                    tool_use_id: tool_call_id.clone(),
                    content: content.clone(),
                    cache_control: None,
                }],
            },
        })
        .collect()
}

/// Enable prompt caching on the last `cache_depth` message content blocks.
/// Anthropic caches the last marked block and all preceding blocks up to
/// the previous cache marker, so marking the final N blocks effectively
/// caches the tail of the conversation for KV cache reuse.
fn enable_message_caching(messages: &mut [AnthropicMessage], cache_depth: usize) {
    if messages.is_empty() || cache_depth == 0 {
        return;
    }

    let cache_count = cache_depth.min(messages.len());
    let start_idx = messages.len() - cache_count;

    for msg in &mut messages[start_idx..] {
        if let Some(last_block) = msg.content.last_mut() {
            last_block.set_cache_control(true);
        }
    }
}

/// 将 `ToolDefinition` 转换为 Anthropic 工具定义格式。
fn to_anthropic_tools(tools: &[ToolDefinition]) -> Vec<AnthropicTool> {
    tools
        .iter()
        .map(|tool| AnthropicTool {
            name: tool.name.clone(),
            description: tool.description.clone(),
            input_schema: tool.parameters.clone(),
        })
        .collect()
}

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
fn response_to_output(response: AnthropicResponse) -> LlmOutput {
    let usage = response.usage.map(|usage| LlmUsage {
        input_tokens: usage.input_tokens.unwrap_or_default() as usize,
        output_tokens: usage.output_tokens.unwrap_or_default() as usize,
    });

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

/// 解析单个 Anthropic SSE 块。
///
/// Anthropic SSE 块由多行组成（`event: ...\ndata: {...}\n\n`），
/// 本函数提取事件类型和 JSON payload，支持事件类型回退到 payload 中的 `type` 字段。
fn parse_sse_block(block: &str) -> Result<Option<(String, Value)>> {
    let trimmed = block.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let mut event_type = None;
    let mut data_lines = Vec::new();

    for line in trimmed.lines() {
        if let Some(value) = sse_field_value(line, "event") {
            event_type = Some(value.trim().to_string());
        } else if let Some(value) = sse_field_value(line, "data") {
            data_lines.push(value);
        }
    }

    if data_lines.is_empty() {
        return Ok(None);
    }

    let data = data_lines.join("\n");
    let data = data.trim();
    if data.is_empty() {
        return Ok(None);
    }

    // 兼容部分 Anthropic 网关沿用 OpenAI 风格的流结束哨兵。
    // 如果这里严格要求 JSON，会在流尾直接误报 parse error。
    if data == "[DONE]" {
        return Ok(Some((
            "message_stop".to_string(),
            json!({ "type": "message_stop" }),
        )));
    }

    let payload = serde_json::from_str::<Value>(data)
        .map_err(|error| AstrError::parse("failed to parse anthropic sse payload", error))?;
    let event_type = event_type
        .or_else(|| {
            payload
                .get("type")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default();

    Ok(Some((event_type, payload)))
}

fn sse_field_value<'a>(line: &'a str, field: &str) -> Option<&'a str> {
    let value = line.strip_prefix(field)?.strip_prefix(':')?;

    // SSE 规范只忽略冒号后的一个可选空格；这里兼容 `data:...` 和 `data: ...`，
    // 同时保留业务数据中其余前导空白，避免悄悄改写 payload。
    Some(value.strip_prefix(' ').unwrap_or(value))
}

/// 从 `content_block_start` 事件 payload 中提取内容块。
///
/// Anthropic 在 `content_block_start` 事件中将块数据放在 `content_block` 字段，
/// 但某些事件可能直接放在根级别，因此有回退逻辑。
fn extract_start_block(payload: &Value) -> &Value {
    payload.get("content_block").unwrap_or(payload)
}

/// 从 `content_block_delta` 事件 payload 中提取增量数据。
///
/// Anthropic 在 `content_block_delta` 事件中将增量数据放在 `delta` 字段。
fn extract_delta_block(payload: &Value) -> &Value {
    payload.get("delta").unwrap_or(payload)
}

/// 处理单个 Anthropic SSE 块，返回 `(is_done, stop_reason)`。
///
/// Anthropic SSE 事件类型分派：
/// - `content_block_start`: 新内容块开始（可能是文本或工具调用）
/// - `content_block_delta`: 增量内容（文本/思考/签名/工具参数）
/// - `message_stop`: 流结束信号，返回 is_done=true
/// - `message_delta`: 包含 `stop_reason`，用于检测 max_tokens 截断 (P4.2)
/// - `message_start/content_block_stop/ping`: 元数据事件，静默忽略
fn process_sse_block(
    block: &str,
    accumulator: &mut LlmAccumulator,
    sink: &EventSink,
) -> Result<(bool, Option<String>)> {
    let Some((event_type, payload)) = parse_sse_block(block)? else {
        return Ok((false, None));
    };

    match event_type.as_str() {
        "content_block_start" => {
            let index = payload
                .get("index")
                .and_then(Value::as_u64)
                .unwrap_or_default() as usize;
            let block = extract_start_block(&payload);

            // 工具调用块开始时，发射 ToolCallDelta（id + name，参数为空）
            if block_type(block) == Some("tool_use") {
                emit_event(
                    LlmEvent::ToolCallDelta {
                        index,
                        id: block.get("id").and_then(Value::as_str).map(str::to_string),
                        name: block
                            .get("name")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        arguments_delta: String::new(),
                    },
                    accumulator,
                    sink,
                );
            }
            Ok((false, None))
        },
        "content_block_delta" => {
            let index = payload
                .get("index")
                .and_then(Value::as_u64)
                .unwrap_or_default() as usize;
            let delta = extract_delta_block(&payload);

            // 根据增量类型分派到对应的事件
            match block_type(delta) {
                Some("text_delta") => {
                    if let Some(text) = delta.get("text").and_then(Value::as_str) {
                        emit_event(LlmEvent::TextDelta(text.to_string()), accumulator, sink);
                    }
                },
                Some("thinking_delta") => {
                    if let Some(text) = delta.get("thinking").and_then(Value::as_str) {
                        emit_event(LlmEvent::ThinkingDelta(text.to_string()), accumulator, sink);
                    }
                },
                Some("signature_delta") => {
                    if let Some(signature) = delta.get("signature").and_then(Value::as_str) {
                        emit_event(
                            LlmEvent::ThinkingSignature(signature.to_string()),
                            accumulator,
                            sink,
                        );
                    }
                },
                Some("input_json_delta") => {
                    // 工具调用参数增量，partial_json 是 JSON 的片段
                    emit_event(
                        LlmEvent::ToolCallDelta {
                            index,
                            id: None,
                            name: None,
                            arguments_delta: delta
                                .get("partial_json")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                        },
                        accumulator,
                        sink,
                    );
                },
                _ => {},
            }
            Ok((false, None))
        },
        "message_stop" => Ok((true, None)),
        // message_delta 可能包含 stop_reason (P4.2)
        "message_delta" => {
            let stop_reason = payload
                .get("delta")
                .and_then(|d| d.get("stop_reason"))
                .and_then(Value::as_str)
                .map(str::to_string);
            Ok((false, stop_reason))
        },
        "message_start" | "content_block_stop" | "ping" => Ok((false, None)),
        other => {
            warn!("anthropic: unknown sse event: {}", other);
            Ok((false, None))
        },
    }
}

/// 为 Claude 模型生成 extended thinking 配置。
///
/// 当模型名称以 `claude-` 开头且 max_tokens >= 2 时启用 thinking 模式，
/// 预算 token 数为 max_tokens 的 75%（向下取整）。
///
/// ## 设计动机
///
/// Extended thinking 让 Claude 在输出前进行深度推理，提升复杂任务的回答质量。
/// 预算设为 75% 是为了保留至少 25% 的 token 给实际输出内容。
/// 如果预算为 0 或等于 max_tokens，则不启用（避免无意义配置）。
fn thinking_config_for_model(model: &str, max_tokens: u32) -> Option<AnthropicThinking> {
    if !model.starts_with("claude-") || max_tokens < 2 {
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

/// 在 SSE 缓冲区中查找下一个完整的 SSE 块边界。
///
/// Anthropic SSE 块由双换行符分隔（`\r\n\r\n` 或 `\n\n`）。
/// 返回 `(块结束位置, 分隔符长度)`，如果未找到完整块则返回 `None`。
fn next_sse_block(buffer: &str) -> Option<(usize, usize)> {
    if let Some(idx) = buffer.find("\r\n\r\n") {
        return Some((idx, 4));
    }
    if let Some(idx) = buffer.find("\n\n") {
        return Some((idx, 2));
    }
    None
}

fn consume_sse_text_chunk(
    chunk_text: &str,
    sse_buffer: &mut String,
    accumulator: &mut LlmAccumulator,
    sink: &EventSink,
    stop_reason_out: &mut Option<String>,
) -> Result<bool> {
    sse_buffer.push_str(chunk_text);

    while let Some((block_end, delimiter_len)) = next_sse_block(sse_buffer) {
        let block: String = sse_buffer.drain(..block_end + delimiter_len).collect();
        let block = &block[..block_end];

        let (done, reason) = process_sse_block(block, accumulator, sink)?;
        if let Some(r) = reason {
            *stop_reason_out = Some(r);
        }
        if done {
            return Ok(true);
        }
    }

    Ok(false)
}

fn flush_sse_buffer(
    sse_buffer: &mut String,
    accumulator: &mut LlmAccumulator,
    sink: &EventSink,
    stop_reason_out: &mut Option<String>,
) -> Result<()> {
    if sse_buffer.trim().is_empty() {
        sse_buffer.clear();
        return Ok(());
    }

    let (done, reason) = process_sse_block(sse_buffer, accumulator, sink)?;
    if let Some(r) = reason {
        *stop_reason_out = Some(r);
    }
    let _ = done;
    sse_buffer.clear();
    Ok(())
}

// ---------------------------------------------------------------------------
// Anthropic API 请求/响应 DTO（仅用于 serde 序列化/反序列化）
// ---------------------------------------------------------------------------

/// Anthropic Messages API 请求体。
///
/// 注意：`stream` 字段为 `Option<bool>`，`None` 时表示非流式模式，
/// 这样可以在序列化时省略该字段（Anthropic API 默认非流式）。
#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<AnthropicThinking>,
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
#[derive(Debug, Serialize)]
struct AnthropicThinking {
    #[serde(rename = "type")]
    type_: String,
    budget_tokens: u32,
}

/// Anthropic 消息（包含角色和内容块数组）。
///
/// Anthropic 的消息结构与 OpenAI 不同：`content` 是内容块数组而非纯文本，
/// 这使得单条消息可以混合文本、推理、工具调用等多种内容类型。
#[derive(Debug, Serialize)]
struct AnthropicMessage {
    role: String,
    content: Vec<AnthropicContentBlock>,
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
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicContentBlock {
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
        cache_control: Option<AnthropicCacheControl>,
    },
}

/// Anthropic prompt caching 控制标记。
///
/// `type: "ephemeral"` 告诉 Anthropic 后端该块可作为缓存前缀的一部分。
/// 缓存是临时的（ephemeral），不保证长期有效，但在短时间内重复请求可以显著减少延迟。
#[derive(Debug, Clone, Serialize)]
struct AnthropicCacheControl {
    #[serde(rename = "type")]
    type_: String,
}

impl AnthropicCacheControl {
    /// 创建 ephemeral 类型的缓存控制标记。
    fn ephemeral() -> Self {
        Self {
            type_: "ephemeral".to_string(),
        }
    }
}

impl AnthropicContentBlock {
    /// 为内容块设置或清除 cache_control 标记。
    ///
    /// 通过模式匹配更新所有变体的 `cache_control` 字段，
    /// 保持枚举变体间的修改逻辑集中。
    fn set_cache_control(&mut self, enabled: bool) {
        let control = if enabled {
            Some(AnthropicCacheControl::ephemeral())
        } else {
            None
        };
        match self {
            AnthropicContentBlock::Text { cache_control, .. } => *cache_control = control,
            AnthropicContentBlock::Thinking { cache_control, .. } => *cache_control = control,
            AnthropicContentBlock::ToolUse { cache_control, .. } => *cache_control = control,
            AnthropicContentBlock::ToolResult { cache_control, .. } => *cache_control = control,
        }
    }
}

/// Anthropic 工具定义。
///
/// 与 OpenAI 不同，Anthropic 工具定义不需要 `type` 字段，
/// 直接使用 `name`、`description`、`input_schema` 三个字段。
#[derive(Debug, Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: Value,
}

/// Anthropic Messages API 非流式响应体。
///
/// NOTE: `content` 使用 `Vec<Value>` 而非强类型结构体，
/// 因为 Anthropic 响应可能包含多种内容块类型（text / tool_use / thinking），
/// 使用 `Value` 可以灵活处理未知或新增的块类型，避免每次 API 更新都要修改 DTO。
#[derive(Debug, serde::Deserialize)]
struct AnthropicResponse {
    content: Vec<Value>,
    #[allow(dead_code)]
    stop_reason: Option<String>,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

/// Anthropic 响应中的 token 用量统计。
///
/// 两个字段均为 `Option` 且带 `#[serde(default)]`，
/// 因为某些旧版 API 或特殊响应可能不包含用量信息。
#[derive(Debug, serde::Deserialize)]
struct AnthropicUsage {
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use astrcode_core::UserMessageOrigin;
    use serde_json::json;

    use super::*;
    use crate::sink_collector;

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
    fn streaming_sse_parses_tool_calls_and_text() {
        let mut accumulator = LlmAccumulator::default();
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = sink_collector(events.clone());
        let mut sse_buffer = String::new();

        let chunk = concat!(
            "event: content_block_start\n",
            "data: {\"index\":1,\"type\":\"tool_use\",\"id\":\"call_1\",\"name\":\"search\"}\n\n",
            "event: content_block_delta\n",
            "data: {\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"\
             q\\\":\\\"ru\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"st\\\"\
             }\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n"
        );

        let mut stop_reason_out: Option<String> = None;
        let done = consume_sse_text_chunk(
            chunk,
            &mut sse_buffer,
            &mut accumulator,
            &sink,
            &mut stop_reason_out,
        )
        .expect("stream chunk should parse");

        assert!(done);
        let output = accumulator.finish();
        let events = events.lock().expect("lock").clone();

        assert!(events.iter().any(|event| {
            matches!(
                event,
                LlmEvent::ToolCallDelta { index, id, name, arguments_delta }
                if *index == 1
                    && id.as_deref() == Some("call_1")
                    && name.as_deref() == Some("search")
                    && arguments_delta.is_empty()
            )
        }));
        assert!(
            events
                .iter()
                .any(|event| matches!(event, LlmEvent::TextDelta(text) if text == "hello"))
        );
        assert_eq!(output.content, "hello");
        assert_eq!(output.tool_calls.len(), 1);
        assert_eq!(output.tool_calls[0].args, json!({ "q": "rust" }));
    }

    #[test]
    fn parse_sse_block_accepts_data_lines_without_space_after_colon() {
        let block = concat!(
            "event:content_block_delta\n",
            "data:{\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n"
        );

        let parsed = parse_sse_block(block)
            .expect("block should parse")
            .expect("block should contain payload");

        assert_eq!(parsed.0, "content_block_delta");
        assert_eq!(parsed.1["delta"]["text"], json!("hello"));
    }

    #[test]
    fn parse_sse_block_treats_done_sentinel_as_message_stop() {
        let parsed = parse_sse_block("data: [DONE]\n")
            .expect("done sentinel should parse")
            .expect("done sentinel should produce payload");

        assert_eq!(parsed.0, "message_stop");
        assert_eq!(parsed.1["type"], json!("message_stop"));
    }

    #[test]
    fn parse_sse_block_ignores_empty_data_payload() {
        let parsed = parse_sse_block("event: ping\ndata:\n");
        assert!(matches!(parsed, Ok(None)));
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
        )
        .expect("provider should build");
        let request = provider.build_request(
            &[LlmMessage::User {
                content: "hi".to_string(),
                origin: UserMessageOrigin::User,
            }],
            &[],
            Some("Follow the rules"),
            true,
        );
        let body = serde_json::to_value(&request).expect("request should serialize");

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

    #[test]
    fn provider_keeps_custom_messages_api_url() {
        let provider = AnthropicProvider::new(
            "https://gateway.example.com/anthropic/v1/messages".to_string(),
            "sk-ant-test".to_string(),
            "claude-sonnet-4-5".to_string(),
            ModelLimits {
                context_window: 200_000,
                max_output_tokens: 8096,
            },
        )
        .expect("provider should build");

        assert_eq!(
            provider.messages_api_url,
            "https://gateway.example.com/anthropic/v1/messages"
        );
    }

    #[test]
    fn streaming_sse_handles_multibyte_text_split_across_chunks() {
        let mut accumulator = LlmAccumulator::default();
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = sink_collector(events.clone());
        let mut sse_buffer = String::new();
        let mut decoder = Utf8StreamDecoder::default();
        let mut stop_reason_out = None;
        let chunk = concat!(
            "event: content_block_delta\n",
            "data: {\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"你",
            "好\"}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n"
        );
        let bytes = chunk.as_bytes();
        let split_index = chunk
            .find("好")
            .expect("chunk should contain multibyte char")
            + 1;

        let first_text = decoder
            .push(
                &bytes[..split_index],
                "anthropic response stream was not valid utf-8",
            )
            .expect("first split should decode");
        let second_text = decoder
            .push(
                &bytes[split_index..],
                "anthropic response stream was not valid utf-8",
            )
            .expect("second split should decode");

        let first_done = first_text
            .as_deref()
            .map(|text| {
                consume_sse_text_chunk(
                    text,
                    &mut sse_buffer,
                    &mut accumulator,
                    &sink,
                    &mut stop_reason_out,
                )
            })
            .transpose()
            .expect("first chunk should parse")
            .unwrap_or(false);
        let second_done = second_text
            .as_deref()
            .map(|text| {
                consume_sse_text_chunk(
                    text,
                    &mut sse_buffer,
                    &mut accumulator,
                    &sink,
                    &mut stop_reason_out,
                )
            })
            .transpose()
            .expect("second chunk should parse")
            .unwrap_or(false);

        assert!(!first_done);
        assert!(second_done);
        let output = accumulator.finish();
        let events = events.lock().expect("lock").clone();

        assert!(
            events
                .iter()
                .any(|event| matches!(event, LlmEvent::TextDelta(text) if text == "你好"))
        );
        assert_eq!(output.content, "你好");
    }
}
