use std::collections::HashMap;
use std::sync::Arc;

use astrcode_core::{AstrError, CancelToken, ModelRequest, ReasoningContent, Result};
use async_trait::async_trait;
use serde_json::Value;
use tokio::{
    select,
    time::{sleep, Duration},
};

use astrcode_core::{LlmMessage, ToolCallRequest, ToolDefinition};

pub mod anthropic;
pub mod openai;

// ---------------------------------------------------------------------------
// Shared constants & helpers used by all LLM providers
// ---------------------------------------------------------------------------

/// TCP connect timeout applied to every outbound HTTP request.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Server-side read timeout – long enough for slow streaming responses but not
/// infinite so we can detect stalled connections.
const READ_TIMEOUT: Duration = Duration::from_secs(90);

/// Maximum number of automatic retries for transient HTTP failures.
const MAX_RETRIES: u32 = 2;

/// Base delay in milliseconds for the first retry.  Each subsequent retry
/// doubles the delay (exponential back-off).
const RETRY_BASE_DELAY_MS: u64 = 250;

/// Build a `reqwest::Client` with the shared connect / read timeout policy.
pub(crate) fn build_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .build()
        .expect("http client should build")
}

/// Classify an HTTP status code as transient / retryable.
///
/// Covers 408 (request timeout), 429 (rate-limit), all 5xx codes, and the
/// common gateway error codes (502, 503, 504).
pub(crate) fn is_retryable_status(status: reqwest::StatusCode) -> bool {
    matches!(
        status,
        reqwest::StatusCode::REQUEST_TIMEOUT
            | reqwest::StatusCode::TOO_MANY_REQUESTS
            | reqwest::StatusCode::BAD_GATEWAY
            | reqwest::StatusCode::SERVICE_UNAVAILABLE
            | reqwest::StatusCode::GATEWAY_TIMEOUT
    ) || status.is_server_error()
}

/// Wait the exponential back-off delay for the given `attempt` index, or abort
/// early when the cancellation token fires.
pub(crate) async fn wait_retry_delay(attempt: u32, cancel: CancelToken) -> Result<()> {
    let delay_ms = RETRY_BASE_DELAY_MS.saturating_mul(1_u64 << attempt);
    select! {
        _ = crate::cancel::cancelled(cancel) => Err(AstrError::LlmInterrupted),
        _ = sleep(Duration::from_millis(delay_ms)) => Ok(()),
    }
}

/// Forward an event to the external sink **and** accumulate it internally.
pub(crate) fn emit_event(event: LlmEvent, accumulator: &mut LlmAccumulator, sink: &EventSink) {
    sink(event.clone());
    accumulator.apply(&event);
}

// ---------------------------------------------------------------------------
// Test helpers (shared across provider test modules)
// ---------------------------------------------------------------------------

/// Create an `EventSink` that records every received event into a
/// `Vec<LlmEvent>` guarded by a `Mutex`.
///
/// Used by unit tests in both `anthropic` and `openai` modules.
#[cfg(test)]
pub(crate) fn sink_collector(events: Arc<std::sync::Mutex<Vec<LlmEvent>>>) -> EventSink {
    Arc::new(move |event| {
        events.lock().expect("lock").push(event);
    })
}

#[derive(Clone, Debug)]
/// Runtime-scoped model call request consumed by the agent loop.
///
/// This request intentionally stops at "messages in, optional system prompt, tool surface, cancel
/// token". Provider discovery, credential resolution, model selection, and failover remain runtime
/// assembly concerns outside of the loop contract.
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
