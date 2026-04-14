//! # LLM 提供者运行时
//!
//! 本 crate 实现了对多种 LLM API 后端的统一抽象，包括 Anthropic Claude 和所有兼容
//! OpenAI Chat Completions API 的服务（如 OpenAI 自身、DeepSeek、本地 Ollama/vLLM 等）。
//!
//! ## 架构设计
//!
//! 核心是 [`LlmProvider`] trait，它定义了运行时与 LLM 后端交互的最小契约：
//! - `generate()` 执行一次模型调用，支持流式和非流式两种模式
//! - `model_limits()` 返回模型的上下文窗口和最大输出 token 估算
//!
//! 各提供者实现（`anthropic::AnthropicProvider`、`openai::OpenAiProvider`）封装了
//! 各自的协议细节，对外暴露统一的接口。
//!
//! ## 流式处理模型
//!
//! 流式响应通过 SSE（Server-Sent Events）协议传输，本 crate 使用 [`LlmAccumulator`]
//! 将增量事件重新组装为完整的 [`LlmOutput`]：
//! 1. HTTP 响应流逐块读取字节
//! 2. 按 SSE 协议解析出事件块（Anthropic 使用多行 `event:/data:` 格式， OpenAI 使用单行 `data:
//!    {...}` 格式）
//! 3. 每个事件通过 [`emit_event`] 同时发送到外部 `EventSink` 和内部累加器
//! 4. 流结束后，累加器输出包含完整文本、工具调用和推理内容的 [`LlmOutput`]
//!
//! ## 容错与重试
//!
//! 所有提供者内置指数退避重试逻辑：
//! - 可重试状态码：408（超时）、429（限流）、5xx（服务器错误）
//! - 传输层错误（DNS 解析失败、连接断开等）也会重试
//! - 重试期间持续监听 [`CancelToken`]，取消请求会立即中断
//! - 最大重试次数由运行时 `LlmClientConfig` 控制（默认 2 次）
//!
//! ## Prompt Caching
//!
//! Anthropic 支持显式 prompt caching：对选定消息标记 `ephemeral` 类型缓存控制，
//! 使后端可以复用 KV cache，减少重复上下文的延迟和成本。
//! OpenAI 兼容 API 目前依赖自动前缀缓存（prefix caching），不发送显式缓存控制头。
//!
//! ## 模块结构
//!
//! - [`anthropic`] — Anthropic Messages API 实现
//! - [`openai`] — OpenAI Chat Completions API 兼容实现

use std::{collections::HashMap, sync::Arc, time::Duration};

use astrcode_core::{
    AstrError, CancelToken, LlmMessage, ModelRequest, ReasoningContent, Result, SystemPromptBlock,
    ToolCallRequest, ToolDefinition,
};
use async_trait::async_trait;
use log::warn;
use serde_json::Value;
use tokio::{select, time::sleep};

pub mod anthropic;
pub mod cache_tracker;
pub mod core_port;
pub mod openai;

// ---------------------------------------------------------------------------
// Structured LLM error types (P4.3)
// ---------------------------------------------------------------------------

/// 结构化的 LLM 错误分类，用于 turn 级别的错误恢复决策。
///
/// 替代原先基于字符串匹配的 `is_prompt_too_long()`，让上层能够
/// 通过类型匹配精确判断错误性质并采取对应恢复策略。
#[derive(Debug, Clone)]
pub enum LlmError {
    /// Prompt 超出模型上下文窗口 (HTTP 400/413)
    PromptTooLong { status: u16, body: String },
    /// 其他不可重试的客户端错误 (4xx, 非 413)
    ClientError { status: u16, body: String },
    /// 服务端错误 (5xx)
    ServerError { status: u16, body: String },
    /// 传输层错误 (DNS 失败、连接断开等)
    Transport(String),
    /// 请求被取消
    Interrupted,
    /// 流解析错误 (SSE 协议解析失败、JSON 无效等)
    StreamParse(String),
}

impl LlmError {
    /// 判断是否为 prompt too long 错误。
    pub fn is_prompt_too_long(&self) -> bool {
        matches!(self, LlmError::PromptTooLong { .. })
    }

