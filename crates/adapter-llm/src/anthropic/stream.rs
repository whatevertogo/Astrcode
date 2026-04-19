use astrcode_core::{AstrError, Result};
use log::warn;
use serde_json::{Value, json};

use super::dto::{AnthropicUsage, SseProcessResult, extract_usage_from_payload};
use crate::{EventSink, LlmAccumulator, LlmEvent, classify_http_error, emit_event};

/// 解析单个 Anthropic SSE 块。
///
/// Anthropic SSE 块由多行组成（`event: ...\ndata: {...}\n\n`），
/// 本函数提取事件类型和 JSON payload，支持事件类型回退到 payload 中的 `type` 字段。
pub(super) fn parse_sse_block(block: &str) -> Result<Option<(String, Value)>> {
    let trimmed = block.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let mut event_type = None;
    let mut data_lines = Vec::new();

    for line in trimmed.lines() {
        if let Some(value) = sse_field_value(line, "event") {
            event_type = Some(value.trim().to_string());
        } else if let Some(value) = sse_field_value(line, "data") {
            data_lines.push(value);
        }
    }

    if data_lines.is_empty() {
        return Ok(None);
    }

    let data = data_lines.join("\n");
    let data = data.trim();
    if data.is_empty() {
        return Ok(None);
    }

    // 兼容部分 Anthropic 网关沿用 OpenAI 风格的流结束哨兵。
    // 如果这里严格要求 JSON，会在流尾直接误报 parse error。
    if data == "[DONE]" {
        return Ok(Some((
            "message_stop".to_string(),
            json!({ "type": "message_stop" }),
        )));
    }

    let payload = serde_json::from_str::<Value>(data)
        .map_err(|error| AstrError::parse("failed to parse anthropic sse payload", error))?;
    let event_type = event_type
        .or_else(|| {
            payload
                .get("type")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default();

    Ok(Some((event_type, payload)))
}

fn sse_field_value<'a>(line: &'a str, field: &str) -> Option<&'a str> {
    let value = line.strip_prefix(field)?.strip_prefix(':')?;

    // SSE 规范只忽略冒号后的一个可选空格；这里兼容 `data:...` 和 `data: ...`，
    // 同时保留业务数据中其余前导空白，避免悄悄改写 payload。
    Some(value.strip_prefix(' ').unwrap_or(value))
}

/// 从 `content_block_start` 事件 payload 中提取内容块。
///
/// Anthropic 在 `content_block_start` 事件中将块数据放在 `content_block` 字段，
/// 但某些事件可能直接放在根级别，因此有回退逻辑。
fn extract_start_block(payload: &Value) -> &Value {
    payload.get("content_block").unwrap_or(payload)
}

/// 从 `content_block_delta` 事件 payload 中提取增量数据。
///
/// Anthropic 在 `content_block_delta` 事件中将增量数据放在 `delta` 字段。
fn extract_delta_block(payload: &Value) -> &Value {
    payload.get("delta").unwrap_or(payload)
}

