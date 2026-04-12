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
    AgentEventContext, CancelToken, LlmEvent, LlmRequest, Result, StorageEvent, StorageEventPayload,
};
use astrcode_kernel::KernelGateway;
use tokio::sync::mpsc;

/// 调用 LLM 并收集流式 delta 为 StorageEvent。
///
/// LLM 完成前推送的最后几个 delta 可能还在 channel 缓冲中，
/// 因此在 LLM 返回后还需 `try_recv()` 排空残余事件。
pub async fn call_llm_streaming(
    gateway: &KernelGateway,
    request: LlmRequest,
    turn_id: &str,
    agent: &AgentEventContext,
    cancel: &CancelToken,
    events: &mut Vec<StorageEvent>,
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
                    Some(event) => push_llm_delta(event, turn_id, agent, events),
                    None => event_rx_open = false,
                }
            }
        }

        if cancel.is_cancelled() {
            return Err(astrcode_core::AstrError::Internal("cancelled".to_string()));
        }
    };

    // 排空 channel 中残余事件：LLM 完成前推送的最后几个 delta
    while let Ok(event) = event_rx.try_recv() {
        push_llm_delta(event, turn_id, agent, events);
    }

    output.map_err(|e| astrcode_core::AstrError::Internal(e.to_string()))
}

/// 检查错误是否为 prompt-too-long 类型。
///
/// 不同 provider 使用不同的错误消息描述上下文长度溢出，
/// 此函数覆盖常见的几种表述方式。
pub fn is_prompt_too_long(error: &astrcode_core::AstrError) -> bool {
    let message = error.to_string();
    contains_ascii_case_insensitive(&message, "prompt too long")
        || contains_ascii_case_insensitive(&message, "context length")
        || contains_ascii_case_insensitive(&message, "maximum context")
        || contains_ascii_case_insensitive(&message, "too many tokens")
}

/// 将 LLM 流式增量事件转为 StorageEvent。
///
/// TextDelta → AssistantDelta, ThinkingDelta → ThinkingDelta。
/// ThinkingSignature（计费令牌）和 ToolCallDelta（provider 内部聚合）直接丢弃。
fn push_llm_delta(
    event: LlmEvent,
    turn_id: &str,
    agent: &AgentEventContext,
    events: &mut Vec<StorageEvent>,
) {
    match event {
        LlmEvent::TextDelta(text) => {
            events.push(StorageEvent {
                turn_id: Some(turn_id.to_string()),
                agent: agent.clone(),
                payload: StorageEventPayload::AssistantDelta { token: text },
            });
        },
        LlmEvent::ThinkingDelta(text) => {
            events.push(StorageEvent {
                turn_id: Some(turn_id.to_string()),
                agent: agent.clone(),
                payload: StorageEventPayload::ThinkingDelta { token: text },
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
