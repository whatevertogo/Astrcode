use std::sync::Arc;

use astrcode_core::{CancelToken, Result};
use tokio::sync::mpsc;

use crate::llm::{EventSink, LlmEvent, LlmOutput, LlmProvider, LlmRequest};
use crate::provider_factory::DynProviderFactory;
use astrcode_core::StorageEvent;
use astrcode_core::{LlmMessage, ToolDefinition};

pub(crate) async fn build_provider(factory: DynProviderFactory) -> Result<Arc<dyn LlmProvider>> {
    Ok(tokio::task::spawn_blocking(move || factory.build())
        .await
        .map_err(|e| astrcode_core::AstrError::Internal(format!("blocking task failed: {e}")))??)
}

pub(crate) async fn generate_response(
    provider: &Arc<dyn LlmProvider>,
    messages: &[LlmMessage],
    tool_definitions: Vec<ToolDefinition>,
    turn_id: &str,
    system_prompt: Option<String>,
    cancel: CancelToken,
    on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
) -> Result<LlmOutput> {
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<LlmEvent>();
    let sink: EventSink = Arc::new(move |event| {
        let _ = event_tx.send(event);
    });
    let request = LlmRequest::new(messages.to_vec(), tool_definitions, cancel);
    let request = match system_prompt {
        Some(prompt) => request.with_system(prompt),
        None => request,
    };

    let generate_future = provider.generate(request, Some(sink));
    tokio::pin!(generate_future);

    let mut event_rx_open = true;
    let output = loop {
        tokio::select! {
            result = &mut generate_future => break result,
            maybe_event = event_rx.recv(), if event_rx_open => {
                match maybe_event {
                    Some(LlmEvent::TextDelta(text)) => {
                        #[cfg(debug_assertions)]
                        eprintln!("[delta] {}", text);
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
