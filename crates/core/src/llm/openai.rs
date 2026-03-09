use std::collections::HashMap;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::select;
use tokio_util::sync::CancellationToken;

use crate::action::{LlmMessage, LlmResponse, ToolCallRequest, ToolDefinition};
use crate::llm::{DeltaCallback, LlmProvider};

#[derive(Clone, Debug)]
pub struct OpenAiProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
}

impl OpenAiProvider {
    pub fn new(base_url: String, api_key: String, model: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url,
            api_key,
            model,
        }
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn complete(
        &self,
        messages: &[LlmMessage],
        tools: &[ToolDefinition],
        cancel: CancellationToken,
    ) -> Result<LlmResponse> {
        let request = OpenAiChatRequest {
            model: &self.model,
            messages: messages.iter().map(to_openai_message).collect(),
            tools: if tools.is_empty() {
                None
            } else {
                Some(tools.iter().map(to_openai_tool).collect())
            },
            tool_choice: if tools.is_empty() { None } else { Some("auto") },
            stream: false,
        };

        let endpoint = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let send_future = self
            .client
            .post(endpoint)
            .bearer_auth(&self.api_key)
            .json(&request)
            .send();

        let response = select! {
            _ = cancel.cancelled() => {
                return Err(anyhow!("llm request interrupted"));
            }
            result = send_future => result.context("failed to call openai-compatible endpoint")?
        };

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "openai-compatible request failed with {}: {}",
                status,
                body
            ));
        }

        let parsed: OpenAiChatResponse = response
            .json()
            .await
            .context("failed to parse openai-compatible response")?;
        let first_choice = parsed
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("openai-compatible response did not include choices"))?;

        let message = first_choice.message;
        let content = message.content.unwrap_or_default();
        let tool_calls = message
            .tool_calls
            .unwrap_or_default()
            .into_iter()
            .map(|call| ToolCallRequest {
                id: call.id,
                name: call.function.name,
                args: serde_json::from_str::<Value>(&call.function.arguments)
                    .unwrap_or_else(|_| Value::String(call.function.arguments)),
            })
            .collect();

        Ok(LlmResponse {
            content,
            tool_calls,
        })
    }

    async fn stream_complete(
        &self,
        messages: &[LlmMessage],
        tools: &[ToolDefinition],
        cancel: CancellationToken,
        on_delta: DeltaCallback,
    ) -> Result<LlmResponse> {
        let request = OpenAiChatRequest {
            model: &self.model,
            messages: messages.iter().map(to_openai_message).collect(),
            tools: if tools.is_empty() {
                None
            } else {
                Some(tools.iter().map(to_openai_tool).collect())
            },
            tool_choice: if tools.is_empty() { None } else { Some("auto") },
            stream: true,
        };

        let endpoint = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let send_future = self
            .client
            .post(endpoint)
            .bearer_auth(&self.api_key)
            .json(&request)
            .send();

        let response = select! {
            _ = cancel.cancelled() => {
                return Err(anyhow!("llm request interrupted"));
            }
            result = send_future => result.context("failed to call openai-compatible endpoint")?
        };

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "openai-compatible request failed with {}: {}",
                status,
                body
            ));
        }

        let mut body_stream = response.bytes_stream();
        let mut sse_buffer = String::new();
        let mut full_content = String::new();
        let mut tool_calls_acc: HashMap<usize, AccToolCall> = HashMap::new();

        loop {
            if cancel.is_cancelled() {
                return Err(anyhow!("llm request interrupted"));
            }

            let next_item = select! {
                _ = cancel.cancelled() => {
                    return Err(anyhow!("llm request interrupted"));
                }
                item = body_stream.next() => item,
            };

            let Some(item) = next_item else {
                break;
            };

            let bytes = item.context("failed to read openai-compatible response stream")?;
            let chunk_text = std::str::from_utf8(&bytes)
                .context("openai-compatible response stream was not valid utf-8")?;

            if consume_sse_text_chunk(
                chunk_text,
                &mut sse_buffer,
                &mut full_content,
                &mut tool_calls_acc,
                &on_delta,
            )? {
                return Ok(LlmResponse {
                    content: full_content,
                    tool_calls: to_tool_call_requests(tool_calls_acc),
                });
            }
        }

        flush_sse_buffer(
            &mut sse_buffer,
            &mut full_content,
            &mut tool_calls_acc,
            &on_delta,
        )?;

        Ok(LlmResponse {
            content: full_content,
            tool_calls: to_tool_call_requests(tool_calls_acc),
        })
    }
}

