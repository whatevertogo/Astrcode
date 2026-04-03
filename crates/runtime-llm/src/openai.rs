//! # OpenAI 兼容 API 的 LLM 提供者
//!
//! 实现了 `LlmProvider` trait，对接所有兼容 OpenAI Chat Completions API 的后端
//! （包括 OpenAI 自身、DeepSeek、本地 Ollama/vLLM 等）。
//!
//! ## 核心能力
//!
//! - 非流式/流式两种调用模式
//! - SSE 流式解析（`data: {...}` 行协议）
//! - 指数退避重试（瞬态故障自动恢复）
//! - 取消令牌支持（`select!` 分支中断）
//! - Prompt Caching（标记最后 N 条消息以复用 KV cache）
//!
//! ## 协议差异处理
//!
//! OpenAI 兼容 API 的流式响应使用标准的 SSE 格式（`data: {...}` 行），
//! 与 Anthropic 的多行 SSE 块（`event: ...\ndata: {...}\n\n`）不同，
//! 因此本模块有独立的 SSE 解析逻辑。

use std::fmt;

use astrcode_core::{
    AstrError, CancelToken, LlmMessage, ReasoningContent, Result, ToolCallRequest, ToolDefinition,
};
use async_trait::async_trait;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::select;

use crate::{
    build_http_client, emit_event, is_retryable_status, wait_retry_delay, EventSink,
    LlmAccumulator, LlmEvent, LlmOutput, LlmProvider, LlmRequest, LlmUsage, ModelLimits,
    MAX_RETRIES,
};

/// OpenAI 兼容 API 的 LLM 提供者实现。
///
/// 封装了 HTTP 客户端、认证信息和模型配置，提供统一的 `LlmProvider` 接口。
#[derive(Clone)]
pub struct OpenAiProvider {
    /// 共享的 HTTP 客户端（含统一超时策略）
    client: reqwest::Client,
    /// API 基础 URL（如 `https://api.openai.com/v1` 或本地代理地址）
    base_url: String,
    /// API 密钥（Bearer token 认证）
    api_key: String,
    /// 模型名称（如 `gpt-4o`、`deepseek-chat`）
    model: String,
    /// 最大输出 token 数
    max_tokens: u32,
}

impl fmt::Debug for OpenAiProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiProvider")
            .field("client", &self.client)
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .field("model", &self.model)
            .field("max_tokens", &self.max_tokens)
            .finish()
    }
}

impl OpenAiProvider {
    /// 创建新的 OpenAI 兼容提供者实例。
    pub fn new(base_url: String, api_key: String, model: String, max_tokens: u32) -> Self {
        Self {
            client: build_http_client(),
            base_url,
            api_key,
            model,
            max_tokens,
        }
    }

