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
    AgentEvent, AgentEventContext, AstrError, CancelToken, LlmEvent, LlmRequest, Result,
};
use astrcode_kernel::{KernelError, KernelGateway};
use tokio::sync::mpsc;

use crate::SessionState;

/// 调用 LLM 并收集流式 delta 为 StorageEvent。
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
) -> Result<astrcode_core::LlmOutput> {
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<LlmEvent>();

    let sink: astrcode_core::LlmEventSink = Arc::new(move |event| {
        let _ = event_tx.send(event);
    });

    let generate_future = gateway.call_llm(request, Some(sink));
    tokio::pin!(generate_future);

    let mut event_rx_open = true;
    let output = loop {
        tokio::select! {
            result = &mut generate_future => break result,
            maybe_event = event_rx.recv(), if event_rx_open => {
                match maybe_event {
                    Some(event) => emit_llm_delta_live(event, turn_id, agent, session_state),
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
        emit_llm_delta_live(event, turn_id, agent, session_state);
    }

    output.map_err(map_kernel_error)
}

/// 检查错误是否为 prompt-too-long 类型。
///
/// 不同 provider 使用不同的错误消息描述上下文长度溢出，
/// 此函数覆盖常见的几种表述方式。
pub fn is_prompt_too_long(error: &astrcode_core::AstrError) -> bool {
    error.is_prompt_too_long()
}

/// 将 LLM 流式增量直接发到 live-only 广播通道。
///
/// 为什么不再把 token 级 delta 塞进 turn 结束后的 durable 批量事件：
/// 旧实现会等整轮 run_turn 完成后才 append，导致前端只能在结束时一次性看到全文。
/// live delta 负责“即时吐字”，durable 真相继续由 AssistantFinal / TurnDone 承担。
fn emit_llm_delta_live(
    event: LlmEvent,
    turn_id: &str,
    agent: &AgentEventContext,
    session_state: &SessionState,
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
            session_state.broadcast_live_event(AgentEvent::ThinkingDelta {
                turn_id: turn_id.to_string(),
                agent: agent.clone(),
                delta: text,
            });
        },
        // ThinkingSignature 是 Anthropic API 计费验证令牌，ToolCallDelta 由 provider 内部聚合。
        LlmEvent::ThinkingSignature(_) | LlmEvent::ToolCallDelta { .. } => {},
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
    use astrcode_core::AstrError;
    use astrcode_kernel::KernelError;

    use super::map_kernel_error;

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
}