#[derive(Debug, Default)]
struct AccToolCall {
    id: String,
    name: String,
    arguments: String,
}

enum ParsedSseLine {
    Ignore,
    Done,
    Chunk(OpenAiStreamChunk),
}

fn parse_sse_line(line: &str) -> Result<ParsedSseLine> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(ParsedSseLine::Ignore);
    }

    let Some(data) = trimmed.strip_prefix("data: ") else {
        return Ok(ParsedSseLine::Ignore);
    };

    if data == "[DONE]" {
        return Ok(ParsedSseLine::Done);
    }

    let chunk = serde_json::from_str::<OpenAiStreamChunk>(data)
        .context("failed to parse streaming chunk")?;
    Ok(ParsedSseLine::Chunk(chunk))
}

fn apply_stream_chunk(
    chunk: OpenAiStreamChunk,
    full_content: &mut String,
    tool_calls_acc: &mut HashMap<usize, AccToolCall>,
    on_delta: &DeltaCallback,
) {
    for choice in chunk.choices {
        let _ = choice.finish_reason;

        if let Some(content) = choice.delta.content {
            if !content.is_empty() {
                if let Ok(mut cb) = on_delta.lock() {
                    cb(content.clone());
                }
                full_content.push_str(&content);
            }
        }

        if let Some(tool_calls) = choice.delta.tool_calls {
            for tool_call in tool_calls {
                let entry = tool_calls_acc.entry(tool_call.index).or_default();

                if let Some(id) = tool_call.id {
                    entry.id = id;
                }

                if let Some(function) = tool_call.function {
                    if let Some(name) = function.name {
                        entry.name = name;
                    }
                    if let Some(arguments) = function.arguments {
                        entry.arguments.push_str(&arguments);
                    }
                }
            }
        }
    }
}

fn process_sse_line(
    line: &str,
    full_content: &mut String,
    tool_calls_acc: &mut HashMap<usize, AccToolCall>,
    on_delta: &DeltaCallback,
) -> Result<bool> {
    match parse_sse_line(line)? {
        ParsedSseLine::Ignore => Ok(false),
        ParsedSseLine::Done => Ok(true),
        ParsedSseLine::Chunk(chunk) => {
            apply_stream_chunk(chunk, full_content, tool_calls_acc, on_delta);
            Ok(false)
        }
    }
}

fn consume_sse_text_chunk(
    chunk_text: &str,
    sse_buffer: &mut String,
    full_content: &mut String,
    tool_calls_acc: &mut HashMap<usize, AccToolCall>,
    on_delta: &DeltaCallback,
) -> Result<bool> {
    sse_buffer.push_str(chunk_text);

    while let Some(newline_idx) = sse_buffer.find('\n') {
        let line_with_newline: String = sse_buffer.drain(..=newline_idx).collect();
        let line = line_with_newline
            .trim_end_matches('\n')
            .trim_end_matches('\r');

        if process_sse_line(line, full_content, tool_calls_acc, on_delta)? {
            return Ok(true);
        }
    }

    Ok(false)
}

fn flush_sse_buffer(
    sse_buffer: &mut String,
    full_content: &mut String,
    tool_calls_acc: &mut HashMap<usize, AccToolCall>,
    on_delta: &DeltaCallback,
) -> Result<bool> {
    if sse_buffer.is_empty() {
        return Ok(false);
    }

    let line = sse_buffer.trim_end_matches('\r');
    let done = process_sse_line(line, full_content, tool_calls_acc, on_delta)?;
    sse_buffer.clear();
    Ok(done)
}