    /// 构建 OpenAI Chat Completions API 请求体。
    ///
    /// - 如果存在系统提示，将其作为 `role: "system"` 消息插入到消息列表最前面
    /// - 将 `LlmMessage` 转换为 OpenAI 格式的消息结构
    /// - 如果启用了工具，附加工具定义和 `tool_choice: "auto"`
    /// - 对最后 2 条消息启用 prompt caching（KV cache 复用）
    fn build_request<'a>(
        &'a self,
        messages: &'a [LlmMessage],
        tools: &'a [ToolDefinition],
        system_prompt: Option<&'a str>,
        stream: bool,
    ) -> OpenAiChatRequest<'a> {
        let mut request_messages =
            Vec::with_capacity(messages.len() + if system_prompt.is_some() { 1 } else { 0 });
        if let Some(text) = system_prompt {
            request_messages.push(OpenAiRequestMessage {
                role: "system".to_string(),
                content: Some(text.to_string()),
                tool_call_id: None,
                tool_calls: None,
                cache_control: None,
            });
        }
        request_messages.extend(messages.iter().map(to_openai_message));

        // 对最后 2 条消息启用 prompt caching，以便 OpenAI 复用 KV cache
        // OpenAI 使用 prediction type "content" 来标记可缓存上下文
        enable_message_caching(&mut request_messages, 2);

        OpenAiChatRequest {
            model: &self.model,
            max_tokens: self.max_tokens,
            messages: request_messages,
            tools: if tools.is_empty() {
                None
            } else {
                Some(tools.iter().map(to_openai_tool).collect())
            },
            tool_choice: if tools.is_empty() { None } else { Some("auto") },
            stream,
        }
    }

    /// 发送 HTTP 请求并处理响应。
    ///
    /// 内置指数退避重试逻辑：
    /// - 可重试的 HTTP 状态码（408/429/5xx）和传输层错误会自动重试
    /// - 重试期间监听取消令牌，一旦取消立即中断
    /// - 非重试错误（如 400/401/403）直接返回
    async fn send_request(
        &self,
        req: &OpenAiChatRequest<'_>,
        cancel: CancelToken,
    ) -> Result<reqwest::Response> {
        let endpoint = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));

        for attempt in 0..=MAX_RETRIES {
            let send_future = self
                .client
                .post(&endpoint)
                .bearer_auth(&self.api_key)
                .json(req)
                .send();

            let response = select! {
                _ = crate::cancelled(cancel.clone()) => {
                    return Err(AstrError::LlmInterrupted);
                }
                result = send_future => result
                    .map_err(|error| AstrError::http("failed to call openai-compatible endpoint", error))
            };

            match response {
                Ok(response) => {
                    let status = response.status();
                    if status.is_success() {
                        return Ok(response);
                    }

                    let body = response.text().await.unwrap_or_default();
                    if is_retryable_status(status) && attempt < MAX_RETRIES {
                        wait_retry_delay(attempt, cancel.clone()).await?;
                        continue;
                    }

                    return Err(AstrError::LlmRequestFailed {
                        status: status.as_u16(),
                        body,
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

        Err(AstrError::Network(
            "openai-compatible request failed after retries".to_string(),
        ))
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    /// 执行一次模型调用。
    ///
    /// 根据 `sink` 是否存在选择流式或非流式路径：
    /// - **非流式**（`sink = None`）：等待完整响应后解析 JSON，提取文本和工具调用
    /// - **流式**（`sink = Some`）：逐块读取 SSE 响应，实时发射事件并累加
    async fn generate(&self, request: LlmRequest, sink: Option<EventSink>) -> Result<LlmOutput> {
        let cancel = request.cancel;
        let req = self.build_request(
            &request.messages,
            &request.tools,
            request.system_prompt.as_deref(),
            sink.is_some(),
        );
        let response = self.send_request(&req, cancel.clone()).await?;

        match sink {
            None => {
                // 非流式路径：解析完整 JSON 响应
                let parsed: OpenAiChatResponse = response.json().await.map_err(|error| {
                    AstrError::http("failed to parse openai-compatible response", error)
                })?;
                let usage = parsed.usage.as_ref().map(|usage| LlmUsage {
                    input_tokens: usage.prompt_tokens.unwrap_or_default() as usize,
                    output_tokens: usage.completion_tokens.unwrap_or_default() as usize,
                });
                let first_choice = parsed.choices.into_iter().next().ok_or_else(|| {
                    AstrError::LlmStreamError(
                        "openai-compatible response did not include choices".to_string(),
                    )
                })?;
                Ok(message_to_output(first_choice.message, usage))
            }
            Some(sink) => {
                // 流式路径：逐块读取 SSE 响应
                let mut body_stream = response.bytes_stream();
                let mut sse_buffer = String::new();
                let mut accumulator = LlmAccumulator::default();

                loop {
                    let next_item = select! {
                        _ = crate::cancelled(cancel.clone()) => {
                            return Err(AstrError::LlmInterrupted);
                        }
                        item = body_stream.next() => item,
                    };

                    let Some(item) = next_item else {
                        break;
                    };

                    let bytes = item.map_err(|error| {
                        AstrError::http("failed to read openai-compatible response stream", error)
                    })?;
                    let chunk_text = std::str::from_utf8(&bytes).map_err(|error| {
                        AstrError::from(error)
                            .context("openai-compatible response stream was not valid utf-8")
                    })?;

                    if consume_sse_text_chunk(chunk_text, &mut sse_buffer, &mut accumulator, &sink)?
                    {
                        return Ok(accumulator.finish());
                    }
                }

                // 流结束后处理缓冲区中剩余的不完整行
                flush_sse_buffer(&mut sse_buffer, &mut accumulator, &sink)?;
                Ok(accumulator.finish())
            }
        }
    }

    /// 返回当前模型的上下文窗口估算。
    ///
    /// 基于模型名称的启发式匹配，当前所有已知模型族统一返回 128k。
    ///
    /// TODO(claude-auto-compact): 当 OpenAI 兼容后端暴露权威的上下文窗口信息时，
    /// 应替换为提供者/模型元数据而非模型名称启发式。
    fn model_limits(&self) -> ModelLimits {
        ModelLimits {
            context_window: estimate_openai_context_window(&self.model),
            max_output_tokens: self.max_tokens as usize,
        }
    }
}

/// 将 OpenAI 响应消息转换为统一的 `LlmOutput`。
///
/// 处理文本内容、工具调用和推理内容（`reasoning_content` 字段，
/// 部分兼容 API 使用 `reasoning` 别名）。
///
/// ## 设计要点
///
/// - 工具调用参数可能不是合法 JSON，解析失败时回退为原始字符串
/// - 推理内容为空字符串时不保留（避免无意义的空 reasoning 对象）
/// - `usage` 参数在非流式路径下由调用方传入，流式路径下为 `None`
fn message_to_output(message: OpenAiResponseMessage, usage: Option<LlmUsage>) -> LlmOutput {
    let content = message.content.unwrap_or_default();
    let tool_calls = message
        .tool_calls
        .unwrap_or_default()
        .into_iter()
        .map(|call| ToolCallRequest {
            id: call.id,
            name: call.function.name,
            // NOTE: 参数可能不是合法 JSON，解析失败时回退为原始字符串
            args: serde_json::from_str::<Value>(&call.function.arguments)
                .unwrap_or(Value::String(call.function.arguments)),
        })
        .collect();

    LlmOutput {
        content,
        tool_calls,
        // 推理内容为空字符串时不保留
        reasoning: message
            .reasoning_content
            .filter(|value| !value.is_empty())
            .map(|content| ReasoningContent {
                content,
                signature: None,
            }),
        usage,
    }
}

/// SSE 行解析结果。
///
/// OpenAI 兼容 API 的 SSE 格式为单行 `data: {...}`，每行独立一个 JSON chunk。
/// 与 Anthropic 的多行 SSE 块不同，OpenAI 格式更简单：每行以 `data: ` 开头，
/// 流结束由特殊的 `data: [DONE]` 标记。
enum ParsedSseLine {
    /// 空行或无 data 前缀的行，应忽略
    Ignore,
    /// `[DONE]` 标记，表示流结束
    Done,
    /// 解析出的流式 chunk
    Chunk(OpenAiStreamChunk),
}

/// 解析单行 SSE 文本。
///
/// 期望格式：`data: <json>` 或 `data: [DONE]`。
/// 空行或不带 `data: ` 前缀的行返回 `Ignore`。
///
/// ## 错误处理
///
/// JSON 解析失败会返回 `AstrError::Parse` 错误，这通常意味着后端响应格式异常。
fn parse_sse_line(line: &str) -> Result<ParsedSseLine> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(ParsedSseLine::Ignore);
    }

    let Some(after_prefix) = trimmed.strip_prefix("data:") else {
        return Ok(ParsedSseLine::Ignore);
    };
    let data = after_prefix.trim_start();

    if data == "[DONE]" {
        return Ok(ParsedSseLine::Done);
    }

    let chunk = serde_json::from_str::<OpenAiStreamChunk>(data)
        .map_err(|error| AstrError::parse("failed to parse streaming chunk", error))?;
    Ok(ParsedSseLine::Chunk(chunk))
}

/// 将 OpenAI 流式 chunk 转换为 `LlmEvent` 列表。
///
/// 每个 choice 的 delta 可能包含文本、推理内容或工具调用增量。
///
/// ## 设计要点
///
/// - `finish_reason` 字段当前未使用，流结束由 `[DONE]` 标记判断
/// - 空字符串的文本和推理内容会被过滤，避免发射无意义的空增量
/// - 工具调用参数缺失时回退为空字符串，由累加器负责拼接
fn apply_stream_chunk(chunk: OpenAiStreamChunk) -> Vec<LlmEvent> {
    let mut events = Vec::new();

    for choice in chunk.choices {
        // NOTE: finish_reason 当前未使用，流结束由 `[DONE]` 标记判断

        if let Some(content) = choice.delta.content {
            if !content.is_empty() {
                events.push(LlmEvent::TextDelta(content));
            }
        }

        if let Some(reasoning_content) = choice.delta.reasoning_content {
            if !reasoning_content.is_empty() {
                events.push(LlmEvent::ThinkingDelta(reasoning_content));
            }
        }

        if let Some(tool_calls) = choice.delta.tool_calls {
            for tool_call in tool_calls {
                let (name, arguments_delta) = match tool_call.function {
                    Some(function) => (function.name, function.arguments.unwrap_or_default()),
                    None => (None, String::new()),
                };

                events.push(LlmEvent::ToolCallDelta {
                    index: tool_call.index,
                    id: tool_call.id,
                    name,
                    arguments_delta,
                });
            }
        }
    }

    events
}

/// 处理单行 SSE 文本，返回 `true` 表示流已结束（遇到 `[DONE]`）。
///
/// 这是 SSE 处理链路的中间层：解析行 → 转换 chunk → 发射事件。
fn process_sse_line(
    line: &str,
    accumulator: &mut LlmAccumulator,
    sink: &EventSink,
) -> Result<bool> {
    match parse_sse_line(line)? {
        ParsedSseLine::Ignore => Ok(false),
        ParsedSseLine::Done => Ok(true),
        ParsedSseLine::Chunk(chunk) => {
            for event in apply_stream_chunk(chunk) {
                emit_event(event, accumulator, sink);
            }
            Ok(false)
        }
    }
}

/// 消费一块 SSE 文本 chunk，按行分割并处理。
///
/// 由于 TCP 流可能将一行 SSE 分割到多个 chunk 中，
/// 本函数使用 `sse_buffer` 累积未完成的行，等待后续 chunk 补齐。
/// 返回 `true` 表示遇到 `[DONE]`，流应停止读取。
///
/// ## TCP 分片处理
///
/// TCP 是字节流协议，不保证消息边界。一个完整的 SSE 行可能被分成多个 TCP chunk，
/// 因此不能假设每个 `chunk_text` 包含完整的 `data: {...}` 行。
fn consume_sse_text_chunk(
    chunk_text: &str,
    sse_buffer: &mut String,
    accumulator: &mut LlmAccumulator,
    sink: &EventSink,
) -> Result<bool> {
    sse_buffer.push_str(chunk_text);

    while let Some(newline_idx) = sse_buffer.find('\n') {
        let line_with_newline: String = sse_buffer.drain(..=newline_idx).collect();
        let line = line_with_newline
            .trim_end_matches('\n')
            .trim_end_matches('\r');

        if process_sse_line(line, accumulator, sink)? {
            return Ok(true);
        }
    }

    Ok(false)
}

/// 刷新 SSE 缓冲区中剩余的不完整行（流结束后的收尾处理）。
///
/// 当 HTTP 流结束时，缓冲区中可能还剩一行没有换行符。
/// 本函数处理这最后一行并清空缓冲区。
fn flush_sse_buffer(
    sse_buffer: &mut String,
    accumulator: &mut LlmAccumulator,
    sink: &EventSink,
) -> Result<bool> {
    if sse_buffer.is_empty() {
        return Ok(false);
    }

    let line = sse_buffer.trim_end_matches('\r');
    let done = process_sse_line(line, accumulator, sink)?;
    sse_buffer.clear();
    Ok(done)
}

/// 将 `ToolDefinition` 转换为 OpenAI 工具定义格式。
///
/// OpenAI 工具定义需要 `type: "function"` 包装层，
/// 内部包含 `name`、`description`、`parameters`（JSON Schema）。
fn to_openai_tool(def: &ToolDefinition) -> OpenAiTool {
    OpenAiTool {
        tool_type: "function".to_string(),
        function: OpenAiToolFunction {
            name: def.name.clone(),
            description: def.description.clone(),
            parameters: def.parameters.clone(),
        },
    }
}

/// 将 `LlmMessage` 转换为 OpenAI 请求消息格式。
///
/// - User 消息 → `role: "user"`
/// - Assistant 消息 → `role: "assistant"`（包含 tool_calls 和可选 content）
/// - Tool 消息 → `role: "tool"`（携带 tool_call_id 关联结果）
///
/// ## 设计要点
///
/// - Assistant 消息的 `reasoning` 字段当前不转换（OpenAI 兼容 API 不标准支持）
/// - 空内容的 assistant 消息将 content 设为 `None` 而非空字符串
fn to_openai_message(message: &LlmMessage) -> OpenAiRequestMessage {
    match message {
        LlmMessage::User { content, .. } => OpenAiRequestMessage {
            role: "user".to_string(),
            content: Some(content.clone()),
            tool_call_id: None,
            tool_calls: None,
            cache_control: None,
        },
        LlmMessage::Assistant {
            content,
            tool_calls,
            reasoning: _,
        } => OpenAiRequestMessage {
            role: "assistant".to_string(),
            content: if content.is_empty() {
                None
            } else {
                Some(content.clone())
            },
            tool_call_id: None,
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(
                    tool_calls
                        .iter()
                        .map(|call| OpenAiToolCall {
                            id: call.id.clone(),
                            tool_type: "function".to_string(),
                            function: OpenAiToolCallFunction {
                                name: call.name.clone(),
                                arguments: call.args.to_string(),
                            },
                        })
                        .collect(),
                )
            },
            cache_control: None,
        },
        LlmMessage::Tool {
            tool_call_id,
            content,
        } => OpenAiRequestMessage {
            role: "tool".to_string(),
            content: Some(content.clone()),
            tool_call_id: Some(tool_call_id.clone()),
            tool_calls: None,
            cache_control: None,
        },
    }
}

/// 估算 OpenAI 兼容模型的上下文窗口大小。
///
/// 当前所有已知模型族统一返回 128k 保守默认值。
/// 当 OpenAI 兼容后端暴露精确的上下文窗口元数据时应替换为提供者报告的值。
fn estimate_openai_context_window(_model: &str) -> usize {
    128_000
}

/// 对最后 `cache_depth` 条消息启用 prompt caching。
///
/// OpenAI 的 prompt caching 通过复用缓存上下文来减少延迟和成本，
/// 当请求前缀匹配时生效。标记尾部消息可以在多轮对话中有效复用历史 KV cache。
///
/// ## 与 Anthropic 的差异
///
/// OpenAI 使用 `prediction: { type: "content" }` 标记可缓存内容，
/// 而 Anthropic 使用 `cache_control: { type: "ephemeral" }`。
fn enable_message_caching(messages: &mut [OpenAiRequestMessage], cache_depth: usize) {
    if messages.is_empty() || cache_depth == 0 {
        return;
    }

    let cache_count = cache_depth.min(messages.len());
    let start_idx = messages.len() - cache_count;

    for msg in &mut messages[start_idx..] {
        msg.cache_control = Some(OpenAiCacheControl {
            type_: "content".to_string(),
        });
    }
}

// ---------------------------------------------------------------------------
// OpenAI API 请求/响应 DTO（仅用于 serde 序列化/反序列化）
// ---------------------------------------------------------------------------

/// OpenAI Chat Completions API 请求体。
///
/// 使用生命周期 `'a` 借用模型名称和工具选择字符串，
/// 避免不必要的字符串克隆。`stream` 字段为 `bool`（非 `Option`），
/// 因为 OpenAI API 始终需要该字段。
#[derive(Debug, Serialize)]
struct OpenAiChatRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    messages: Vec<OpenAiRequestMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OpenAiTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'a str>,
    stream: bool,
}

