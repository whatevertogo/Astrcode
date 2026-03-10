use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use log::warn;
use serde::Serialize;
use serde_json::Value;
use tokio::select;
use tokio_util::sync::CancellationToken;

use crate::action::{LlmMessage, ToolCallRequest, ToolDefinition};
use crate::llm::{EventSink, LlmAccumulator, LlmEvent, LlmOutput, LlmProvider, LlmRequest};

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_MAX_TOKENS: u32 = 8096;

#[derive(Clone, Debug)]
pub struct AnthropicProvider {
    client: reqwest::Client,
    api_key: String,
    model: String,
    max_tokens: u32,
}

impl AnthropicProvider {
    pub fn new(api_key: String, model: String) -> Self {
        Self::with_max_tokens(api_key, model, DEFAULT_MAX_TOKENS)
    }

    pub fn with_max_tokens(api_key: String, model: String, max_tokens: u32) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            model,
            max_tokens,
        }
    }

    fn build_request(&self, messages: &[LlmMessage], tools: &[ToolDefinition], stream: bool) -> AnthropicRequest {
        AnthropicRequest {
            model: self.model.clone(),
            max_tokens: self.max_tokens,
            messages: to_anthropic_messages(messages),
            tools: if tools.is_empty() {
                None
            } else {
                Some(to_anthropic_tools(tools))
            },
            stream: stream.then_some(true),
        }
    }

    async fn send_request(
        &self,
        request: &AnthropicRequest,
        cancel: CancellationToken,
    ) -> Result<reqwest::Response> {
        let send_future = self
            .client
            .post(ANTHROPIC_API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(request)
            .send();

        let response = select! {
            _ = cancel.cancelled() => {
                return Err(anyhow!("llm request interrupted"));
            }
            result = send_future => result.context("failed to call anthropic endpoint")?
        };

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(anyhow!("Anthropic API Key 无效"));
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!("请求失败: {}: {}", status, body));
        }

        Ok(response)
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn generate(&self, request: LlmRequest, sink: Option<EventSink>) -> Result<LlmOutput> {
        let cancel = request.cancel;
        let body = self.build_request(&request.messages, &request.tools, sink.is_some());
        let response = self.send_request(&body, cancel.child_token()).await?;

        match sink {
            None => {
                let payload: AnthropicResponse = response
                    .json()
                    .await
                    .context("failed to parse anthropic response")?;
                Ok(response_to_output(payload))
            }
            Some(sink) => {
                let mut stream = response.bytes_stream();
                let mut sse_buffer = String::new();
                let mut accumulator = LlmAccumulator::default();

                loop {
                    let next_item = select! {
                        _ = cancel.cancelled() => {
                            return Err(anyhow!("llm request interrupted"));
                        }
                        item = stream.next() => item,
                    };

                    let Some(item) = next_item else {
                        break;
                    };

                    let bytes = item.context("failed to read anthropic response stream")?;
                    let chunk_text = std::str::from_utf8(&bytes)
                        .context("anthropic response stream was not valid utf-8")?;

                    if consume_sse_text_chunk(chunk_text, &mut sse_buffer, &mut accumulator, &sink)? {
                        return Ok(accumulator.finish());
                    }
                }

                flush_sse_buffer(&mut sse_buffer, &mut accumulator, &sink)?;
                Ok(accumulator.finish())
            }
        }
    }
}

fn to_anthropic_messages(messages: &[LlmMessage]) -> Vec<AnthropicMessage> {
    messages
        .iter()
        .map(|message| match message {
            LlmMessage::User { content } => AnthropicMessage {
                role: "user".to_string(),
                content: vec![AnthropicContentBlock::Text {
                    text: content.clone(),
                }],
            },
            LlmMessage::Assistant {
                content,
                tool_calls,
            } => {
                let mut blocks = Vec::new();
                if !content.is_empty() {
                    blocks.push(AnthropicContentBlock::Text {
                        text: content.clone(),
                    });
                }
                blocks.extend(tool_calls.iter().map(|call| AnthropicContentBlock::ToolUse {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    input: call.args.clone(),
                }));

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
                }],
            },
        })
        .collect()
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

fn emit_event(event: LlmEvent, accumulator: &mut LlmAccumulator, sink: &EventSink) {
    sink(event.clone());
    accumulator.apply(&event);
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
    let payload = serde_json::from_str::<Value>(&data).context("failed to parse anthropic sse payload")?;
    let event_type = event_type
        .or_else(|| payload.get("type").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default();

    Ok(Some((event_type, payload)))
}

fn extract_start_block<'a>(payload: &'a Value) -> &'a Value {
    payload.get("content_block").unwrap_or(payload)
}

fn extract_delta_block<'a>(payload: &'a Value) -> &'a Value {
    payload.get("delta").unwrap_or(payload)
}

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

            if block_type(block) == Some("tool_use") {
                emit_event(
                    LlmEvent::ToolCallDelta {
                        index,
                        id: block.get("id").and_then(Value::as_str).map(str::to_string),
                        name: block.get("name").and_then(Value::as_str).map(str::to_string),
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

            match block_type(delta) {
                Some("text_delta") => {
                    if let Some(text) = delta.get("text").and_then(Value::as_str) {
                        emit_event(LlmEvent::TextDelta(text.to_string()), accumulator, sink);
                    }
                }
                Some("input_json_delta") => {
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
        "message_start" | "message_delta" | "content_block_stop" | "ping" => Ok(false),
        other => {
            warn!("anthropic: unknown sse event: {}", other);
            Ok(false)
        }
    }
}

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

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

#[derive(Debug, Serialize)]
struct AnthropicMessage {
    role: String,
    content: Vec<AnthropicContentBlock>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicContentBlock {
    Text { text: String },
    ToolUse { id: String, name: String, input: Value },
    ToolResult { tool_use_id: String, content: String },
}

#[derive(Debug, Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: Value,
}

#[derive(Debug, serde::Deserialize)]
struct AnthropicResponse {
    content: Vec<Value>,
    stop_reason: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use serde_json::json;

    use super::*;

    fn sink_collector(events: Arc<Mutex<Vec<LlmEvent>>>) -> EventSink {
        Arc::new(move |event| {
            events.lock().expect("lock").push(event);
        })
    }

    #[test]
    fn to_anthropic_messages_converts_user_assistant_and_tool() {
        let messages = to_anthropic_messages(&[
            LlmMessage::User {
                content: "hello".to_string(),
            },
            LlmMessage::Assistant {
                content: "done".to_string(),
                tool_calls: vec![ToolCallRequest {
                    id: "call_1".to_string(),
                    name: "search".to_string(),
                    args: json!({ "q": "rust" }),
                }],
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
        }]);

        match &messages[0].content[..] {
            [
                AnthropicContentBlock::Text { text },
                AnthropicContentBlock::ToolUse { id, name, input },
            ] => {
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
        });

        assert_eq!(output.content, "hello world");
        assert_eq!(output.tool_calls.len(), 1);
        assert_eq!(output.tool_calls[0].id, "call_1");
        assert_eq!(output.tool_calls[0].args, json!({ "q": "rust" }));
    }

    #[test]
    fn streaming_content_block_delta_emits_and_accumulates_events() {
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
}