fn to_tool_call_requests(tool_calls_acc: HashMap<usize, AccToolCall>) -> Vec<ToolCallRequest> {
    let mut entries: Vec<(usize, AccToolCall)> = tool_calls_acc.into_iter().collect();
    entries.sort_by_key(|(index, _)| *index);

    entries
        .into_iter()
        .map(|(_, call)| ToolCallRequest {
            id: call.id,
            name: call.name,
            args: serde_json::from_str::<Value>(&call.arguments)
                .unwrap_or_else(|_| Value::String(call.arguments)),
        })
        .collect()
}

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

fn to_openai_message(message: &LlmMessage) -> OpenAiRequestMessage {
    match message {
        LlmMessage::User { content } => OpenAiRequestMessage {
            role: "user".to_string(),
            content: Some(content.clone()),
            tool_call_id: None,
            tool_calls: None,
        },
        LlmMessage::Assistant {
            content,
            tool_calls,
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
        },
        LlmMessage::Tool {
            tool_call_id,
            content,
        } => OpenAiRequestMessage {
            role: "tool".to_string(),
            content: Some(content.clone()),
            tool_call_id: Some(tool_call_id.clone()),
            tool_calls: None,
        },
    }
}

#[derive(Debug, Serialize)]
struct OpenAiChatRequest<'a> {
    model: &'a str,
    messages: Vec<OpenAiRequestMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OpenAiTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'a str>,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct OpenAiRequestMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenAiToolCall>>,
}

#[derive(Debug, Serialize)]
struct OpenAiTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: OpenAiToolFunction,
}

#[derive(Debug, Serialize)]
struct OpenAiToolFunction {
    name: String,
    description: String,
    parameters: Value,
}

#[derive(Debug, Serialize)]
struct OpenAiToolCall {
    id: String,
    #[serde(rename = "type")]
    tool_type: String,
    function: OpenAiToolCallFunction,
}

