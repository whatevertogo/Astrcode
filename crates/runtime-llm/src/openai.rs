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
    EventSink, FinishReason, LlmAccumulator, LlmEvent, LlmOutput, LlmProvider, LlmRequest,
    LlmUsage, MAX_RETRIES, ModelLimits, Utf8StreamDecoder, build_http_client, emit_event,
    is_retryable_status, wait_retry_delay,
};

/// OpenAI 兼容 API 的 LLM 提供者实现。
///
/// 封装了 HTTP 客户端、认证信息和模型配置，提供统一的 `LlmProvider` 接口。
#[derive(Clone)]
pub struct OpenAiProvider {
    /// 共享的 HTTP 客户端（含统一超时策略）
    client: reqwest::Client,
    /// 已解析好的 Chat Completions endpoint。
    ///
    /// provider_factory 会先把用户配置标准化到最终请求地址，这里不再二次拼接，
    /// 避免 `baseUrl` 已经包含 `/chat/completions` 时又被重复追加一次。
    chat_completions_api_url: String,
    /// API 密钥（Bearer token 认证）
    api_key: String,
    /// 模型名称（如 `gpt-4o`、`deepseek-chat`）
    model: String,
    /// 运行时已解析好的模型 limits。
    ///
    /// 这样 provider 不再自己猜上下文窗口，也不会继续依赖过时的 profile 级配置。
    limits: ModelLimits,
}

impl fmt::Debug for OpenAiProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiProvider")
            .field("client", &self.client)
            .field("chat_completions_api_url", &self.chat_completions_api_url)
            .field("api_key", &"<redacted>")
            .field("model", &self.model)
            .field("limits", &self.limits)
            .finish()
    }
}

impl OpenAiProvider {
    /// 创建新的 OpenAI 兼容提供者实例。
    pub fn new(
        chat_completions_api_url: String,
        api_key: String,
        model: String,
        limits: ModelLimits,
    ) -> Result<Self> {
        Ok(Self {
            client: build_http_client()?,
            chat_completions_api_url,
            api_key,
            model,
            limits,
        })
    }

