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
//! 2. 按 SSE 协议解析出事件块（Anthropic 使用多行 `event:/data:` 格式，
//!    OpenAI 使用单行 `data: {...}` 格式）
//! 3. 每个事件通过 [`emit_event`] 同时发送到外部 `EventSink` 和内部累加器
//! 4. 流结束后，累加器输出包含完整文本、工具调用和推理内容的 [`LlmOutput`]
//!
//! ## 容错与重试
//!
//! 所有提供者内置指数退避重试逻辑：
//! - 可重试状态码：408（超时）、429（限流）、5xx（服务器错误）
//! - 传输层错误（DNS 解析失败、连接断开等）也会重试
//! - 重试期间持续监听 [`CancelToken`]，取消请求会立即中断
//! - 最大重试次数由 `MAX_RETRIES` 常量控制（默认 2 次）
//!
//! ## Prompt Caching
//!
//! 两个提供者均支持 prompt caching 优化：对最后 N 条消息标记缓存控制，
//! 使后端可以复用 KV cache，减少重复上下文的延迟和成本。
//! Anthropic 使用 `ephemeral` 类型，OpenAI 兼容 API 使用 `content` 类型。
//!
//! ## 模块结构
//!
//! - [`anthropic`] — Anthropic Messages API 实现
//! - [`openai`] — OpenAI Chat Completions API 兼容实现

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use astrcode_core::{AstrError, CancelToken, ModelRequest, ReasoningContent, Result};
use async_trait::async_trait;
use serde_json::Value;
use tokio::{select, time::sleep};

use astrcode_core::{LlmMessage, ToolCallRequest, ToolDefinition};

pub mod anthropic;
pub mod openai;

/// 模型能力限制，用于请求预算决策。
///
/// 包含上下文窗口大小和最大输出 token 数，由具体提供者根据模型名称启发式估算。
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

/// TCP 连接超时（10 秒）
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// 读取超时（90 秒）- 足够慢的流式响应，但能检测卡死的连接
const READ_TIMEOUT: Duration = Duration::from_secs(90);

/// 最大自动重试次数（瞬态故障）
const MAX_RETRIES: u32 = 2;

/// 首次重试延迟（毫秒），后续重试指数退避
const RETRY_BASE_DELAY_MS: u64 = 250;

/// 构建共享超时策略的 HTTP 客户端
pub fn build_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .build()
        .expect("http client should build")
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
pub async fn wait_retry_delay(attempt: u32, cancel: CancelToken) -> Result<()> {
    let delay_ms = RETRY_BASE_DELAY_MS.saturating_mul(1_u64 << attempt);
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
    pub tools: Vec<ToolDefinition>,
    pub cancel: CancelToken,
    pub system_prompt: Option<String>,
}

impl LlmRequest {
    pub fn new(messages: Vec<LlmMessage>, tools: Vec<ToolDefinition>, cancel: CancelToken) -> Self {
        Self {
            messages,
            tools,
            cancel,
            system_prompt: None,
        }
    }

    pub fn with_system(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    pub fn from_model_request(request: ModelRequest, cancel: CancelToken) -> Self {
        Self {
            messages: request.messages,
            tools: request.tools,
            cancel,
            system_prompt: request.system_prompt,
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
#[derive(Clone, Debug, Default)]
pub struct LlmOutput {
    pub content: String,
    pub tool_calls: Vec<ToolCallRequest>,
    pub reasoning: Option<ReasoningContent>,
    pub usage: Option<LlmUsage>,
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

    /// 返回模型的上下文窗口估算。
    ///
    /// 用于调用方判断当前消息历史是否接近上下文限制，触发压缩或截断。
    ///
    /// TODO(claude-auto-compact): 当上游 API 暴露精确的上下文窗口元数据时，
    /// 应替换为提供者报告的权威值而非模型名称启发式。
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
            }
            LlmEvent::ThinkingDelta(text) => {
                self.thinking.push_str(text);
            }
            LlmEvent::ThinkingSignature(signature) => {
                self.thinking_signature = Some(signature.clone());
            }
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
            }
        }
    }