/// OpenAI 请求消息（user / assistant / system / tool）。
///
/// 与 Anthropic 的内容块数组不同，OpenAI 使用扁平的消息结构：
/// - `content`: 纯文本内容（assistant 消息可为空）
/// - `tool_calls`: 工具调用列表（仅 assistant 消息使用）
/// - `tool_call_id`: 关联的工具调用 ID（仅 tool 消息使用）
/// - `cache_control`: prompt caching 标记（可选）
#[derive(Debug, Serialize)]
struct OpenAiRequestMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenAiToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<OpenAiCacheControl>,
}

/// OpenAI prompt caching 控制标记。
///
/// `type: "content"` 告诉 OpenAI 后端该消息的内容可作为缓存前缀的一部分。
/// 与 Anthropic 的 `ephemeral` 不同，OpenAI 使用 `content` 作为 prediction type。
#[derive(Debug, Clone, Serialize)]
struct OpenAiCacheControl {
    #[serde(rename = "type")]
    type_: String,
}

/// OpenAI 工具定义（用于请求体中的 `tools` 字段）。
///
/// OpenAI 工具定义需要 `type: "function"` 包装层，
/// 这是 OpenAI API 的固定约定，当前不支持其他工具类型。
#[derive(Debug, Serialize)]
struct OpenAiTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: OpenAiToolFunction,
}

