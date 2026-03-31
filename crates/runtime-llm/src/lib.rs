//! # LLM 提供者
//!
//! 本模块定义了 LLM 提供者的抽象接口和共享逻辑。
//!
//! ## 架构
//!
//! - `LlmProvider`: LLM 提供者 trait（Anthropic、OpenAI 等）
//! - `LlmRequest`: 模型调用请求
//! - `LlmOutput`: 模型调用响应
//! - `LlmEvent`: 流式事件（文本增量、工具调用增量等）
//! - `LlmAccumulator`: 流式事件累加器

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

// ---------------------------------------------------------------------------
// Cancel helper (moved from runtime::cancel)
// ---------------------------------------------------------------------------

/// Polls the cancel token until it is signalled.
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

/// 转发事件到外部汇并同时累加到内部
pub fn emit_event(event: LlmEvent, accumulator: &mut LlmAccumulator, sink: &EventSink) {
    sink(event.clone());
    accumulator.apply(&event);
}

// ---------------------------------------------------------------------------
// Test helpers (shared across provider test modules)
// ---------------------------------------------------------------------------

/// 创建记录所有事件的 EventSink（用于测试）
#[cfg(test)]
pub fn sink_collector(events: Arc<std::sync::Mutex<Vec<LlmEvent>>>) -> EventSink {
    Arc::new(move |event| {
        events.lock().expect("lock").push(event);
    })
}

/// 运行时范围的模型调用请求
///
/// 限制在"消息、系统提示、工具、取消令牌"，不包含提供者发现和凭据解析。
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

#[derive(Clone, Debug, Default)]
pub struct LlmOutput {
    pub content: String,
    pub tool_calls: Vec<ToolCallRequest>,
    pub reasoning: Option<ReasoningContent>,
}

pub type EventSink = Arc<dyn Fn(LlmEvent) + Send + Sync>;

#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Executes one model call for the current turn.
    ///
    /// This trait is the runtime's concrete model-call port. It does not own provider registry
    /// concerns such as API-key management or model discovery; those are handled before the loop
    /// receives an implementation.
    async fn generate(&self, request: LlmRequest, sink: Option<EventSink>) -> Result<LlmOutput>;
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
                    .unwrap_or_else(|_| Value::String(call.arguments)),
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
