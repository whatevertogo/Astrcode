//! # LLM 调用周期 (LLM Cycle)
//!
//! 负责与 LLM Provider 交互，包括：
//! - Provider 构建（根据工作目录解析配置）
//! - 流式响应处理（通过 unbounded channel + select 模式）
//! - 增量事件转发（将 LLM delta 转换为 StorageEvent）
//!
//! ## 架构模式
//!
//! LLM Provider 的 `generate()` 方法接受一个 `EventSink` 回调来推送流式事件。
//! 但我们需要在接收事件的同时等待 `generate()` 完成并获取最终结果。
//! 使用 `tokio::select!` 同时等待这两个源：
//! - `generate_future` 完成 → 返回最终 `LlmOutput`
//! - `event_rx.recv()` → 实时转发增量 delta 为 `StorageEvent`
//!
//! ## 为什么使用 unbounded channel
//!
//! 生产者（LLM 流式传输）受网络 I/O 带宽约束，消费者（select! 循环）
//! 以同等速度处理事件，因此缓冲区中积压的数据始终只是少量未处理的 delta。
//! 若使用 bounded channel，反压逻辑会不必要地复杂化代码。

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
    if factory.build_requires_blocking_pool() {
        // 配置文件 factory 会同步读取磁盘，因此需要切到 blocking pool，
        // 避免把 tokio worker 卡在 fs I/O 上。
        tokio::task::spawn_blocking(move || factory.build_for_working_dir(working_dir))
            .await
            .map_err(|e| astrcode_core::AstrError::Internal(format!("blocking task failed: {e}")))?
    } else {
        // 纯内存 factory（尤其测试桩）直接构建，避免 spawn_blocking 的调度抖动
        // 破坏流式 delta 的时序断言。
        factory.build_for_working_dir(working_dir)
    }
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
    // 使用 unbounded channel 而非 bounded 是安全的：生产者（LLM 流式传输）
    // 受网络 I/O 带宽约束，消费者（select! 循环）以同等速度处理事件，
    // 因此缓冲区中积压的数据始终只是少量未处理的 delta（几 KB 级别）。
    // 若使用 bounded channel，反压逻辑会不必要地复杂化代码。
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
                    // ThinkingSignature 是 Anthropic API 用于计费验证的不透明令牌，
                    // 不含可显示内容，直接丢弃。ToolCallDelta 由 provider 内部
                    // 聚合到最终 LlmOutput 中，也无需在此处理。
                    // TODO:也许anthropic和openai格式需要统一一下内容和逻辑
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
            // 同上：drain 阶段也丢弃这两种无需转发的事件类型
            LlmEvent::ThinkingSignature(_) | LlmEvent::ToolCallDelta { .. } => {}
        }
    }

    output
}