/// OpenAI 工具的函数定义。
///
/// `parameters` 是 JSON Schema 对象，描述工具参数的类型和约束。
#[derive(Debug, Serialize)]
struct OpenAiToolFunction {
    name: String,
    description: String,
    parameters: Value,
}

/// OpenAI 响应中的工具调用（请求体中 assistant 消息的 `tool_calls` 字段）。
///
/// 注意：这是请求体中的结构（序列化），与响应体中的 `OpenAiResponseToolCall` 不同。
#[derive(Debug, Serialize)]
struct OpenAiToolCall {
    id: String,
    #[serde(rename = "type")]
    tool_type: String,
    function: OpenAiToolCallFunction,
}

/// OpenAI 工具调用的函数部分（请求体中）。
///
/// `arguments` 为 JSON 字符串（已序列化），而非 `Value` 对象，
/// 因为 OpenAI API 期望接收字符串形式的 JSON。
#[derive(Debug, Serialize)]
struct OpenAiToolCallFunction {
    name: String,
    arguments: String,
}

/// OpenAI Chat Completions API 非流式响应体。
///
/// 包含 `choices` 数组（通常只有一个元素）和可选的 `usage` 统计。
#[derive(Debug, Deserialize)]
struct OpenAiChatResponse {
    choices: Vec<OpenAiChoice>,
    #[serde(default)]
    usage: Option<OpenAiUsage>,
}

