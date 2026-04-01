use std::path::PathBuf;
use std::sync::Arc;

use astrcode_core::{CancelToken, ModelRequest, Result};
use tokio::sync::mpsc;

use crate::llm::{EventSink, LlmEvent, LlmOutput, LlmProvider, LlmRequest};
use crate::provider_factory::DynProviderFactory;
use astrcode_core::StorageEvent;
pub(crate) async fn build_provider(
    factory: DynProviderFactory,
    working_dir: Option<PathBuf>,
) -> Result<Arc<dyn LlmProvider>> {
    tokio::task::spawn_blocking(move || factory.build_for_working_dir(working_dir))
        .await
        .map_err(|e| astrcode_core::AstrError::Internal(format!("blocking task failed: {e}")))?
}

/// 调用 LLM 提供者并实时转发流式增量事件。
///
/// ## 架构模式：unbounded channel + select + drain
///
/// LLM 提供者的 `generate()` 方法接受一个 `EventSink` 回调来推送流式事件。
/// 但我们需要在接收事件的同时等待 `generate()` 完成并获取最终结果。
/// 使用 `tokio::select!` 同时等待这两个源：
/// - `generate_future` 完成 → 返回最终 `LlmOutput`
/// - `event_rx.recv()` → 实时转发增量 delta 为 `StorageEvent`
///
/// `generate()` 可能在返回结果之前推送最后几个事件到 channel，
/// 所以下方用 `try_recv()` 循环排空 channel 中残余事件。
pub(crate) async fn generate_response(
    provider: &Arc<dyn LlmProvider>,
    request: ModelRequest,
    turn_id: &str,
    cancel: CancelToken,
    on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
) -> Result<LlmOutput> {
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<LlmEvent>();
    let sink: EventSink = Arc::new(move |event| {
        let _ = event_tx.send(event);
    });
    let request = LlmRequest::from_model_request(request, cancel);

    let generate_future = provider.generate(request, Some(sink));
    tokio::pin!(generate_future);

    let mut event_rx_open = true;
    let output = loop {
        tokio::select! {
            result = &mut generate_future => break result,
            maybe_event = event_rx.recv(), if event_rx_open => {
                match maybe_event {
                    Some(LlmEvent::TextDelta(text)) => {
                        log::debug!("[delta] {}", text);
                        on_event(StorageEvent::AssistantDelta {
                            turn_id: Some(turn_id.to_string()),
                            token: text,
                        })?;
                    }
                    Some(LlmEvent::ThinkingDelta(text)) => {
                        on_event(StorageEvent::ThinkingDelta {
                            turn_id: Some(turn_id.to_string()),
                            token: text,
                        })?;
                    }
                    Some(LlmEvent::ThinkingSignature(_)) => {}
                    Some(LlmEvent::ToolCallDelta { .. }) => {}
                    None => event_rx_open = false,
                }
            }
        }
    };

    while let Ok(event) = event_rx.try_recv() {
        match event {
            LlmEvent::TextDelta(text) => {
                on_event(StorageEvent::AssistantDelta {
                    turn_id: Some(turn_id.to_string()),
                    token: text,
                })?;
            }
            LlmEvent::ThinkingDelta(text) => {
                on_event(StorageEvent::ThinkingDelta {
                    turn_id: Some(turn_id.to_string()),
                    token: text,
                })?;
            }
            LlmEvent::ThinkingSignature(_) | LlmEvent::ToolCallDelta { .. } => {}
        }
    }

    output
}
