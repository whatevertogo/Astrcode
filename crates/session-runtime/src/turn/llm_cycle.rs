//! LLM 调用周期
//!
//! 封装流式 LLM 调用和 prompt-too-long 错误检测。
//!
//! ## 架构模式：unbounded channel + select + drain
//!
//! 使用 `tokio::select!` 同时等待 LLM 完成和实时转发流式事件。
//! LLM 完成后用 `try_recv()` 排空 channel 中残余事件。
//!
//! 为什么使用 unbounded channel：生产者（LLM 流式传输）受网络 I/O 带宽约束，
//! 消费者（select 循环）以同等速度处理事件，缓冲区积压始终是少量 delta。
//! 使用 bounded channel 会不必要地复杂化反压逻辑。

use std::sync::Arc;

use astrcode_core::{
    AgentEvent, AgentEventContext, AstrError, CancelToken, LlmEvent, LlmOutput, LlmRequest,
    ReasoningContent, Result,
};
use astrcode_kernel::{KernelError, KernelGateway};
use tokio::sync::mpsc;

use crate::SessionState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StreamedToolCallDelta {
    pub index: usize,
    pub id: Option<String>,
    pub name: Option<String>,
    pub arguments_delta: String,
}

pub(crate) type ToolCallDeltaSink = Arc<dyn Fn(StreamedToolCallDelta) + Send + Sync>;

/// 调用 LLM，并把流式 thinking 片段回填到最终 `LlmOutput.reasoning`。
///
/// LLM 完成前推送的最后几个 delta 可能还在 channel 缓冲中，
/// 因此在 LLM 返回后还需 `try_recv()` 排空残余事件。
pub async fn call_llm_streaming(
    gateway: &KernelGateway,
    request: LlmRequest,
    turn_id: &str,
    agent: &AgentEventContext,
    session_state: &SessionState,
    cancel: &CancelToken,
    tool_delta_sink: Option<ToolCallDeltaSink>,
) -> Result<astrcode_core::LlmOutput> {
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<LlmEvent>();

    let sink: astrcode_core::LlmEventSink = Arc::new(move |event| {
        let _ = event_tx.send(event);
    });

    let generate_future = gateway.call_llm(request, Some(sink));
    tokio::pin!(generate_future);

    let mut event_rx_open = true;
    let mut thinking_deltas = Vec::new();
    let mut thinking_signature = None;
    let output = loop {
        tokio::select! {
            result = &mut generate_future => break result,
            maybe_event = event_rx.recv(), if event_rx_open => {
                match maybe_event {
                    Some(event) => emit_llm_delta_live(
                        event,
                        turn_id,
                        agent,
                        session_state,
                        tool_delta_sink.as_ref(),
                        &mut thinking_deltas,
                        &mut thinking_signature,
                    ),
                    None => event_rx_open = false,
                }
            }
        }

        if cancel.is_cancelled() {
            return Err(AstrError::LlmInterrupted);
        }
    };

    // 排空 channel 中残余事件：LLM 完成前推送的最后几个 delta
    while let Ok(event) = event_rx.try_recv() {
        emit_llm_delta_live(
            event,
            turn_id,
            agent,
            session_state,
            tool_delta_sink.as_ref(),
            &mut thinking_deltas,
            &mut thinking_signature,
        );
    }

    let mut output = output.map_err(map_kernel_error)?;
    hydrate_reasoning_from_stream(&mut output, &thinking_deltas, thinking_signature.as_deref());

    Ok(output)
}

fn hydrate_reasoning_from_stream(
    output: &mut LlmOutput,
    thinking_deltas: &[String],
    thinking_signature: Option<&str>,
) {
    if output.reasoning.is_none() && !thinking_deltas.is_empty() {
        output.reasoning = Some(ReasoningContent {
            content: thinking_deltas.concat(),
            signature: thinking_signature.map(ToString::to_string),
        });
        return;
    }

    if let Some(reasoning) = output.reasoning.as_mut() {
        if reasoning.signature.is_none() {
            reasoning.signature = thinking_signature.map(ToString::to_string);
        }
    }
}

