use std::fmt;

use astrcode_core::{AstrError, CancelToken, ReasoningContent, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use log::warn;
use serde::Serialize;
use serde_json::Value;
use tokio::select;

use crate::{
    build_http_client, emit_event, is_retryable_status, wait_retry_delay, EventSink,
    LlmAccumulator, LlmEvent, LlmOutput, LlmProvider, LlmRequest, LlmUsage, ModelLimits,
    MAX_RETRIES,
};
use astrcode_core::{LlmMessage, ToolCallRequest, ToolDefinition};

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Clone)]
pub struct AnthropicProvider {
    client: reqwest::Client,
    api_key: String,
    model: String,
    max_tokens: u32,
}

impl fmt::Debug for AnthropicProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnthropicProvider")
            .field("client", &self.client)
            .field("api_key", &"<redacted>")
            .field("model", &self.model)
            .field("max_tokens", &self.max_tokens)
            .finish()
    }
}

impl AnthropicProvider {
    pub fn with_max_tokens(api_key: String, model: String, max_tokens: u32) -> Self {
        Self {
            client: build_http_client(),
            api_key,
            model,
            max_tokens,
        }
    }

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
            max_tokens: self.max_tokens,
            messages: anthropic_messages,
            system: system_prompt.map(str::to_string),
            tools: if tools.is_empty() {
                None
            } else {
                Some(to_anthropic_tools(tools))
            },
            stream: stream.then_some(true),
            thinking: thinking_config_for_model(&self.model, self.max_tokens),
        }
    }

    async fn send_request(
        &self,
        request: &AnthropicRequest,
        cancel: CancelToken,
    ) -> Result<reqwest::Response> {
        for attempt in 0..=MAX_RETRIES {
            let send_future = self
                .client
                .post(ANTHROPIC_API_URL)
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

                    let error_kind = if is_retryable_status(status) {
                        "retryable"
                    } else {
                        "non-retryable"
                    };
                    return Err(AstrError::LlmRequestFailed {
                        status: status.as_u16(),
                        body: format!("Anthropic 请求失败 ({}): {}", error_kind, body),
                    });
                }
                Err(error) => {
                    if error.is_retryable() && attempt < MAX_RETRIES {
                        wait_retry_delay(attempt, cancel.clone()).await?;
                        continue;
                    }
                    return Err(error);
                }
            }
        }

        Err(AstrError::LlmStreamError(
            "Anthropic 请求在重试后仍然失败".to_string(),
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
            }
            Some(sink) => {
                let mut stream = response.bytes_stream();
                let mut sse_buffer = String::new();
                let mut accumulator = LlmAccumulator::default();

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
                    let chunk_text = std::str::from_utf8(&bytes).map_err(|e| AstrError::Utf8 {
                        context: "anthropic response stream was not valid utf-8".to_string(),
                        source: e,
                    })?;

                    if consume_sse_text_chunk(chunk_text, &mut sse_buffer, &mut accumulator, &sink)?
                    {
                        return Ok(accumulator.finish());
                    }
                }

                flush_sse_buffer(&mut sse_buffer, &mut accumulator, &sink)?;
                Ok(accumulator.finish())
            }
        }
    }

    fn model_limits(&self) -> ModelLimits {
        ModelLimits {
            // Claude model families currently expose 200k-class context windows. We keep the
            // heuristic conservative and local until provider APIs return explicit limits.
            context_window: 200_000,
            max_output_tokens: self.max_tokens as usize,
        }
    }
}

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
            }
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

fn response_to_output(response: AnthropicResponse) -> LlmOutput {
    let mut output = LlmOutput::default();
    let _ = response.stop_reason;
    output.usage = response.usage.map(|usage| LlmUsage {
        input_tokens: usage.input_tokens.unwrap_or_default() as usize,
        output_tokens: usage.output_tokens.unwrap_or_default() as usize,
    });

    for block in response.content {
        match block_type(&block) {
            Some("text") => {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    output.content.push_str(text);
                }
            }
            Some("tool_use") => {
                let id = match block.get("id").and_then(Value::as_str) {
                    Some(id) if !id.is_empty() => id.to_string(),
                    _ => {
                        warn!("anthropic: tool_use block missing non-empty id, skipping");
                        continue;
                    }
                };
                let name = match block.get("name").and_then(Value::as_str) {
                    Some(name) if !name.is_empty() => name.to_string(),
                    _ => {
                        warn!("anthropic: tool_use block missing non-empty name, skipping");
                        continue;
                    }
                };
                let args = block.get("input").cloned().unwrap_or(Value::Null);
                output.tool_calls.push(ToolCallRequest { id, name, args });
            }
            Some("thinking") => {
                if let Some(thinking) = block.get("thinking").and_then(Value::as_str) {
                    output.reasoning = Some(ReasoningContent {
                        content: thinking.to_string(),
                        signature: block
                            .get("signature")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    });
                }
            }
            Some(other) => {
                warn!("anthropic: unknown content block type: {}", other);
            }
            None => {
                warn!("anthropic: content block missing type");
            }
        }
    }

    output
}