/// OpenAI 响应中的单个 choice。
///
/// 非流式响应中通常只有一个 choice，流式响应中每个 chunk 也包含一个 choice。
#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: OpenAiResponseMessage,
}

/// OpenAI 响应消息（从 choice 中提取）。
///
/// `reasoning_content` 字段通过 `#[serde(alias = "reasoning")]` 兼容
/// 部分 API 后端使用 `reasoning` 作为字段名的情况。
#[derive(Debug, Deserialize)]
struct OpenAiResponseMessage {
    content: Option<String>,
    /// 推理内容，部分兼容 API 使用 `reasoning` 字段名（通过 `alias` 兼容）。
    #[serde(alias = "reasoning")]
    reasoning_content: Option<String>,
    tool_calls: Option<Vec<OpenAiResponseToolCall>>,
}

/// OpenAI 响应中的 token 用量统计。
///
/// 两个字段均为 `Option` 且带 `#[serde(default)]`，
/// 因为某些兼容 API 可能不返回用量信息。
#[derive(Debug, Deserialize)]
struct OpenAiUsage {
    #[serde(default)]
    prompt_tokens: Option<u64>,
    #[serde(default)]
    completion_tokens: Option<u64>,
}

/// OpenAI 响应中的工具调用（响应体中）。
///
/// 与请求体中的 `OpenAiToolCall` 不同，响应体中的工具调用不包含 `type` 字段。
#[derive(Debug, Deserialize)]
struct OpenAiResponseToolCall {
    id: String,
    function: OpenAiResponseToolFunction,
}