    /// 判断是否为可恢复的错误 (prompt too long 可触发 compact).
    pub fn is_recoverable(&self) -> bool {
        self.is_prompt_too_long()
    }
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmError::PromptTooLong { status, body } => {
                write!(f, "prompt too long (HTTP {status}): {body}")
            },
            LlmError::ClientError { status, body } => {
                write!(f, "client error (HTTP {status}): {body}")
            },
            LlmError::ServerError { status, body } => {
                write!(f, "server error (HTTP {status}): {body}")
            },
            LlmError::Transport(msg) => write!(f, "transport error: {msg}"),
            LlmError::Interrupted => write!(f, "LLM request interrupted"),
            LlmError::StreamParse(msg) => write!(f, "stream parse error: {msg}"),
        }
    }
}

impl From<LlmError> for AstrError {
    fn from(err: LlmError) -> Self {
        match err {
            LlmError::PromptTooLong { status, body } => {
                AstrError::LlmRequestFailed { status, body }
            },
            LlmError::ClientError { status, body } => AstrError::LlmRequestFailed { status, body },
            LlmError::ServerError { status, body } => AstrError::LlmRequestFailed { status, body },
            LlmError::Transport(msg) => AstrError::Network(msg),
            LlmError::Interrupted => AstrError::LlmInterrupted,
            LlmError::StreamParse(msg) => AstrError::LlmStreamError(msg),
        }
    }
}

/// 从 HTTP 响应状态和 body 中分类 LLM 错误。
///
/// 优先匹配 prompt too long 特征，其次按状态码范围分类。
pub fn classify_http_error(status: u16, body: &str) -> LlmError {
    let body_lower = body.to_ascii_lowercase();
    let is_context_exceeded = body_lower.contains("prompt too long")
        || body_lower.contains("context length")
        || body_lower.contains("maximum context")
        || body_lower.contains("too many tokens");

    if is_context_exceeded && matches!(status, 400 | 413) {
        return LlmError::PromptTooLong {
            status,
            body: body.to_string(),
        };
    }

    if status < 500 {
        LlmError::ClientError {
            status,
            body: body.to_string(),
        }
    } else {
        LlmError::ServerError {
            status,
            body: body.to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Finish reason (P4.2)
// ---------------------------------------------------------------------------

/// LLM 响应结束原因，用于判断是否需要自动继续生成。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FinishReason {
    /// 模型自然结束输出
    #[default]
    Stop,
    /// 输出被 max_tokens 限制截断，需要继续生成
    MaxTokens,
    /// 模型调用了工具
    ToolCalls,
    /// 其他未知原因
    Other(String),
}

impl FinishReason {
    /// 判断是否因 max_tokens 截断。
    pub fn is_max_tokens(&self) -> bool {
        matches!(self, FinishReason::MaxTokens)
    }

    /// 从 OpenAI/Anthropic API 返回的 finish_reason/stop_reason 字符串解析。
    ///
    /// 支持两种 API 的值：
    /// - OpenAI: `stop`, `max_tokens`, `length`, `tool_calls`, `content_filter`
    /// - Anthropic: `end_turn`, `max_tokens`, `tool_use`, `stop_sequence`
    pub fn from_api_value(value: &str) -> Self {
        match value {
            // OpenAI 值
            "stop" => FinishReason::Stop,
            "max_tokens" | "length" => FinishReason::MaxTokens,
            "tool_calls" => FinishReason::ToolCalls,
            // Anthropic 值
            "end_turn" | "stop_sequence" => FinishReason::Stop,
            "tool_use" => FinishReason::ToolCalls,
            other => FinishReason::Other(other.to_string()),
        }
    }
}

/// 模型能力限制，用于请求预算决策。
///
/// 包含上下文窗口大小和最大输出 token 数。它们在 provider 构造阶段就已经被解析为权威值
/// 或本地手动值，后续的 agent loop 只消费这一份统一结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelLimits {
    /// 模型支持的上下文窗口大小（token 数）
    pub context_window: usize,
    /// 模型单次响应允许的最大输出 token 数
    pub max_output_tokens: usize,
}

/// 模型调用的 token 用量统计。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LlmUsage {
    /// 输入（prompt）消耗的 token 数
    pub input_tokens: usize,
    /// 输出（completion）消耗的 token 数
    pub output_tokens: usize,
    /// 本次请求中新写入 provider cache 的输入 token 数。
    pub cache_creation_input_tokens: usize,
    /// 本次请求从 provider cache 读取的输入 token 数。
    pub cache_read_input_tokens: usize,
}