    pub fn finish(self) -> LlmOutput {
        let mut entries: Vec<_> = self.tool_calls.into_iter().collect();
        entries.sort_by_key(|(index, _)| *index);

        let tool_calls = entries
            .into_iter()
            .map(|(_, call)| ToolCallRequest {
                id: call.id,
                name: call.name,
                args: serde_json::from_str(&call.arguments)
                    .unwrap_or(Value::String(call.arguments)),
            })
            .collect();

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
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn text_delta_accumulates_content() {
        let mut acc = LlmAccumulator::default();

        acc.apply(&LlmEvent::TextDelta("Hel".to_string()));
        acc.apply(&LlmEvent::TextDelta("lo".to_string()));

        assert_eq!(acc.content, "Hello");
    }

    #[test]
    fn thinking_delta_accumulates_reasoning_content() {
        let mut acc = LlmAccumulator::default();

        acc.apply(&LlmEvent::ThinkingDelta("Hel".to_string()));
        acc.apply(&LlmEvent::ThinkingDelta("lo".to_string()));
        acc.apply(&LlmEvent::ThinkingSignature("sig".to_string()));

        let output = acc.finish();
        assert_eq!(
            output
                .reasoning
                .as_ref()
                .map(|value| value.content.as_str()),
            Some("Hello")
        );
        assert_eq!(
            output
                .reasoning
                .as_ref()
                .and_then(|value| value.signature.as_deref()),
            Some("sig")
        );
    }

    #[test]
    fn tool_call_delta_appends_arguments_across_events() {
        let mut acc = LlmAccumulator::default();

        acc.apply(&LlmEvent::ToolCallDelta {
            index: 1,
            id: Some("call_1".to_string()),
            name: Some("search".to_string()),
            arguments_delta: "{\"q\":\"hel".to_string(),
        });
        acc.apply(&LlmEvent::ToolCallDelta {
            index: 1,
            id: None,
            name: None,
            arguments_delta: "lo\"}".to_string(),
        });

        let output = acc.finish();
        assert_eq!(output.tool_calls.len(), 1);
        assert_eq!(output.tool_calls[0].id, "call_1");
        assert_eq!(output.tool_calls[0].name, "search");
        assert_eq!(output.tool_calls[0].args, json!({ "q": "hello" }));
    }

    #[test]
    fn empty_arguments_delta_does_not_change_result() {
        let mut acc = LlmAccumulator::default();

        acc.apply(&LlmEvent::ToolCallDelta {
            index: 0,
            id: Some("call_1".to_string()),
            name: Some("search".to_string()),
            arguments_delta: String::new(),
        });

        let output = acc.finish();
        assert_eq!(output.tool_calls.len(), 1);
        assert_eq!(output.tool_calls[0].args, Value::String(String::new()));
        assert_eq!(output.reasoning, None);
    }

    #[test]
    fn finish_sorts_by_index_and_parses_json_arguments() {
        let mut acc = LlmAccumulator::default();

        acc.apply(&LlmEvent::ToolCallDelta {
            index: 2,
            id: Some("call_2".to_string()),
            name: Some("second".to_string()),
            arguments_delta: "{\"b\":2}".to_string(),
        });
        acc.apply(&LlmEvent::ToolCallDelta {
            index: 0,
            id: Some("call_0".to_string()),
            name: Some("first".to_string()),
            arguments_delta: "{\"a\":1}".to_string(),
        });

        let output = acc.finish();
        assert_eq!(output.tool_calls.len(), 2);
        assert_eq!(output.tool_calls[0].id, "call_0");
        assert_eq!(output.tool_calls[0].args, json!({ "a": 1 }));
        assert_eq!(output.tool_calls[1].id, "call_2");
        assert_eq!(output.tool_calls[1].args, json!({ "b": 2 }));
    }
}