/// OpenAI 响应中工具调用的函数部分。
///
/// `arguments` 为 JSON 字符串（未解析），调用方需要自行反序列化。
#[derive(Debug, Deserialize)]
struct OpenAiResponseToolFunction {
    name: String,
    arguments: String,
}

/// OpenAI 流式响应中的单个 chunk（对应一行 `data: {...}`）。
///
/// 每个 chunk 包含 `choices` 数组，每个 choice 的 delta 包含增量内容。
#[derive(Debug, Deserialize)]
struct OpenAiStreamChunk {
    choices: Vec<OpenAiStreamChoice>,
}

/// OpenAI 流式 chunk 中的单个 choice。
///
/// `finish_reason` 保留以兼容 API 响应结构，但当前流结束判断由 `[DONE]` 标记决定。
#[derive(Debug, Deserialize)]
struct OpenAiStreamChoice {
    delta: OpenAiStreamDelta,
    // 保留以兼容 API 响应结构，当前流结束判断由 `[DONE]` 标记决定
    #[allow(dead_code)]
    finish_reason: Option<String>,
}

/// OpenAI 流式 delta（增量内容）。
///
/// `reasoning_content` 同样通过 `alias` 兼容 `reasoning` 字段名。
#[derive(Debug, Deserialize)]
struct OpenAiStreamDelta {
    content: Option<String>,
    /// 推理内容增量，部分兼容 API 使用 `reasoning` 字段名。
    #[serde(alias = "reasoning")]
    reasoning_content: Option<String>,
    tool_calls: Option<Vec<OpenAiStreamToolCall>>,
}