pub(super) fn anthropic_stream_error(payload: &Value) -> AstrError {
    let error = payload.get("error").unwrap_or(payload);
    let message = error
        .get("message")
        .or_else(|| error.get("msg"))
        .or_else(|| payload.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("anthropic stream returned an error event");

    let mut error_type = error
        .get("type")
        .or_else(|| error.get("code"))
        .or_else(|| payload.get("error_type"))
        .or_else(|| payload.get("code"))
        .and_then(Value::as_str)
        .unwrap_or("unknown_error");

    // Why: 部分兼容网关不回传结构化 error.type，只给中文文案。
    // 这类错误本质仍是请求参数错误，不应退化成 internal stream error。
    let message_lower = message.to_lowercase();
    if matches!(error_type, "unknown_error" | "error")
        && (message_lower.contains("参数非法")
            || message_lower.contains("invalid request")
            || message_lower.contains("invalid parameter")
            || message_lower.contains("invalid arguments")
            || (message_lower.contains("messages") && message_lower.contains("illegal")))
    {
        error_type = "invalid_request_error";
    }

    let detail = format!("{error_type}: {message}");

    match error_type {
        "invalid_request_error" => classify_http_error(400, &detail).into(),
        "authentication_error" => classify_http_error(401, &detail).into(),
        "permission_error" => classify_http_error(403, &detail).into(),
        "not_found_error" => classify_http_error(404, &detail).into(),
        "rate_limit_error" => classify_http_error(429, &detail).into(),
        "overloaded_error" => classify_http_error(529, &detail).into(),
        "api_error" => classify_http_error(500, &detail).into(),
        _ => classify_http_error(400, &detail).into(),
    }
}

/// 处理单个 Anthropic SSE 块，返回 `(is_done, stop_reason)`。
///
/// Anthropic SSE 事件类型分派：
/// - `content_block_start`: 新内容块开始（可能是文本或工具调用）
/// - `content_block_delta`: 增量内容（文本/思考/签名/工具参数）
/// - `message_stop`: 流结束信号，返回 is_done=true
/// - `message_delta`: 包含 `stop_reason`，用于检测 max_tokens 截断 (P4.2)
/// - `message_start/content_block_stop/ping`: 元数据事件，静默忽略
fn process_sse_block(
    block: &str,
    accumulator: &mut LlmAccumulator,
    sink: &EventSink,
) -> Result<SseProcessResult> {
    let Some((event_type, payload)) = parse_sse_block(block)? else {
        return Ok(SseProcessResult::default());
    };

    match event_type.as_str() {
        "content_block_start" => {
            let index = payload
                .get("index")
                .and_then(Value::as_u64)
                .unwrap_or_default() as usize;
            let block = extract_start_block(&payload);

            // 工具调用块开始时，发射 ToolCallDelta（id + name，参数为空）
            if block_type(block) == Some("tool_use") {
                emit_event(
                    LlmEvent::ToolCallDelta {
                        index,
                        id: block.get("id").and_then(Value::as_str).map(str::to_string),
                        name: block
                            .get("name")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        arguments_delta: String::new(),
                    },
                    accumulator,
                    sink,
                );
            }
            Ok(SseProcessResult::default())
        },
        "content_block_delta" => {
            let index = payload
                .get("index")
                .and_then(Value::as_u64)
                .unwrap_or_default() as usize;
            let delta = extract_delta_block(&payload);

            // 根据增量类型分派到对应的事件
            match block_type(delta) {
                Some("text_delta") => {
                    if let Some(text) = delta.get("text").and_then(Value::as_str) {
                        emit_event(LlmEvent::TextDelta(text.to_string()), accumulator, sink);
                    }
                },
                Some("thinking_delta") => {
                    if let Some(text) = delta.get("thinking").and_then(Value::as_str) {
                        emit_event(LlmEvent::ThinkingDelta(text.to_string()), accumulator, sink);
                    }
                },
                Some("signature_delta") => {
                    if let Some(signature) = delta.get("signature").and_then(Value::as_str) {
                        emit_event(
                            LlmEvent::ThinkingSignature(signature.to_string()),
                            accumulator,
                            sink,
                        );
                    }
                },
                Some("input_json_delta") => {
                    // 工具调用参数增量，partial_json 是 JSON 的片段
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
                },
                _ => {},
            }
            Ok(SseProcessResult::default())
        },
        "message_stop" => Ok(SseProcessResult {
            done: true,
            ..SseProcessResult::default()
        }),
        // message_delta 可能包含 stop_reason (P4.2)
        "message_delta" => {
            let stop_reason = payload
                .get("delta")
                .and_then(|d| d.get("stop_reason"))
                .and_then(Value::as_str)
                .map(str::to_string);
            Ok(SseProcessResult {
                stop_reason,
                usage: extract_usage_from_payload(&event_type, &payload),
                ..SseProcessResult::default()
            })
        },
        "message_start" => Ok(SseProcessResult {
            usage: extract_usage_from_payload(&event_type, &payload),
            ..SseProcessResult::default()
        }),
        "content_block_stop" | "ping" => Ok(SseProcessResult::default()),
        "error" => Err(anthropic_stream_error(&payload)),
        other => {
            warn!("anthropic: unknown sse event: {}", other);
            Ok(SseProcessResult::default())
        },
    }
}

/// 在 SSE 缓冲区中查找下一个完整的 SSE 块边界。
///
/// Anthropic SSE 块由双换行符分隔（`\r\n\r\n` 或 `\n\n`）。
/// 返回 `(块结束位置, 分隔符长度)`，如果未找到完整块则返回 `None`。
fn next_sse_block(buffer: &str) -> Option<(usize, usize)> {
    if let Some(idx) = buffer.find("\r\n\r\n") {
        return Some((idx, 4));
    }
    if let Some(idx) = buffer.find("\n\n") {
        return Some((idx, 2));
    }
    None
}

fn apply_sse_process_result(
    result: SseProcessResult,
    stop_reason_out: &mut Option<String>,
    usage_out: &mut AnthropicUsage,
) -> bool {
    if let Some(r) = result.stop_reason {
        *stop_reason_out = Some(r);
    }
    if let Some(usage) = result.usage {
        usage_out.merge_from(usage);
    }
    result.done
}

pub(super) fn consume_sse_text_chunk(
    chunk_text: &str,
    sse_buffer: &mut String,
    accumulator: &mut LlmAccumulator,
    sink: &EventSink,
    stop_reason_out: &mut Option<String>,
    usage_out: &mut AnthropicUsage,
) -> Result<bool> {
    sse_buffer.push_str(chunk_text);

    while let Some((block_end, delimiter_len)) = next_sse_block(sse_buffer) {
        let block: String = sse_buffer.drain(..block_end + delimiter_len).collect();
        let block = &block[..block_end];

        let result = process_sse_block(block, accumulator, sink)?;
        if apply_sse_process_result(result, stop_reason_out, usage_out) {
            return Ok(true);
        }
    }

    Ok(false)
}

pub(super) fn flush_sse_buffer(
    sse_buffer: &mut String,
    accumulator: &mut LlmAccumulator,
    sink: &EventSink,
    stop_reason_out: &mut Option<String>,
    usage_out: &mut AnthropicUsage,
) -> Result<()> {
    if sse_buffer.trim().is_empty() {
        sse_buffer.clear();
        return Ok(());
    }

    while let Some((block_end, delimiter_len)) = next_sse_block(sse_buffer) {
        let block: String = sse_buffer.drain(..block_end + delimiter_len).collect();
        let block = &block[..block_end];

        let result = process_sse_block(block, accumulator, sink)?;
        if apply_sse_process_result(result, stop_reason_out, usage_out) {
            sse_buffer.clear();
            return Ok(());
        }
    }

    if !sse_buffer.trim().is_empty() {
        let result = process_sse_block(sse_buffer, accumulator, sink)?;
        apply_sse_process_result(result, stop_reason_out, usage_out);
    }
    sse_buffer.clear();
    Ok(())
}

fn block_type(value: &Value) -> Option<&str> {
    value.get("type").and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use serde_json::json;

    use super::{consume_sse_text_chunk, flush_sse_buffer, parse_sse_block};
    use crate::{
        LlmAccumulator, LlmEvent, LlmUsage, Utf8StreamDecoder, anthropic::dto::AnthropicUsage,
        sink_collector,
    };

    #[test]
    fn streaming_sse_parses_tool_calls_and_text() {
        let mut accumulator = LlmAccumulator::default();
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = sink_collector(events.clone());
        let mut sse_buffer = String::new();

        let chunk = concat!(
            "event: content_block_start\n",
            "data: {\"index\":1,\"type\":\"tool_use\",\"id\":\"call_1\",\"name\":\"search\"}\n\n",
            "event: content_block_delta\n",
            "data: {\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"\
             q\\\":\\\"ru\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"st\\\"\
             }\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n"
        );

        let mut stop_reason_out: Option<String> = None;
        let mut usage_out = AnthropicUsage::default();
        let done = consume_sse_text_chunk(
            chunk,
            &mut sse_buffer,
            &mut accumulator,
            &sink,
            &mut stop_reason_out,
            &mut usage_out,
        )
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
        assert!(
            events
                .iter()
                .any(|event| matches!(event, LlmEvent::TextDelta(text) if text == "hello"))
        );
        assert_eq!(output.content, "hello");
        assert_eq!(output.tool_calls.len(), 1);
        assert_eq!(output.tool_calls[0].args, json!({ "q": "rust" }));
    }

    #[test]
    fn parse_sse_block_accepts_data_lines_without_space_after_colon() {
        let block = concat!(
            "event:content_block_delta\n",
            "data:{\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n"
        );

        let parsed = parse_sse_block(block)
            .expect("block should parse")
            .expect("block should contain payload");

        assert_eq!(parsed.0, "content_block_delta");
        assert_eq!(parsed.1["delta"]["text"], json!("hello"));
    }

    #[test]
    fn parse_sse_block_treats_done_sentinel_as_message_stop() {
        let parsed = parse_sse_block("data: [DONE]\n")
            .expect("done sentinel should parse")
            .expect("done sentinel should produce payload");

        assert_eq!(parsed.0, "message_stop");
        assert_eq!(parsed.1["type"], json!("message_stop"));
    }

    #[test]
    fn parse_sse_block_ignores_empty_data_payload() {
        let parsed = parse_sse_block("event: ping\ndata:\n");
        assert!(matches!(parsed, Ok(None)));
    }

    #[test]
    fn streaming_sse_error_event_surfaces_structured_provider_failure() {
        let mut accumulator = LlmAccumulator::default();
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = sink_collector(events);
        let mut sse_buffer = String::new();
        let mut stop_reason_out: Option<String> = None;
        let mut usage_out = AnthropicUsage::default();

        let error = consume_sse_text_chunk(
            concat!(
                "event: error\n",
                "data: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",",
                "\"message\":\"capacity exhausted\"}}\n\n"
            ),
            &mut sse_buffer,
            &mut accumulator,
            &sink,
            &mut stop_reason_out,
            &mut usage_out,
        )
        .expect_err("error event should terminate the stream with a structured error");

        match error {
            astrcode_core::AstrError::LlmRequestFailed { status, body } => {
                assert_eq!(status, 529);
                assert!(body.contains("overloaded_error"));
                assert!(body.contains("capacity exhausted"));
            },
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[test]
    fn streaming_sse_error_event_without_type_still_maps_to_request_error() {
        let mut accumulator = LlmAccumulator::default();
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = sink_collector(events);
        let mut sse_buffer = String::new();
        let mut stop_reason_out: Option<String> = None;
        let mut usage_out = AnthropicUsage::default();

        let error = consume_sse_text_chunk(
            concat!(
                "event: error\n",
                "data: {\"type\":\"error\",\"error\":{\"message\":\"messages 参数非法\"}}\n\n"
            ),
            &mut sse_buffer,
            &mut accumulator,
            &sink,
            &mut stop_reason_out,
            &mut usage_out,
        )
        .expect_err("error event should terminate the stream with a structured error");

        match error {
            astrcode_core::AstrError::LlmRequestFailed { status, body } => {
                assert_eq!(status, 400);
                assert!(body.contains("invalid_request_error"));
                assert!(body.contains("messages 参数非法"));
            },
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[test]
    fn streaming_sse_extracts_usage_from_message_events() {
        let mut accumulator = LlmAccumulator::default();
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = sink_collector(events);
        let mut usage_out = AnthropicUsage::default();
        let mut stop_reason_out = None;
        let mut sse_buffer = String::new();

        let chunk = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":120,\"\
             cache_creation_input_tokens\":90,\"cache_read_input_tokens\":70}}}\n\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":\
             {\"output_tokens\":33}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n"
        );

        let done = consume_sse_text_chunk(
            chunk,
            &mut sse_buffer,
            &mut accumulator,
            &sink,
            &mut stop_reason_out,
            &mut usage_out,
        )
        .expect("stream chunk should parse");

        assert!(done);
        assert_eq!(stop_reason_out.as_deref(), Some("end_turn"));
        assert_eq!(
            usage_out.into_llm_usage(),
            Some(LlmUsage {
                input_tokens: 120,
                output_tokens: 33,
                cache_creation_input_tokens: 90,
                cache_read_input_tokens: 70,
            })
        );
    }

    #[test]
    fn streaming_sse_handles_multibyte_text_split_across_chunks() {
        let mut accumulator = LlmAccumulator::default();
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = sink_collector(events.clone());
        let mut sse_buffer = String::new();
        let mut decoder = Utf8StreamDecoder::default();
        let mut stop_reason_out = None;
        let mut usage_out = AnthropicUsage::default();
        let chunk = concat!(
            "event: content_block_delta\n",
            "data: {\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"你",
            "好\"}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n"
        );
        let bytes = chunk.as_bytes();
        let split_index = chunk
            .find("好")
            .expect("chunk should contain multibyte char")
            + 1;

        let first_text = decoder
            .push(
                &bytes[..split_index],
                "anthropic response stream was not valid utf-8",
            )
            .expect("first split should decode");
        let second_text = decoder
            .push(
                &bytes[split_index..],
                "anthropic response stream was not valid utf-8",
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
                    &mut stop_reason_out,
                    &mut usage_out,
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
                    &mut stop_reason_out,
                    &mut usage_out,
                )
            })
            .transpose()
            .expect("second chunk should parse")
            .unwrap_or(false);

        assert!(!first_done);
        assert!(second_done);
        let output = accumulator.finish();
        let events = events.lock().expect("lock").clone();

        assert!(
            events
                .iter()
                .any(|event| matches!(event, LlmEvent::TextDelta(text) if text == "你好"))
        );
        assert_eq!(output.content, "你好");
    }

    #[test]
    fn flush_sse_buffer_processes_all_complete_blocks_before_tail_block() {
        let mut accumulator = LlmAccumulator::default();
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = sink_collector(events.clone());
        let mut sse_buffer = concat!(
            "event: content_block_delta\n",
            "data: {\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":",
            "{\"output_tokens\":7}}"
        )
        .to_string();
        let mut stop_reason_out = None;
        let mut usage_out = AnthropicUsage::default();

        flush_sse_buffer(
            &mut sse_buffer,
            &mut accumulator,
            &sink,
            &mut stop_reason_out,
            &mut usage_out,
        )
        .expect("flush should process buffered blocks");

        let output = accumulator.finish();
        let events = events.lock().expect("lock").clone();

        assert!(sse_buffer.is_empty());
        assert!(
            events
                .iter()
                .any(|event| matches!(event, LlmEvent::TextDelta(text) if text == "hello"))
        );
        assert_eq!(output.content, "hello");
        assert_eq!(stop_reason_out.as_deref(), Some("end_turn"));
        assert_eq!(
            usage_out.into_llm_usage(),
            Some(LlmUsage {
                input_tokens: 0,
                output_tokens: 7,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            })
        );
    }
}