/// 将 LLM 流式增量发到 live 广播，并收集 thinking 片段用于最终 output 回填。
///
/// Why:
/// - live 广播负责“即时吐字”，避免前端只能在 turn 结束后一次性看到内容
/// - durable 真相只保留 `AssistantFinal.reasoning_content`，因此这里需要兜底补齐
fn emit_llm_delta_live(
    event: LlmEvent,
    turn_id: &str,
    agent: &AgentEventContext,
    session_state: &SessionState,
    tool_delta_sink: Option<&ToolCallDeltaSink>,
    thinking_deltas: &mut Vec<String>,
    thinking_signature: &mut Option<String>,
) {
    match event {
        LlmEvent::TextDelta(text) => {
            session_state.broadcast_live_event(AgentEvent::ModelDelta {
                turn_id: turn_id.to_string(),
                agent: agent.clone(),
                delta: text,
            });
        },
        LlmEvent::ThinkingDelta(text) => {
            thinking_deltas.push(text.clone());
            session_state.broadcast_live_event(AgentEvent::ThinkingDelta {
                turn_id: turn_id.to_string(),
                agent: agent.clone(),
                delta: text,
            });
        },
        LlmEvent::ToolCallDelta {
            index,
            id,
            name,
            arguments_delta,
        } => {
            if let Some(sink) = tool_delta_sink {
                sink(StreamedToolCallDelta {
                    index,
                    id,
                    name,
                    arguments_delta,
                });
            }
        },
        // ThinkingSignature 是 Anthropic API 的 thinking 完整性令牌。
        // live UI 不消费它，但 durable AssistantFinal 需要保留这份事实。
        LlmEvent::ThinkingSignature(signature) => {
            *thinking_signature = Some(signature);
        },
    }
}

fn contains_ascii_case_insensitive(haystack: &str, needle: &str) -> bool {
    let needle = needle.as_bytes();
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
}