/// OpenAI 流式响应中的工具调用增量。
///
/// 流式工具调用分多个 chunk 到达：
/// - 首个 chunk 包含 `id` 和 `function.name`
/// - 后续 chunk 只包含 `function.arguments` 的片段
#[derive(Debug, Deserialize)]
struct OpenAiStreamToolCall {
    index: usize,
    id: Option<String>,
    function: Option<OpenAiStreamToolCallFunction>,
}

/// OpenAI 流式工具调用的函数增量部分。
///
/// `name` 和 `arguments` 均为 `Option`，因为不同 chunk 中可能只出现其中一个。
#[derive(Debug, Deserialize)]
struct OpenAiStreamToolCallFunction {
    name: Option<String>,
    arguments: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    use astrcode_core::{CancelToken, UserMessageOrigin};
    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::task::JoinHandle;

    use super::*;
    use crate::sink_collector;

    fn spawn_server(response: String) -> (String, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let addr = listener.local_addr().expect("listener should have addr");
        listener
            .set_nonblocking(true)
            .expect("listener should be nonblocking");
        let listener = tokio::net::TcpListener::from_std(listener).expect("tokio listener");

        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept should work");
            let mut buf = [0_u8; 4096];
            let _ = socket.read(&mut buf).await;
            socket
                .write_all(response.as_bytes())
                .await
                .expect("response should be written");
            let _ = socket.shutdown().await;
        });

        (format!("http://{}", addr), handle)
    }

    #[test]
    fn sse_line_parser_handles_done_and_data_prefix_variants() {
        assert!(matches!(
            parse_sse_line("data: [DONE]").expect("should parse"),
            ParsedSseLine::Done
        ));
        assert!(matches!(
            parse_sse_line("data:[DONE]").expect("should parse"),
            ParsedSseLine::Done
        ));
        assert!(matches!(
            parse_sse_line("   ").expect("should parse"),
            ParsedSseLine::Ignore
        ));

        let parsed = parse_sse_line(
            r#"data: {"choices":[{"delta":{"content":"Hello"},"finish_reason":null}]}"#,
        )
        .expect("should parse");
        assert!(matches!(parsed, ParsedSseLine::Chunk(_)));
    }

    #[test]
    fn build_request_prepends_system_message_when_present() {
        let provider = OpenAiProvider::new(
            "http://127.0.0.1:12345".to_string(),
            "sk-test".to_string(),
            "model-a".to_string(),
            2048,
        );
        let messages = [LlmMessage::User {
            content: "hi".to_string(),
            origin: UserMessageOrigin::User,
        }];
        let request = provider.build_request(&messages, &[], Some("Follow the rules"), false);

        assert_eq!(request.messages[0].role, "system");
        assert_eq!(
            request.messages[0].content.as_deref(),
            Some("Follow the rules")
        );
    }

    #[tokio::test]
    async fn generate_non_streaming_parses_text_and_tool_calls() {
        let body = json!({
            "choices": [{
                "message": {
                    "content": "hello",
                    "tool_calls": [{
                        "id": "call_1",
                        "function": {
                            "name": "search",
                            "arguments": "{\"q\":\"hello\"}"
                        }
                    }]
                }
            }]
        })
        .to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let (base_url, handle) = spawn_server(response);
        let provider =
            OpenAiProvider::new(base_url, "sk-test".to_string(), "model-a".to_string(), 2048);

        let output = provider
            .generate(
                LlmRequest::new(
                    vec![LlmMessage::User {
                        content: "hi".to_string(),
                        origin: UserMessageOrigin::User,
                    }],
                    vec![],
                    CancelToken::new(),
                ),
                None,
            )
            .await
            .expect("generate should succeed");

        handle.await.expect("server should join");
        assert_eq!(output.content, "hello");
        assert_eq!(output.tool_calls.len(), 1);
        assert_eq!(output.tool_calls[0].name, "search");
    }

    #[tokio::test]
    async fn generate_streaming_emits_events_and_accumulates_output() {
        let body = format!(
            "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
            json!({
                "choices": [{
                    "delta": { "content": "hel" },
                    "finish_reason": null
                }]
            }),
            json!({
                "choices": [{
                    "delta": {
                        "content": "lo",
                        "tool_calls": [{
                            "index": 0,
                            "id": "call_1",
                            "function": {
                                "name": "search",
                                "arguments": "{\"q\":\"hello\"}"
                            }
                        }]
                    },
                    "finish_reason": "stop"
                }]
            })
        );
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let (base_url, handle) = spawn_server(response);
        let provider =
            OpenAiProvider::new(base_url, "sk-test".to_string(), "model-a".to_string(), 2048);
        let events = Arc::new(Mutex::new(Vec::new()));

        let output = provider
            .generate(
                LlmRequest::new(
                    vec![LlmMessage::User {
                        content: "hi".to_string(),
                        origin: UserMessageOrigin::User,
                    }],
                    vec![],
                    CancelToken::new(),
                ),
                Some(sink_collector(events.clone())),
            )
            .await
            .expect("generate should succeed");

        handle.await.expect("server should join");
        let events = events.lock().expect("lock").clone();

        assert!(events
            .iter()
            .any(|event| matches!(event, LlmEvent::TextDelta(text) if text == "hel")));
        assert!(events.iter().any(|event| {
            matches!(
                event,
                LlmEvent::ToolCallDelta { index, id, name, arguments_delta }
                if *index == 0
                    && id.as_deref() == Some("call_1")
                    && name.as_deref() == Some("search")
                    && arguments_delta == "{\"q\":\"hello\"}"
            )
        }));
        assert_eq!(output.content, "hello");
        assert_eq!(output.tool_calls.len(), 1);
        assert_eq!(output.tool_calls[0].args, json!({ "q": "hello" }));
    }
}