fn block_type(value: &Value) -> Option<&str> {
    value.get("type").and_then(Value::as_str)
}

fn parse_sse_block(block: &str) -> Result<Option<(String, Value)>> {
    let trimmed = block.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let mut event_type = None;
    let mut data_lines = Vec::new();

    for line in trimmed.lines() {
        if let Some(value) = line.strip_prefix("event: ") {
            event_type = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("data: ") {
            data_lines.push(value);
        }
    }

    if data_lines.is_empty() {
        return Ok(None);
    }

    let data = data_lines.join("\n");
    let payload = serde_json::from_str::<Value>(&data)
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

fn extract_start_block(payload: &Value) -> &Value {
    payload.get("content_block").unwrap_or(payload)
}

fn extract_delta_block(payload: &Value) -> &Value {
    payload.get("delta").unwrap_or(payload)
}

/// 处理单个 Anthropic SSE 块，返回 `true` 表示流已结束。
///
/// Anthropic SSE 事件类型分派：
/// - `content_block_start`: 新内容块开始（可能是文本或工具调用）
/// - `content_block_delta`: 增量内容（文本/思考/签名/工具参数）
/// - `message_stop`: 流结束信号，返回 true 通知上层停止读取
/// - `message_start/delta/content_block_stop/ping`: 元数据事件，静默忽略
fn process_sse_block(
    block: &str,
    accumulator: &mut LlmAccumulator,
    sink: &EventSink,
) -> Result<bool> {
    let Some((event_type, payload)) = parse_sse_block(block)? else {
        return Ok(false);
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
            Ok(false)
        }
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
                }
                Some("thinking_delta") => {
                    if let Some(text) = delta.get("thinking").and_then(Value::as_str) {
                        emit_event(LlmEvent::ThinkingDelta(text.to_string()), accumulator, sink);
                    }
                }
                Some("signature_delta") => {
                    if let Some(signature) = delta.get("signature").and_then(Value::as_str) {
                        emit_event(
                            LlmEvent::ThinkingSignature(signature.to_string()),
                            accumulator,
                            sink,
                        );
                    }
                }
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
                }
                _ => {}
            }
            Ok(false)
        }
        "message_stop" => Ok(true),
        // 元数据事件：静默忽略
        "message_start" | "message_delta" | "content_block_stop" | "ping" => Ok(false),
        other => {
            warn!("anthropic: unknown sse event: {}", other);
            Ok(false)
        }
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
) -> Result<bool> {
    sse_buffer.push_str(chunk_text);

    while let Some((block_end, delimiter_len)) = next_sse_block(sse_buffer) {
        let block: String = sse_buffer.drain(..block_end + delimiter_len).collect();
        let block = &block[..block_end];

        if process_sse_block(block, accumulator, sink)? {
            return Ok(true);
        }
    }

    Ok(false)
}

fn flush_sse_buffer(
    sse_buffer: &mut String,
    accumulator: &mut LlmAccumulator,
    sink: &EventSink,
) -> Result<bool> {
    if sse_buffer.trim().is_empty() {
        sse_buffer.clear();
        return Ok(false);
    }

    let done = process_sse_block(sse_buffer, accumulator, sink)?;
    sse_buffer.clear();
    Ok(done)
}

// ---------------------------------------------------------------------------
// Anthropic API 请求/响应 DTO（仅用于 serde 序列化/反序列化）
// ---------------------------------------------------------------------------

/// Anthropic Messages API 请求体。
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
#[derive(Debug, Serialize)]
struct AnthropicThinking {
    #[serde(rename = "type")]
    type_: String,
    budget_tokens: u32,
}

/// Anthropic 消息（包含角色和内容块数组）。
#[derive(Debug, Serialize)]
struct AnthropicMessage {
    role: String,
    content: Vec<AnthropicContentBlock>,
}

/// Anthropic 内容块——消息内容由多个块组成。
///
/// 使用 `#[serde(tag = "type")]` 实现内部标记序列化，
/// 每个变体对应一个 `type` 值（`text`、`thinking`、`tool_use`、`tool_result`）。
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
/// 使用 `Value` 可以灵活处理未知或新增的块类型。
#[derive(Debug, serde::Deserialize)]
struct AnthropicResponse {
    content: Vec<Value>,
    stop_reason: Option<String>,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

/// Anthropic 响应中的 token 用量统计。
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

    fn test_provider() -> AnthropicProvider {
        AnthropicProvider::with_max_tokens(
            "sk-ant-test".to_string(),
            "claude-test".to_string(),
            8096,
        )
    }