impl LlmUsage {
    pub fn total_tokens(self) -> usize {
        self.input_tokens.saturating_add(self.output_tokens)
    }
}

// ---------------------------------------------------------------------------
// Cancel helper (moved from runtime::cancel)
// ---------------------------------------------------------------------------

/// 轮询取消令牌直到被标记为已取消。
///
/// 用于 `select!` 分支中监听取消信号，每 25ms 检查一次状态。
pub async fn cancelled(cancel: CancelToken) {
    while !cancel.is_cancelled() {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

// ---------------------------------------------------------------------------
// Shared constants & helpers used by all LLM providers
// ---------------------------------------------------------------------------

/// LLM HTTP 客户端配置。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LlmClientConfig {
    /// TCP 连接超时。
    pub connect_timeout: Duration,
    /// 读取超时。
    pub read_timeout: Duration,
    /// 最大自动重试次数（瞬态故障）。
    pub max_retries: u32,
    /// 首次重试延迟，后续重试指数退避。
    pub retry_base_delay: Duration,
}

impl Default for LlmClientConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            read_timeout: Duration::from_secs(90),
            max_retries: 2,
            retry_base_delay: Duration::from_millis(250),
        }
    }
}

/// 构建共享超时策略的 HTTP 客户端。
///
/// 不在库层 panic，统一返回 `AstrError` 交由上层决定是降级、重试还是失败。
pub fn build_http_client(config: LlmClientConfig) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(config.connect_timeout)
        .read_timeout(config.read_timeout)
        .build()
        .map_err(|error| AstrError::http("failed to build shared http client", error))
}

/// 判断 HTTP 状态码是否可重试
///
/// 包括 408（超时）、429（限流）、所有 5xx 和网关错误。
pub fn is_retryable_status(status: reqwest::StatusCode) -> bool {
    matches!(
        status,
        reqwest::StatusCode::REQUEST_TIMEOUT
            | reqwest::StatusCode::TOO_MANY_REQUESTS
            | reqwest::StatusCode::BAD_GATEWAY
            | reqwest::StatusCode::SERVICE_UNAVAILABLE
            | reqwest::StatusCode::GATEWAY_TIMEOUT
    ) || status.is_server_error()
}

/// 等待指数退避延迟（或被取消）
pub async fn wait_retry_delay(
    attempt: u32,
    cancel: CancelToken,
    retry_base_delay: Duration,
) -> Result<()> {
    let delay_ms = (retry_base_delay.as_millis().min(u64::MAX as u128) as u64)
        .saturating_mul(1_u64 << attempt);
    select! {
        _ = cancelled(cancel) => Err(AstrError::LlmInterrupted),
        _ = sleep(Duration::from_millis(delay_ms)) => Ok(()),
    }
}

/// 转发事件到外部汇并同时累加到内部。
///
/// 这是流式处理的核心函数：每个事件既发送给外部消费者（用于实时 UI 更新），
/// 也累加到内部状态（用于流结束后组装完整响应）。
pub fn emit_event(event: LlmEvent, accumulator: &mut LlmAccumulator, sink: &EventSink) {
    sink(event.clone());
    accumulator.apply(&event);
}

/// 增量 UTF-8 流式解码器。
///
/// HTTP/SSE 是按字节块返回的，TCP 分片可能把一个多字节字符拆到两个 chunk 里。
/// 如果直接对每个 chunk 调 `from_utf8`，遇到中文等非 ASCII 内容就会误报 UTF-8 错误。
/// 这里保留尾部不完整字节，等下一个 chunk 到达后再继续解码。
#[derive(Debug, Default)]
pub struct Utf8StreamDecoder {
    pending: Vec<u8>,
}

impl Utf8StreamDecoder {
    /// 追加一个新的字节块，并返回当前已经确认完整的 UTF-8 文本。
    pub fn push(&mut self, chunk: &[u8], context: &str) -> Result<Option<String>> {
        if chunk.is_empty() {
            return Ok(None);
        }

        self.pending.extend_from_slice(chunk);
        self.decode_available(context)
    }