/// 将 kernel 层的 `KernelError` 映射回 `AstrError`。
///
/// kernel 通过字符串前缀区分 LLM 错误类型（"LLM request failed"、"LLM stream error" 等），
/// 这里重建类型化的 `AstrError` 变体，让上层能精确匹配。
fn map_kernel_error(error: KernelError) -> AstrError {
    match error {
        KernelError::Validation(message) => AstrError::Validation(message),
        KernelError::NotFound(message) => AstrError::Internal(message),
        KernelError::Invoke(message) => {
            if contains_ascii_case_insensitive(&message, "llm request interrupted")
                || contains_ascii_case_insensitive(&message, "operation cancelled")
                || contains_ascii_case_insensitive(&message, "cancelled")
            {
                return AstrError::LlmInterrupted;
            }

            if let Some(raw) = message.strip_prefix("LLM request failed: ") {
                if let Some((status_raw, body)) = raw.split_once(" - ") {
                    if let Ok(status) = status_raw.trim().parse::<u16>() {
                        return AstrError::LlmRequestFailed {
                            status,
                            body: body.to_string(),
                        };
                    }
                }
                return AstrError::LlmRequestFailed {
                    status: 400,
                    body: raw.to_string(),
                };
            }

            if let Some(raw) = message.strip_prefix("LLM stream error: ") {
                return AstrError::LlmStreamError(raw.to_string());
            }

            if let Some(raw) = message.strip_prefix("network error: ") {
                return AstrError::Network(raw.to_string());
            }

            if let Some(raw) = message.strip_prefix("invalid api key for provider: ") {
                AstrError::InvalidApiKey(raw.to_string())
            } else {
                AstrError::Internal(message)
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use astrcode_core::{
        AgentEventContext, AstrError, LlmFinishReason, LlmOutput, ReasoningContent,
    };
    use astrcode_kernel::KernelError;

    use super::{
        StreamedToolCallDelta, emit_llm_delta_live, hydrate_reasoning_from_stream, map_kernel_error,
    };
    use crate::turn::test_support::test_session_state;

    #[test]
    fn map_kernel_error_restores_llm_request_failed_variant() {
        let mapped = map_kernel_error(KernelError::Invoke(
            "LLM request failed: 400 - invalid_request_error: messages 参数非法".to_string(),
        ));

        match mapped {
            AstrError::LlmRequestFailed { status, body } => {
                assert_eq!(status, 400);
                assert!(body.contains("messages 参数非法"));
            },
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[test]
    fn map_kernel_error_restores_llm_stream_error_variant() {
        let mapped = map_kernel_error(KernelError::Invoke(
            "LLM stream error: invalid_request_error: messages 参数非法".to_string(),
        ));

        match mapped {
            AstrError::LlmStreamError(message) => {
                assert!(message.contains("messages 参数非法"));
            },
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[test]
    fn emit_llm_delta_live_forwards_tool_call_delta_to_runner_sink() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let sink_received = Arc::clone(&received);
        let sink: super::ToolCallDeltaSink = Arc::new(move |delta: StreamedToolCallDelta| {
            sink_received
                .lock()
                .expect("tool delta sink lock should work")
                .push(delta);
        });

        let mut thinking_deltas = Vec::new();
        let mut thinking_signature = None;
        emit_llm_delta_live(
            astrcode_core::LlmEvent::ToolCallDelta {
                index: 0,
                id: Some("call-1".to_string()),
                name: Some("readFile".to_string()),
                arguments_delta: r#"{"path":"README.md"}"#.to_string(),
            },
            "turn-1",
            &AgentEventContext::default(),
            &test_session_state(),
            Some(&sink),
            &mut thinking_deltas,
            &mut thinking_signature,
        );

        assert_eq!(
            received
                .lock()
                .expect("tool delta sink lock should work")
                .as_slice(),
            &[StreamedToolCallDelta {
                index: 0,
                id: Some("call-1".to_string()),
                name: Some("readFile".to_string()),
                arguments_delta: r#"{"path":"README.md"}"#.to_string(),
            }]
        );
        assert!(thinking_deltas.is_empty());
        assert_eq!(thinking_signature, None);
    }

    #[test]
    fn emit_llm_delta_live_collects_thinking_for_durable_persistence() {
        let mut thinking_deltas = Vec::new();
        let mut thinking_signature = None;

        emit_llm_delta_live(
            astrcode_core::LlmEvent::ThinkingDelta("先检查状态".to_string()),
            "turn-1",
            &AgentEventContext::default(),
            &test_session_state(),
            None,
            &mut thinking_deltas,
            &mut thinking_signature,
        );
        emit_llm_delta_live(
            astrcode_core::LlmEvent::ThinkingSignature("sig-1".to_string()),
            "turn-1",
            &AgentEventContext::default(),
            &test_session_state(),
            None,
            &mut thinking_deltas,
            &mut thinking_signature,
        );

        assert_eq!(thinking_deltas, vec!["先检查状态".to_string()]);
        assert_eq!(thinking_signature.as_deref(), Some("sig-1"));
    }

    #[test]
    fn hydrate_reasoning_from_stream_backfills_missing_reasoning_content() {
        let mut output = LlmOutput {
            content: "done".to_string(),
            tool_calls: Vec::new(),
            reasoning: None,
            usage: None,
            finish_reason: LlmFinishReason::Stop,
            prompt_cache_diagnostics: None,
        };

        hydrate_reasoning_from_stream(
            &mut output,
            &["先检查".to_string(), "再修改".to_string()],
            Some("sig-1"),
        );

        assert_eq!(
            output.reasoning,
            Some(ReasoningContent {
                content: "先检查再修改".to_string(),
                signature: Some("sig-1".to_string()),
            })
        );
    }

    #[test]
    fn hydrate_reasoning_from_stream_preserves_existing_reasoning_and_backfills_signature() {
        let mut output = LlmOutput {
            content: "done".to_string(),
            tool_calls: Vec::new(),
            reasoning: Some(ReasoningContent {
                content: "最终 reasoning".to_string(),
                signature: None,
            }),
            usage: None,
            finish_reason: LlmFinishReason::Stop,
            prompt_cache_diagnostics: None,
        };

        hydrate_reasoning_from_stream(&mut output, &["流式 reasoning".to_string()], Some("sig-2"));

        assert_eq!(
            output.reasoning,
            Some(ReasoningContent {
                content: "最终 reasoning".to_string(),
                signature: Some("sig-2".to_string()),
            })
        );
    }
}