    #[test]
    fn to_anthropic_messages_converts_user_assistant_and_tool() {
        let messages = to_anthropic_messages(&[
            LlmMessage::User {
                content: "hello".to_string(),
                origin: UserMessageOrigin::User,
            },
            LlmMessage::Assistant {
                content: "done".to_string(),
                tool_calls: vec![ToolCallRequest {
                    id: "call_1".to_string(),
                    name: "search".to_string(),
                    args: json!({ "q": "rust" }),
                }],
                reasoning: None,
            },
            LlmMessage::Tool {
                tool_call_id: "call_1".to_string(),
                content: "tool output".to_string(),
            },
        ]);

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[2].role, "user");
    }

    #[test]
    fn assistant_blocks_keep_text_before_tool_use() {
        let messages = to_anthropic_messages(&[LlmMessage::Assistant {
            content: "thinking".to_string(),
            tool_calls: vec![ToolCallRequest {
                id: "call_1".to_string(),
                name: "search".to_string(),
                args: json!({ "q": "rust" }),
            }],
            reasoning: None,
        }]);

        match &messages[0].content[..] {
            [AnthropicContentBlock::Text { text, .. }, AnthropicContentBlock::ToolUse {
                id, name, input, ..
            }] => {
                assert_eq!(text, "thinking");
                assert_eq!(id, "call_1");
                assert_eq!(name, "search");
                assert_eq!(*input, json!({ "q": "rust" }));
            }
            _ => panic!("expected text block before tool_use"),
        }
    }

    #[test]
    fn non_streaming_response_parses_text_and_tool_use() {
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
            ],
            stop_reason: Some("tool_use".to_string()),
            usage: None,
        });

        assert_eq!(output.content, "hello world");
        assert_eq!(output.tool_calls.len(), 1);
        assert_eq!(output.tool_calls[0].id, "call_1");
        assert_eq!(output.tool_calls[0].args, json!({ "q": "rust" }));
        assert_eq!(output.reasoning, None);
    }

    #[test]
    fn non_streaming_response_maps_thinking_block() {
        let output = response_to_output(AnthropicResponse {
            content: vec![
                json!({ "type": "thinking", "thinking": "pondering", "signature": "sig-1" }),
                json!({ "type": "text", "text": "done" }),
            ],
            stop_reason: Some("end_turn".to_string()),
            usage: None,
        });

        assert_eq!(output.content, "done");
        assert_eq!(
            output.reasoning,
            Some(ReasoningContent {
                content: "pondering".to_string(),
                signature: Some("sig-1".to_string()),
            })
        );
    }

    #[test]
    fn streaming_content_block_delta_emits_and_accumates_events() {
        let mut accumulator = LlmAccumulator::default();
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = sink_collector(events.clone());
        let mut sse_buffer = String::new();

        let chunk = concat!(
            "event: content_block_start\n",
            "data: {\"index\":1,\"type\":\"tool_use\",\"id\":\"call_1\",\"name\":\"search\"}\n\n",
            "event: content_block_delta\n",
            "data: {\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"q\\\":\\\"ru\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"st\\\"}\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n"
        );

        let done = consume_sse_text_chunk(chunk, &mut sse_buffer, &mut accumulator, &sink)
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
        assert!(events
            .iter()
            .any(|event| matches!(event, LlmEvent::TextDelta(text) if text == "hello")));
        assert_eq!(output.content, "hello");
        assert_eq!(output.tool_calls.len(), 1);
        assert_eq!(output.tool_calls[0].args, json!({ "q": "rust" }));
    }

    #[test]
    fn build_request_serializes_system_when_present() {
        let provider = test_provider();
        let request = provider.build_request(
            &[LlmMessage::User {
                content: "hi".to_string(),
                origin: UserMessageOrigin::User,
            }],
            &[],
            Some("Follow the rules"),
            false,
        );
        let body = serde_json::to_value(&request).expect("request should serialize");

        assert_eq!(
            body.get("system").and_then(Value::as_str),
            Some("Follow the rules")
        );
    }

    #[test]
    fn build_request_omits_system_when_absent() {
        let provider = test_provider();
        let request = provider.build_request(
            &[LlmMessage::User {
                content: "hi".to_string(),
                origin: UserMessageOrigin::User,
            }],
            &[],
            None,
            false,
        );
        let body = serde_json::to_value(&request).expect("request should serialize");

        assert!(body.get("system").is_none());
    }

    #[test]
    fn build_request_serializes_thinking_when_model_supports_it() {
        let provider = AnthropicProvider::with_max_tokens(
            "sk-ant-test".to_string(),
            "claude-sonnet-4-5".to_string(),
            8096,
        );
        let request = provider.build_request(
            &[LlmMessage::User {
                content: "hi".to_string(),
                origin: UserMessageOrigin::User,
            }],
            &[],
            None,
            true,
        );
        let body = serde_json::to_value(&request).expect("request should serialize");

        assert_eq!(
            body.get("thinking")
                .and_then(|value| value.get("type"))
                .and_then(Value::as_str),
            Some("enabled")
        );
        assert!(body
            .get("thinking")
            .and_then(|value| value.get("budget_tokens"))
            .and_then(Value::as_u64)
            .is_some());
    }

    #[test]
    fn retryable_statuses_are_classified() {
        assert!(is_retryable_status(
            reqwest::StatusCode::SERVICE_UNAVAILABLE
        ));
        assert!(is_retryable_status(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR
        ));
        assert!(!is_retryable_status(reqwest::StatusCode::BAD_REQUEST));
    }
}