    /// 构建 OpenAI Chat Completions API 请求体。
    ///
    /// - 如果存在系统提示块，将每个块作为独立的 `role: "system"` 消息插入
    /// - 如果没有系统提示块但有 system_prompt，使用单一 system 消息
    /// - 将 `LlmMessage` 转换为 OpenAI 格式的消息结构
    /// - 如果启用了工具，附加工具定义和 `tool_choice: "auto"`
    /// - 对 system 消息的层边界和最后 2 条对话消息启用 prompt caching
    fn build_request<'a>(
        &'a self,
        messages: &'a [LlmMessage],
        tools: &'a [ToolDefinition],
        system_prompt: Option<&'a str>,
        system_prompt_blocks: &'a [astrcode_core::SystemPromptBlock],
        stream: bool,
    ) -> OpenAiChatRequest<'a> {
        let system_count = if !system_prompt_blocks.is_empty() {
            system_prompt_blocks.len()
        } else if system_prompt.is_some() {
            1
        } else {
            0
        };
        let mut request_messages = Vec::with_capacity(messages.len() + system_count);

        // 优先使用 system_prompt_blocks（支持分层缓存）
        if !system_prompt_blocks.is_empty() {
            for block in system_prompt_blocks {
                request_messages.push(OpenAiRequestMessage {
                    role: "system".to_string(),
                    content: Some(block.render()),
                    tool_call_id: None,
                    tool_calls: None,
                    // 在层边界标记缓存点，与 Anthropic 保持一致
                    cache_control: if block.cache_boundary {
                        Some(OpenAiCacheControl {
                            type_: "content".to_string(),
                        })
                    } else {
                        None
                    },
                });
            }
        } else if let Some(text) = system_prompt {
            // 回退到单一 system prompt（向后兼容）
            request_messages.push(OpenAiRequestMessage {
                role: "system".to_string(),
                content: Some(text.to_string()),
                tool_call_id: None,
                tool_calls: None,
                cache_control: None,
            });
        }

        request_messages.extend(messages.iter().map(to_openai_message));

        // 对最后 2 条对话消息启用 prompt caching，以便 OpenAI 复用 KV cache
        // 注意：system 消息的缓存已经在上面通过 cache_boundary 标记了
        enable_message_caching(&mut request_messages, 2);

        OpenAiChatRequest {
            model: &self.model,
            max_tokens: self.limits.max_output_tokens.min(u32::MAX as usize) as u32,
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
        for attempt in 0..=MAX_RETRIES {
            let send_future = self
                .client
                .post(&self.chat_completions_api_url)
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
            &request.system_prompt_blocks,
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
                    cache_creation_input_tokens: 0,
                    cache_read_input_tokens: 0,
                });
                let first_choice = parsed.choices.into_iter().next().ok_or_else(|| {
                    AstrError::LlmStreamError(
                        "openai-compatible response did not include choices".to_string(),
                    )
                })?;
                Ok(message_to_output(
                    first_choice.message,
                    usage,
                    first_choice.finish_reason,
                ))
            },
            Some(sink) => {
                // 流式路径：逐块读取 SSE 响应
                let mut body_stream = response.bytes_stream();
                let mut sse_buffer = String::new();
                let mut utf8_decoder = Utf8StreamDecoder::default();
                let mut accumulator = LlmAccumulator::default();
                // 流式路径下从最后一个 chunk 的 finish_reason 提取 (P4.2)
                let mut stream_finish_reason: Option<String> = None;

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
                    let Some(chunk_text) = utf8_decoder.push(
                        &bytes,
                        "openai-compatible response stream was not valid utf-8",
                    )?
                    else {
                        continue;
                    };

                    if consume_sse_text_chunk(
                        &chunk_text,
                        &mut sse_buffer,
                        &mut accumulator,
                        &sink,
                        &mut stream_finish_reason,
                    )? {
                        let mut output = accumulator.finish();
                        // 优先使用 API 返回的 finish_reason，否则使用推断值
                        if let Some(reason) = stream_finish_reason.as_deref() {
                            output.finish_reason = FinishReason::from_api_value(reason);
                        }
                        return Ok(output);
                    }
                }

                if let Some(tail_text) =
                    utf8_decoder.finish("openai-compatible response stream was not valid utf-8")?
                {
                    let done = consume_sse_text_chunk(
                        &tail_text,
                        &mut sse_buffer,
                        &mut accumulator,
                        &sink,
                        &mut stream_finish_reason,
                    )?;
                    if done {
                        let mut output = accumulator.finish();
                        if let Some(reason) = stream_finish_reason.as_deref() {
                            output.finish_reason = FinishReason::from_api_value(reason);
                        }
                        return Ok(output);
                    }
                }

                // 流结束后处理缓冲区中剩余的不完整行
                flush_sse_buffer(
                    &mut sse_buffer,
                    &mut accumulator,
                    &sink,
                    &mut stream_finish_reason,
                )?;
                let mut output = accumulator.finish();
                if let Some(reason) = stream_finish_reason.as_deref() {
                    output.finish_reason = FinishReason::from_api_value(reason);
                }
                Ok(output)
            },
        }
    }

    /// 返回当前模型的上下文窗口估算。
    ///
    /// OpenAI-compatible provider 不再在这里临时猜测 limits，而是直接回放 provider
    /// 构造阶段已经解析好的逐模型配置。
    fn model_limits(&self) -> ModelLimits {
        self.limits
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
/// - `finish_reason` 从响应 choice 中提取，用于检测 max_tokens 截断 (P4.2)
fn message_to_output(
    message: OpenAiResponseMessage,
    usage: Option<LlmUsage>,
    finish_reason: Option<String>,
) -> LlmOutput {
    let content = message.content.unwrap_or_default();
    let tool_calls: Vec<ToolCallRequest> = message
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

    let finish_reason = finish_reason
        .as_deref()
        .map(FinishReason::from_api_value)
        .unwrap_or_else(|| {
            // 无 finish_reason 时根据内容推断
            if !tool_calls.is_empty() {
                FinishReason::ToolCalls
            } else {
                FinishReason::Stop
            }
        });

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
        finish_reason,
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
/// - 空字符串的文本和推理内容会被过滤，避免发射无意义的空增量
/// - 工具调用参数缺失时回退为空字符串，由累加器负责拼接
/// - 返回最后一个非 None 的 finish_reason（P4.2）
fn apply_stream_chunk(chunk: OpenAiStreamChunk) -> (Vec<LlmEvent>, Option<String>) {
    let mut events = Vec::new();
    let mut last_finish_reason: Option<String> = None;

    for choice in chunk.choices {
        // 提取 finish_reason，最后一个非 None 值有效
        if let Some(reason) = choice.finish_reason {
            last_finish_reason = Some(reason);
        }

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

    (events, last_finish_reason)
}

/// 处理单行 SSE 文本，返回 `(is_done, finish_reason)`。
///
/// 这是 SSE 处理链路的中间层：解析行 → 转换 chunk → 发射事件。
fn process_sse_line(
    line: &str,
    accumulator: &mut LlmAccumulator,
    sink: &EventSink,
) -> Result<(bool, Option<String>)> {
    match parse_sse_line(line)? {
        ParsedSseLine::Ignore => Ok((false, None)),
        ParsedSseLine::Done => Ok((true, None)),
        ParsedSseLine::Chunk(chunk) => {
            let (events, finish_reason) = apply_stream_chunk(chunk);
            for event in events {
                emit_event(event, accumulator, sink);
            }
            Ok((false, finish_reason))
        },
    }
}

/// 消费一块 SSE 文本 chunk，按行分割并处理。
///
/// 由于 TCP 流可能将一行 SSE 分割到多个 chunk 中，
/// 本函数使用 `sse_buffer` 累积未完成的行，等待后续 chunk 补齐。
/// 返回 `(is_done, finish_reason)`，is_done 为 true 表示遇到 `[DONE]`，流应停止读取。
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
    finish_reason_out: &mut Option<String>,
) -> Result<bool> {
    sse_buffer.push_str(chunk_text);

    while let Some(newline_idx) = sse_buffer.find('\n') {
        let line_with_newline: String = sse_buffer.drain(..=newline_idx).collect();
        let line = line_with_newline
            .trim_end_matches('\n')
            .trim_end_matches('\r');

        let (done, reason) = process_sse_line(line, accumulator, sink)?;
        if let Some(r) = reason {
            *finish_reason_out = Some(r);
        }
        if done {
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
    finish_reason_out: &mut Option<String>,
) -> Result<()> {
    let remaining = std::mem::take(sse_buffer);
    let remaining = remaining.trim();
    if !remaining.is_empty() {
        let (done, reason) = process_sse_line(remaining, accumulator, sink)?;
        if let Some(r) = reason {
            *finish_reason_out = Some(r);
        }
        // 如果 flush 时遇到 [DONE]，忽略（正常流结束）
        // 故意忽略：消费 done 标志以避免未使用变量警告
        let _ = done;
    }
    Ok(())
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

/// 对最后 `cache_depth` 条消息启用 prompt caching。
///
/// OpenAI 的 prompt caching 通过复用缓存上下文来减少延迟和成本，
/// 当请求前缀匹配时生效。标记尾部消息可以在多轮对话中有效复用历史 KV cache。
///
/// ## 与 Anthropic 的差异
///
/// OpenAI 使用 `prediction: { type: "content" }` 标记可缓存内容，
/// 而 Anthropic 使用 `cache_control: { type: "ephemeral" }`。
///
/// ## 注意
///
/// 此函数会跳过已经有 cache_control 的消息（如 system blocks 的层边界），
/// 避免重复标记。
fn enable_message_caching(messages: &mut [OpenAiRequestMessage], cache_depth: usize) {
    if messages.is_empty() || cache_depth == 0 {
        return;
    }

    let cache_count = cache_depth.min(messages.len());
    let start_idx = messages.len() - cache_count;

    for msg in &mut messages[start_idx..] {
        // 跳过已经有 cache_control 的消息（如 system blocks 的层边界）
        if msg.cache_control.is_none() {
            msg.cache_control = Some(OpenAiCacheControl {
                type_: "content".to_string(),
            });
        }
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
    #[serde(default)]
    finish_reason: Option<String>,
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
    use std::{
        net::TcpListener,
        sync::{Arc, Mutex},
    };

    use astrcode_core::{CancelToken, UserMessageOrigin};
    use serde_json::json;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        task::JoinHandle,
    };

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
            // 故意忽略：读取残余数据仅用于清理，失败无影响
            let _ = socket.read(&mut buf).await;
            socket
                .write_all(response.as_bytes())
                .await
                .expect("response should be written");
            // 故意忽略：关闭 socket 时连接可能已断开
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
            ModelLimits {
                context_window: 128_000,
                max_output_tokens: 2048,
            },
        )
        .expect("provider should build");
        let messages = [LlmMessage::User {
            content: "hi".to_string(),
            origin: UserMessageOrigin::User,
        }];
        let request = provider.build_request(&messages, &[], Some("Follow the rules"), &[], false);

        assert_eq!(request.messages[0].role, "system");
        assert_eq!(
            request.messages[0].content.as_deref(),
            Some("Follow the rules")
        );
    }

    #[test]
    fn build_request_uses_system_blocks_with_layer_boundaries() {
        let provider = OpenAiProvider::new(
            "http://127.0.0.1:12345".to_string(),
            "sk-test".to_string(),
            "model-a".to_string(),
            ModelLimits {
                context_window: 128_000,
                max_output_tokens: 2048,
            },
        )
        .expect("provider should build");
        let messages = [LlmMessage::User {
            content: "hi".to_string(),
            origin: UserMessageOrigin::User,
        }];
        let system_blocks = vec![
            astrcode_core::SystemPromptBlock {
                title: "Stable 1".to_string(),
                content: "stable content 1".to_string(),
                cache_boundary: false,
                layer: astrcode_core::SystemPromptLayer::Stable,
            },
            astrcode_core::SystemPromptBlock {
                title: "Stable 2".to_string(),
                content: "stable content 2".to_string(),
                cache_boundary: true,
                layer: astrcode_core::SystemPromptLayer::Stable,
            },
            astrcode_core::SystemPromptBlock {
                title: "Semi 1".to_string(),
                content: "semi content 1".to_string(),
                cache_boundary: true,
                layer: astrcode_core::SystemPromptLayer::SemiStable,
            },
        ];
        let request = provider.build_request(&messages, &[], None, &system_blocks, false);
        let body = serde_json::to_value(&request).expect("request should serialize");

        // 应该有 3 个 system 消息 + 1 个 user 消息
        assert_eq!(request.messages.len(), 4);
        assert_eq!(request.messages[0].role, "system");
        assert_eq!(request.messages[1].role, "system");
        assert_eq!(request.messages[2].role, "system");
        assert_eq!(request.messages[3].role, "user");

        // 检查缓存边界标记
        assert!(
            body["messages"][0].get("cache_control").is_none(),
            "stable1 should not have cache_control"
        );
        assert_eq!(
            body["messages"][1]["cache_control"]["type"],
            json!("content"),
            "stable2 should have cache_control"
        );
        assert_eq!(
            body["messages"][2]["cache_control"]["type"],
            json!("content"),
            "semi1 should have cache_control"
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
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: \
             {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let (base_url, handle) = spawn_server(response);
        let provider = OpenAiProvider::new(
            base_url,
            "sk-test".to_string(),
            "model-a".to_string(),
            ModelLimits {
                context_window: 128_000,
                max_output_tokens: 2048,
            },
        )
        .expect("provider should build");

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
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: \
             no-cache\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let (base_url, handle) = spawn_server(response);
        let provider = OpenAiProvider::new(
            base_url,
            "sk-test".to_string(),
            "model-a".to_string(),
            ModelLimits {
                context_window: 128_000,
                max_output_tokens: 2048,
            },
        )
        .expect("provider should build");
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

        assert!(
            events
                .iter()
                .any(|event| matches!(event, LlmEvent::TextDelta(text) if text == "hel"))
        );
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

    #[test]
    fn sse_stream_handles_multibyte_text_split_across_chunks() {
        let mut accumulator = LlmAccumulator::default();
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = sink_collector(events.clone());
        let mut sse_buffer = String::new();
        let mut decoder = Utf8StreamDecoder::default();
        let mut finish_reason_out = None;
        let line = r#"data: {"choices":[{"delta":{"content":"你好"},"finish_reason":null}]}"#;
        let bytes = line.as_bytes();
        let split_index = line.find("好").expect("line should contain multibyte char") + 1;

        let first_text = decoder
            .push(
                &bytes[..split_index],
                "openai-compatible response stream was not valid utf-8",
            )
            .expect("first split should decode");
        let second_text = decoder
            .push(
                &bytes[split_index..],
                "openai-compatible response stream was not valid utf-8",
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
                    &mut finish_reason_out,
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
                    &mut finish_reason_out,
                )
            })
            .transpose()
            .expect("second chunk should parse")
            .unwrap_or(false);

        assert!(!first_done);
        assert!(!second_done);

        flush_sse_buffer(
            &mut sse_buffer,
            &mut accumulator,
            &sink,
            &mut finish_reason_out,
        )
        .expect("flush should parse");

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