    /// 在流结束时刷新尾部缓冲。
    ///
    /// 流结束时也做容错恢复：如果尾部是损坏/不完整 UTF-8，替换为 U+FFFD 并继续。
    /// 这样可以避免单个网关脏字节导致整轮会话失败。
    pub fn finish(&mut self, context: &str) -> Result<Option<String>> {
        if self.pending.is_empty() {
            return Ok(None);
        }

        let mut decoded = String::new();

        loop {
            match std::str::from_utf8(&self.pending) {
                Ok(text) => {
                    decoded.push_str(text);
                    self.pending.clear();
                    break;
                },
                Err(error) => {
                    let valid_up_to = error.valid_up_to();
                    if valid_up_to > 0 {
                        let valid_prefix = std::str::from_utf8(&self.pending[..valid_up_to])
                            .expect("valid_up_to should always point to a valid utf-8 prefix");
                        decoded.push_str(valid_prefix);
                    }

                    if let Some(invalid_len) = error.error_len() {
                        warn!(
                            "stream decoder recovered invalid utf-8 sequence at stream end in {}: \
                             valid_up_to={}, invalid_len={}, bytes={}",
                            context,
                            valid_up_to,
                            invalid_len,
                            debug_utf8_bytes(&self.pending, valid_up_to, Some(invalid_len))
                        );
                        decoded.push(char::REPLACEMENT_CHARACTER);
                        self.pending.drain(..valid_up_to + invalid_len);
                        if self.pending.is_empty() {
                            break;
                        }
                    } else {
                        // `error_len == None` 表示尾部是"可能缺失字节"的不完整序列。
                        // 流已经结束，不会再有后续字节，因此直接用替换符收尾并清空缓存。
                        warn!(
                            "stream decoder recovered incomplete utf-8 tail at stream end in {}: \
                             valid_up_to={}, bytes={}",
                            context,
                            valid_up_to,
                            debug_utf8_bytes(&self.pending, valid_up_to, None)
                        );
                        decoded.push(char::REPLACEMENT_CHARACTER);
                        self.pending.clear();
                        break;
                    }
                },
            }
        }

        Ok((!decoded.is_empty()).then_some(decoded))
    }

    fn decode_available(&mut self, context: &str) -> Result<Option<String>> {
        let mut decoded = String::new();

        loop {
            match std::str::from_utf8(&self.pending) {
                Ok(text) => {
                    decoded.push_str(text);
                    self.pending.clear();
                    return Ok((!decoded.is_empty()).then_some(decoded));
                },
                Err(error) => {
                    let valid_up_to = error.valid_up_to();
                    if valid_up_to > 0 {
                        let valid_prefix = std::str::from_utf8(&self.pending[..valid_up_to])
                            .expect("valid_up_to should always point to a valid utf-8 prefix");
                        decoded.push_str(valid_prefix);
                    }

                    let Some(invalid_len) = error.error_len() else {
                        if decoded.is_empty() {
                            return Ok(None);
                        }

                        // 只消费已经确认完整的前缀，把尾部不完整字符留给下一个 chunk。
                        let tail = self.pending.split_off(valid_up_to);
                        self.pending = tail;
                        return Ok(Some(decoded));
                    };

                    warn!(
                        "stream decoder recovered invalid utf-8 sequence in {}: valid_up_to={}, \
                         invalid_len={}, bytes={}",
                        context,
                        valid_up_to,
                        invalid_len,
                        debug_utf8_bytes(&self.pending, valid_up_to, Some(invalid_len))
                    );

                    // 某些第三方网关会在 SSE 文本中混入坏字节。这里把坏字节替换为 U+FFFD，
                    // 继续保住整轮输出，而不是因为单个脏字节直接终止会话。
                    decoded.push(char::REPLACEMENT_CHARACTER);
                    self.pending.drain(..valid_up_to + invalid_len);
                    if self.pending.is_empty() {
                        return Ok(Some(decoded));
                    }
                },
            }
        }
    }
}

