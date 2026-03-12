use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use astrcode_core::CancelToken;
use async_trait::async_trait;
use serde_json::Value;

use astrcode_core::{LlmMessage, ToolCallRequest, ToolDefinition};

pub mod anthropic;
pub mod openai;

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
}

#[derive(Clone, Debug)]
pub enum LlmEvent {
    TextDelta(String),
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
}

pub type EventSink = Arc<dyn Fn(LlmEvent) + Send + Sync>;

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn generate(&self, request: LlmRequest, sink: Option<EventSink>) -> Result<LlmOutput>;
}

#[derive(Default)]
pub struct LlmAccumulator {
    pub content: String,
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
