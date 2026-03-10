use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::select;
use tokio_util::sync::CancellationToken;

use crate::action::{LlmMessage, ToolCallRequest, ToolDefinition};
use crate::llm::{EventSink, LlmAccumulator, LlmEvent, LlmOutput, LlmProvider, LlmRequest};

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

    fn build_request<'a>(
        &'a self,
        messages: &'a [LlmMessage],
        tools: &'a [ToolDefinition],
        stream: bool,
    ) -> OpenAiChatRequest<'a> {
        OpenAiChatRequest {
            model: &self.model,
            messages: messages.iter().map(to_openai_message).collect(),
            tools: if tools.is_empty() {
                None
            } else {
                Some(tools.iter().map(to_openai_tool).collect())
            },
            tool_choice: if tools.is_empty() { None } else { Some("auto") },
            stream,
        }
    }

    async fn send_request(
        &self,
        req: &OpenAiChatRequest<'_>,
        cancel: CancellationToken,
    ) -> Result<reqwest::Response> {
        let endpoint = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let send_future = self
            .client
            .post(endpoint)
            .bearer_auth(&self.api_key)
            .json(req)
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

        Ok(response)
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn generate(&self, request: LlmRequest, sink: Option<EventSink>) -> Result<LlmOutput> {
        let cancel = request.cancel;
        let req = self.build_request(&request.messages, &request.tools, sink.is_some());
        let response = self.send_request(&req, cancel.child_token()).await?;

        match sink {
            None => {
                let parsed: OpenAiChatResponse = response
                    .json()
                    .await
                    .context("failed to parse openai-compatible response")?;
                let first_choice = parsed
                    .choices
                    .into_iter()
                    .next()
                    .ok_or_else(|| anyhow!("openai-compatible response did not include choices"))?;
                Ok(message_to_output(first_choice.message))
            }
            Some(sink) => {
                let mut body_stream = response.bytes_stream();
                let mut sse_buffer = String::new();
                let mut accumulator = LlmAccumulator::default();

                loop {
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
}

fn message_to_output(message: OpenAiResponseMessage) -> LlmOutput {
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

    LlmOutput {
        content,
        tool_calls,
    }
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

fn apply_stream_chunk(chunk: OpenAiStreamChunk) -> Vec<LlmEvent> {
    let mut events = Vec::new();

    for choice in chunk.choices {
        let _ = choice.finish_reason;

        if let Some(content) = choice.delta.content {
            if !content.is_empty() {
                events.push(LlmEvent::TextDelta(content));
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

fn emit_event(event: LlmEvent, accumulator: &mut LlmAccumulator, sink: &EventSink) {
    sink(event.clone());
    accumulator.apply(&event);
}

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
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::task::JoinHandle;
    use tokio_util::sync::CancellationToken;

    use super::*;

    fn sink_collector(events: Arc<Mutex<Vec<LlmEvent>>>) -> EventSink {
        Arc::new(move |event| {
            events.lock().expect("lock").push(event);
        })
    }

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
    fn parse_sse_line_parses_data_json() {
        let parsed = parse_sse_line(
            r#"data: {"choices":[{"delta":{"content":"Hello"},"finish_reason":null}]}"#,
        )
        .expect("line should parse");

        match parsed {
            ParsedSseLine::Chunk(chunk) => {
                let events = apply_stream_chunk(chunk);
                assert!(matches!(
                    events.as_slice(),
                    [LlmEvent::TextDelta(text)] if text == "Hello"
                ));
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
    fn tool_call_delta_maps_empty_arguments_when_missing() {
        let events = apply_stream_chunk(OpenAiStreamChunk {
            choices: vec![OpenAiStreamChoice {
                delta: OpenAiStreamDelta {
                    content: None,
                    tool_calls: Some(vec![OpenAiStreamToolCall {
                        index: 0,
                        id: Some("call_1".to_string()),
                        function: Some(OpenAiStreamToolCallFunction {
                            name: Some("search".to_string()),
                            arguments: None,
                        }),
                    }]),
                },
                finish_reason: None,
            }],
        });

        assert!(matches!(
            events.as_slice(),
            [LlmEvent::ToolCallDelta {
                index: 0,
                id: Some(id),
                name: Some(name),
                arguments_delta,
            }] if id == "call_1" && name == "search" && arguments_delta.is_empty()
        ));
    }

    #[test]
    fn partial_sse_line_buffer_is_preserved_across_chunks() {
        let mut sse_buffer = String::new();
        let mut accumulator = LlmAccumulator::default();
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = sink_collector(events.clone());

        let done = consume_sse_text_chunk(
            "data: {\"choices\":[{\"delta\":{\"content\":\"He",
            &mut sse_buffer,
            &mut accumulator,
            &sink,
        )
        .expect("first chunk should parse");
        assert!(!done);
        assert!(!sse_buffer.is_empty());

        let done = consume_sse_text_chunk(
            "llo\"},\"finish_reason\":null}]}\n\n",
            &mut sse_buffer,
            &mut accumulator,
            &sink,
        )
        .expect("second chunk should parse");
        assert!(!done);
        assert!(sse_buffer.is_empty());
        assert_eq!(accumulator.content, "Hello");
        assert!(matches!(
            events.lock().expect("lock").as_slice(),
            [LlmEvent::TextDelta(text)] if text == "Hello"
        ));
    }

    #[test]
    fn tool_call_arguments_are_concatenated_incrementally() {
        let mut accumulator = LlmAccumulator::default();
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = sink_collector(events);

        for event in apply_stream_chunk(OpenAiStreamChunk {
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
        }) {
            emit_event(event, &mut accumulator, &sink);
        }

        for event in apply_stream_chunk(OpenAiStreamChunk {
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
        }) {
            emit_event(event, &mut accumulator, &sink);
        }

        let output = accumulator.finish();
        assert_eq!(output.tool_calls.len(), 1);
        assert_eq!(output.tool_calls[0].id, "call_1");
        assert_eq!(output.tool_calls[0].name, "search");
        assert_eq!(output.tool_calls[0].args, json!({ "q": "hello" }));
    }

    #[tokio::test]
    async fn generate_without_sink_uses_non_streaming_path() {
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
        let provider = OpenAiProvider::new(base_url, "sk-test".to_string(), "model-a".to_string());

        let output = provider
            .generate(
                LlmRequest::new(
                    vec![LlmMessage::User {
                        content: "hi".to_string(),
                    }],
                    vec![],
                    CancellationToken::new(),
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
    async fn generate_with_sink_emits_events_and_returns_output() {
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
        let provider = OpenAiProvider::new(base_url, "sk-test".to_string(), "model-a".to_string());
        let events = Arc::new(Mutex::new(Vec::new()));

        let output = provider
            .generate(
                LlmRequest::new(
                    vec![LlmMessage::User {
                        content: "hi".to_string(),
                    }],
                    vec![],
                    CancellationToken::new(),
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