#[derive(Debug, Serialize)]
struct OpenAiToolCallFunction {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiChatResponse {
    choices: Vec<OpenAiChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: OpenAiResponseMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponseMessage {
    content: Option<String>,
    tool_calls: Option<Vec<OpenAiResponseToolCall>>,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponseToolCall {
    id: String,
    function: OpenAiResponseToolFunction,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponseToolFunction {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamChunk {
    choices: Vec<OpenAiStreamChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamChoice {
    delta: OpenAiStreamDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamDelta {
    content: Option<String>,
    tool_calls: Option<Vec<OpenAiStreamToolCall>>,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamToolCall {
    index: usize,
    id: Option<String>,
    function: Option<OpenAiStreamToolCallFunction>,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamToolCallFunction {
    name: Option<String>,
    arguments: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::json;

    use super::*;

    #[test]
    fn parse_sse_line_parses_data_json() {
        let parsed = parse_sse_line(
            r#"data: {"choices":[{"delta":{"content":"Hello"},"finish_reason":null}]}"#,
        )
        .expect("line should parse");

        match parsed {
            ParsedSseLine::Chunk(chunk) => {
                assert_eq!(chunk.choices.len(), 1);
                assert_eq!(chunk.choices[0].delta.content.as_deref(), Some("Hello"));
            }
            _ => panic!("expected chunk"),
        }
    }

    #[test]
    fn parse_sse_line_skips_empty_line() {
        let parsed = parse_sse_line("   ").expect("line should parse");
        assert!(matches!(parsed, ParsedSseLine::Ignore));
    }

    #[test]
    fn parse_sse_line_recognizes_done() {
        let parsed = parse_sse_line("data: [DONE]").expect("line should parse");
        assert!(matches!(parsed, ParsedSseLine::Done));
    }

    #[test]
    fn content_delta_accumulates_and_emits_multiple_deltas() {
        use std::sync::{Arc, Mutex};
        let mut full_content = String::new();
        let mut tool_calls_acc = HashMap::new();
        let deltas: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let on_delta: super::DeltaCallback = {
            let deltas = deltas.clone();
            Arc::new(Mutex::new(move |delta: String| {
                deltas.lock().unwrap().push(delta);
            }))
        };

        apply_stream_chunk(
            OpenAiStreamChunk {
                choices: vec![OpenAiStreamChoice {
                    delta: OpenAiStreamDelta {
                        content: Some("Hel".to_string()),
                        tool_calls: None,
                    },
                    finish_reason: None,
                }],
            },
            &mut full_content,
            &mut tool_calls_acc,
            &on_delta,
        );

        apply_stream_chunk(
            OpenAiStreamChunk {
                choices: vec![OpenAiStreamChoice {
                    delta: OpenAiStreamDelta {
                        content: Some("lo".to_string()),
                        tool_calls: None,
                    },
                    finish_reason: None,
                }],
            },
            &mut full_content,
            &mut tool_calls_acc,
            &on_delta,
        );

        assert_eq!(
            deltas.lock().unwrap().as_slice(),
            &["Hel".to_string(), "lo".to_string()]
        );
        assert_eq!(full_content, "Hello");
    }

    #[test]
    fn tool_call_arguments_are_concatenated_incrementally() {
        use std::sync::{Arc, Mutex};
        let mut full_content = String::new();
        let mut tool_calls_acc = HashMap::new();
        let on_delta: super::DeltaCallback = Arc::new(Mutex::new(|_delta: String| {}));

        apply_stream_chunk(
            OpenAiStreamChunk {
                choices: vec![OpenAiStreamChoice {
                    delta: OpenAiStreamDelta {
                        content: None,
                        tool_calls: Some(vec![OpenAiStreamToolCall {
                            index: 0,
                            id: Some("call_1".to_string()),
                            function: Some(OpenAiStreamToolCallFunction {
                                name: Some("search".to_string()),
                                arguments: Some("{\"q\":\"hel".to_string()),
                            }),
                        }]),
                    },
                    finish_reason: None,
                }],
            },
            &mut full_content,
            &mut tool_calls_acc,
            &on_delta,
        );

        apply_stream_chunk(
            OpenAiStreamChunk {
                choices: vec![OpenAiStreamChoice {
                    delta: OpenAiStreamDelta {
                        content: None,
                        tool_calls: Some(vec![OpenAiStreamToolCall {
                            index: 0,
                            id: None,
                            function: Some(OpenAiStreamToolCallFunction {
                                name: None,
                                arguments: Some("lo\"}".to_string()),
                            }),
                        }]),
                    },
                    finish_reason: None,
                }],
            },
            &mut full_content,
            &mut tool_calls_acc,
            &on_delta,
        );

        let calls = to_tool_call_requests(tool_calls_acc);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].name, "search");
        assert_eq!(calls[0].args, json!({ "q": "hello" }));
    }

    #[test]
    fn partial_sse_line_buffer_is_preserved_across_chunks() {
        use std::sync::{Arc, Mutex};
        let mut sse_buffer = String::new();
        let mut full_content = String::new();
        let mut tool_calls_acc = HashMap::new();
        let deltas: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let on_delta: super::DeltaCallback = {
            let deltas = deltas.clone();
            Arc::new(Mutex::new(move |delta: String| {
                deltas.lock().unwrap().push(delta);
            }))
        };

        let done = consume_sse_text_chunk(
            "data: {\"choices\":[{\"delta\":{\"content\":\"He",
            &mut sse_buffer,
            &mut full_content,
            &mut tool_calls_acc,
            &on_delta,
        )
        .expect("first chunk should parse");
        assert!(!done);
        assert!(!sse_buffer.is_empty());

        let done = consume_sse_text_chunk(
            "llo\"},\"finish_reason\":null}]}\n\n",
            &mut sse_buffer,
            &mut full_content,
            &mut tool_calls_acc,
            &on_delta,
        )
        .expect("second chunk should parse");
        assert!(!done);
        assert!(sse_buffer.is_empty());
        assert_eq!(full_content, "Hello");
        assert_eq!(deltas.lock().unwrap().as_slice(), &["Hello".to_string()]);
    }
}