fn debug_utf8_bytes(bytes: &[u8], valid_up_to: usize, invalid_len: Option<usize>) -> String {
    let start = valid_up_to.saturating_sub(8);
    let end = invalid_len
        .map(|len| (valid_up_to + len + 8).min(bytes.len()))
        .unwrap_or(bytes.len().min(valid_up_to + 8));

    bytes[start..end]
        .iter()
        .enumerate()
        .map(|(index, byte)| {
            let absolute_index = start + index;
            if absolute_index == valid_up_to {
                format!("[{byte:02X}]")
            } else {
                format!("{byte:02X}")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------------------------------------------------------------------------
// Test helpers (shared across provider test modules)
// ---------------------------------------------------------------------------

/// 创建记录所有事件的 EventSink（用于测试）。
///
/// 返回的 sink 会将每个事件追加到提供的 `Mutex<Vec<LlmEvent>>` 中，
/// 方便测试断言验证事件序列。
#[cfg(test)]
pub fn sink_collector(events: Arc<std::sync::Mutex<Vec<LlmEvent>>>) -> EventSink {
    Arc::new(move |event| {
        events.lock().expect("lock").push(event);
    })
}

/// 运行时范围的模型调用请求。
///
/// 封装了模型调用所需的最小上下文：消息历史、可用工具定义、取消令牌和可选的系统提示。
/// 不包含提供者发现、API 密钥管理等前置逻辑，这些由调用方在构造本结构体之前处理。
#[derive(Clone, Debug)]
pub struct LlmRequest {
    pub messages: Vec<LlmMessage>,
    pub tools: Arc<[ToolDefinition]>,
    pub cancel: CancelToken,
    pub system_prompt: Option<String>,
    pub system_prompt_blocks: Vec<SystemPromptBlock>,
}

impl LlmRequest {
    pub fn new(
        messages: Vec<LlmMessage>,
        tools: impl Into<Arc<[ToolDefinition]>>,
        cancel: CancelToken,
    ) -> Self {
        Self {
            messages,
            tools: tools.into(),
            cancel,
            system_prompt: None,
            system_prompt_blocks: Vec::new(),
        }
    }

    pub fn with_system(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    pub fn from_model_request(request: ModelRequest, cancel: CancelToken) -> Self {
        Self {
            messages: request.messages,
            tools: request.tools.into(),
            cancel,
            system_prompt: request.system_prompt,
            system_prompt_blocks: request.system_prompt_blocks,
        }
    }
}

/// 流式 LLM 响应事件。
///
/// 每个事件代表响应流中的一个增量片段，由 [`LlmAccumulator`] 重新组装为完整输出。
/// - `TextDelta`: 普通文本增量
/// - `ThinkingDelta`: 推理过程增量（extended thinking / reasoning）
/// - `ThinkingSignature`: 推理签名（Anthropic 特有，用于验证 thinking 完整性）
/// - `ToolCallDelta`: 工具调用增量（id、name 在首个 delta 中出现，arguments 逐片段拼接）
#[derive(Clone, Debug)]
pub enum LlmEvent {
    TextDelta(String),
    ThinkingDelta(String),
    ThinkingSignature(String),
    ToolCallDelta {
        index: usize,
        id: Option<String>,
        name: Option<String>,
        arguments_delta: String,
    },
}

/// 模型调用的完整输出。
///
/// 由 [`LlmAccumulator::finish`] 组装而成，包含所有文本内容、工具调用请求和推理内容。
/// 非流式路径下，`usage` 字段会被填充；流式路径下 `usage` 为 `None`（Anthropic 流式
/// 响应不返回用量，OpenAI 流式响应的用量在最后一个 chunk 中但当前未提取）。
///
/// `finish_reason` 字段用于判断输出是否被 max_tokens 截断 (P4.2)。
#[derive(Clone, Debug, Default)]
pub struct LlmOutput {
    pub content: String,
    pub tool_calls: Vec<ToolCallRequest>,
    pub reasoning: Option<ReasoningContent>,
    pub usage: Option<LlmUsage>,
    /// 输出结束原因，用于检测 max_tokens 截断。
    pub finish_reason: FinishReason,
}

/// 事件回调类型别名。
///
/// 用于接收流式 [`LlmEvent`] 的异步回调，通常由前端或上层运行时订阅。
pub type EventSink = Arc<dyn Fn(LlmEvent) + Send + Sync>;

/// LLM 提供者 trait。
///
/// 这是运行时与 LLM 后端交互的核心抽象。每个实现封装了特定 API 的协议细节
/// （认证、请求格式、SSE 解析等），对外暴露统一的调用接口。
///
/// ## 设计约束
///
/// - 不管理 API 密钥或模型发现，这些由调用方在构造具体提供者实例时处理
/// - `generate()` 执行单次模型调用，不维护多轮对话状态
/// - 流式路径下，事件通过 `sink` 实时发射，同时内部累加返回完整输出
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// 执行一次模型调用。
    ///
    /// `sink` 参数控制调用模式：
    /// - `None`: 非流式模式，等待完整响应后返回
    /// - `Some(sink)`: 流式模式，实时发射 [`LlmEvent`] 到 sink，同时累加返回完整输出
    ///
    /// 取消令牌通过 `request.cancel` 传递，任何时刻取消都会立即中断请求。
    async fn generate(&self, request: LlmRequest, sink: Option<EventSink>) -> Result<LlmOutput>;

    /// 当前 provider 是否原生暴露缓存 token 指标。
    ///
    /// OpenAI 兼容接口目前只依赖自动前缀缓存，缺少稳定的 token 统计；Anthropic 则会明确
    /// 返回 cache creation/read 字段。上层通过这个开关决定是否把 0 值解释成“真实指标”
    /// 还是“provider 不支持”。
    fn supports_cache_metrics(&self) -> bool {
        false
    }

    /// 将 provider 原始 usage 里的输入 token 规范化成适合前端展示的 prompt 总量。
    ///
    /// 某些 provider（如 Anthropic）会把缓存读取的 token 单独放在
    /// `cache_read_input_tokens`，而 `input_tokens` 只表示本次实际重新发送/计费的部分。
    /// 前端展示“缓存命中率”时需要一个统一口径的总输入值，因此默认直接回放
    /// `usage.input_tokens`，特殊 provider 再自行覆盖。
    fn prompt_metrics_input_tokens(&self, usage: LlmUsage) -> usize {
        usage.input_tokens
    }

    /// 返回模型的上下文窗口估算。
    ///
    /// 用于调用方判断当前消息历史是否接近上下文限制，触发压缩或截断。
    ///
    /// 返回值应已经是 provider 构造阶段解析好的稳定 limits，而不是在这里临时猜测。
    fn model_limits(&self) -> ModelLimits;
}

#[derive(Default)]
pub struct LlmAccumulator {
    pub content: String,
    thinking: String,
    thinking_signature: Option<String>,
    tool_calls: HashMap<usize, AccToolCall>,
}

#[derive(Default)]
pub struct AccToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

impl LlmAccumulator {
    pub fn apply(&mut self, event: &LlmEvent) {
        match event {
            LlmEvent::TextDelta(text) => {
                self.content.push_str(text);
            },
            LlmEvent::ThinkingDelta(text) => {
                self.thinking.push_str(text);
            },
            LlmEvent::ThinkingSignature(signature) => {
                self.thinking_signature = Some(signature.clone());
            },
            LlmEvent::ToolCallDelta {
                index,
                id,
                name,
                arguments_delta,
            } => {
                let entry = self.tool_calls.entry(*index).or_default();
                if let Some(value) = id {
                    entry.id = value.clone();
                }
                if let Some(value) = name {
                    entry.name = value.clone();
                }
                entry.arguments.push_str(arguments_delta);
            },
        }
    }

    pub fn finish(self) -> LlmOutput {
        let mut entries: Vec<_> = self.tool_calls.into_iter().collect();
        entries.sort_by_key(|(index, _)| *index);

        let tool_calls: Vec<ToolCallRequest> = entries
            .into_iter()
            .map(|(_, call)| {
                let args = match serde_json::from_str(&call.arguments) {
                    Ok(value) => value,
                    Err(error) => {
                        // JSON 解析失败时降级为原始字符串，并记录警告日志
                        // 这通常意味着 LLM 返回了格式错误的工具参数
                        warn!(
                            "failed to parse tool call '{}' arguments as JSON: {}, falling back \
                             to raw string",
                            call.name, error
                        );
                        Value::String(call.arguments)
                    },
                };
                ToolCallRequest {
                    id: call.id,
                    name: call.name,
                    args,
                }
            })
            .collect();

        // 根据是否有工具调用推断 finish_reason（流式路径下 API 不显式返回）
        let finish_reason = if !tool_calls.is_empty() {
            FinishReason::ToolCalls
        } else {
            FinishReason::Stop
        };

        LlmOutput {
            content: self.content,
            tool_calls,
            reasoning: if self.thinking.is_empty() {
                None
            } else {
                Some(ReasoningContent {
                    content: self.thinking,
                    signature: self.thinking_signature,
                })
            },
            usage: None,
            finish_reason,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn accumulator_handles_text_thinking_and_tool_calls() {
        let mut acc = LlmAccumulator::default();

        acc.apply(&LlmEvent::TextDelta("Hel".to_string()));
        acc.apply(&LlmEvent::TextDelta("lo".to_string()));
        acc.apply(&LlmEvent::ThinkingDelta("reasoning".to_string()));
        acc.apply(&LlmEvent::ThinkingSignature("sig".to_string()));
        acc.apply(&LlmEvent::ToolCallDelta {
            index: 1,
            id: Some("call_1".to_string()),
            name: Some("search".to_string()),
            arguments_delta: "{\"q\":\"hello\"}".to_string(),
        });
        acc.apply(&LlmEvent::ToolCallDelta {
            index: 0,
            id: Some("call_0".to_string()),
            name: Some("other".to_string()),
            arguments_delta: "{\"a\":1}".to_string(),
        });

        let output = acc.finish();
        assert_eq!(output.content, "Hello");
        assert_eq!(
            output.reasoning.as_ref().map(|r| r.content.as_str()),
            Some("reasoning")
        );
        assert_eq!(
            output
                .reasoning
                .as_ref()
                .and_then(|r| r.signature.as_deref()),
            Some("sig")
        );
        assert_eq!(output.tool_calls.len(), 2);
        assert_eq!(output.tool_calls[0].id, "call_0");
        assert_eq!(output.tool_calls[0].args, json!({ "a": 1 }));
        assert_eq!(output.tool_calls[1].id, "call_1");
        assert_eq!(output.tool_calls[1].args, json!({ "q": "hello" }));
    }

    // -----------------------------------------------------------------------
    // P4.3: LlmError classification tests
    // -----------------------------------------------------------------------

    #[test]
    fn llm_error_detects_prompt_too_long_413() {
        let error = classify_http_error(413, "prompt too long for this model");
        assert!(error.is_prompt_too_long());
        assert!(error.is_recoverable());
    }

    #[test]
    fn llm_error_detects_prompt_too_long_400() {
        let error = classify_http_error(400, "context length exceeded");
        assert!(error.is_prompt_too_long());
        assert!(error.is_recoverable());
    }

    #[test]
    fn llm_error_detects_maximum_context() {
        let error = classify_http_error(413, "maximum context length reached");
        assert!(error.is_prompt_too_long());
    }

    #[test]
    fn llm_error_detects_too_many_tokens() {
        let error = classify_http_error(400, "too many tokens in request");
        assert!(error.is_prompt_too_long());
    }

    #[test]
    fn llm_error_classifies_client_errors() {
        let error = classify_http_error(401, "invalid api key");
        assert!(!error.is_prompt_too_long());
        assert!(!error.is_recoverable());
        matches!(error, LlmError::ClientError { status: 401, .. });
    }

    #[test]
    fn llm_error_classifies_server_errors() {
        let error = classify_http_error(500, "internal server error");
        assert!(!error.is_prompt_too_long());
        assert!(!error.is_recoverable());
        matches!(error, LlmError::ServerError { status: 500, .. });
    }

    #[test]
    fn llm_error_display_formats_correctly() {
        let error = LlmError::PromptTooLong {
            status: 413,
            body: "prompt too long".to_string(),
        };
        let display = format!("{error}");
        assert!(display.contains("413"));
        assert!(display.contains("prompt too long"));
    }

    #[test]
    fn llm_error_converts_to_astr_error() {
        let llm_error = LlmError::PromptTooLong {
            status: 413,
            body: "context length exceeded".to_string(),
        };
        let astr_error: AstrError = llm_error.into();
        matches!(astr_error, AstrError::LlmRequestFailed { status: 413, .. });
    }

    // -----------------------------------------------------------------------
    // P4.2: FinishReason tests
    // -----------------------------------------------------------------------

    #[test]
    fn finish_reason_parses_openai_values() {
        assert_eq!(FinishReason::from_api_value("stop"), FinishReason::Stop);
        assert_eq!(
            FinishReason::from_api_value("max_tokens"),
            FinishReason::MaxTokens
        );
        assert_eq!(
            FinishReason::from_api_value("tool_calls"),
            FinishReason::ToolCalls
        );
        assert_eq!(
            FinishReason::from_api_value("length"),
            FinishReason::MaxTokens
        );
        assert_eq!(
            FinishReason::from_api_value("content_filter"),
            FinishReason::Other("content_filter".to_string())
        );
    }

    #[test]
    fn finish_reason_parses_anthropic_values() {
        assert_eq!(FinishReason::from_api_value("end_turn"), FinishReason::Stop);
        assert_eq!(
            FinishReason::from_api_value("max_tokens"),
            FinishReason::MaxTokens
        );
        assert_eq!(
            FinishReason::from_api_value("tool_use"),
            FinishReason::ToolCalls
        );
        assert_eq!(
            FinishReason::from_api_value("stop_sequence"),
            FinishReason::Stop
        );
    }

    #[test]
    fn finish_reason_is_max_tokens_detects_correctly() {
        assert!(FinishReason::MaxTokens.is_max_tokens());
        assert!(!FinishReason::Stop.is_max_tokens());
        assert!(!FinishReason::ToolCalls.is_max_tokens());
    }

    #[test]
    fn utf8_stream_decoder_handles_multibyte_char_split_across_chunks() {
        let mut decoder = Utf8StreamDecoder::default();
        let bytes = "你好".as_bytes();

        let first = decoder
            .push(&bytes[..4], "test utf-8 stream")
            .expect("first chunk should parse");
        let second = decoder
            .push(&bytes[4..], "test utf-8 stream")
            .expect("second chunk should parse");
        let tail = decoder
            .finish("test utf-8 stream")
            .expect("finish should parse");

        assert_eq!(first.as_deref(), Some("你"));
        assert_eq!(second.as_deref(), Some("好"));
        assert_eq!(tail, None);
    }

    #[test]
    fn utf8_stream_decoder_rejects_invalid_utf8_sequences() {
        let mut decoder = Utf8StreamDecoder::default();
        let decoded = decoder
            .push(&[0xFF], "test utf-8 stream")
            .expect("invalid utf-8 should be recovered");

        assert_eq!(decoded.as_deref(), Some("\u{FFFD}"));
    }

    #[test]
    fn utf8_stream_decoder_keeps_valid_suffix_after_invalid_bytes() {
        let mut decoder = Utf8StreamDecoder::default();
        let decoded = decoder
            .push(&[b'a', 0xFF, b'b'], "test utf-8 stream")
            .expect("invalid utf-8 should be recovered");

        assert_eq!(decoded.as_deref(), Some("a\u{FFFD}b"));
    }

    #[test]
    fn utf8_stream_decoder_finish_recovers_incomplete_trailing_sequence() {
        let mut decoder = Utf8StreamDecoder::default();
        let first = decoder
            .push(&[b'a', 0xE4, 0xBD], "test utf-8 stream")
            .expect("partial utf-8 should be buffered");
        assert_eq!(first.as_deref(), Some("a"));

        let tail = decoder
            .finish("test utf-8 stream")
            .expect("finish should recover incomplete trailing utf-8");

        assert_eq!(tail.as_deref(), Some("\u{FFFD}"));
    }

    #[test]
    fn debug_utf8_bytes_marks_failure_boundary() {
        let snippet = debug_utf8_bytes(&[0x61, 0x62, 0xFF, 0x63], 2, Some(1));
        assert_eq!(snippet, "61 62 [FF] 63");
    }
}
