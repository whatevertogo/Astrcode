use std::sync::Arc;

use anyhow::Result;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::action::{LlmMessage, ToolDefinition};
use crate::events::StorageEvent;
use crate::llm::{EventSink, LlmEvent, LlmOutput, LlmProvider, LlmRequest};
use crate::provider_factory::DynProviderFactory;

pub(crate) async fn build_provider(factory: DynProviderFactory) -> Result<Arc<dyn LlmProvider>> {
    tokio::task::spawn_blocking(move || factory.build()).await?
}

pub(crate) async fn generate_response(
    provider: &Arc<dyn LlmProvider>,
    messages: &[LlmMessage],
    tool_definitions: Vec<ToolDefinition>,
    system_prompt: Option<String>,
    cancel: CancellationToken,
    on_event: &mut impl FnMut(StorageEvent),
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
                        eprintln!("[delta] {}", text);
                        on_event(StorageEvent::AssistantDelta { token: text });
                    }
                    Some(LlmEvent::ToolCallDelta { .. }) => {}
                    None => event_rx_open = false,
                }
            }
        }
    };

    while let Ok(event) = event_rx.try_recv() {
        if let LlmEvent::TextDelta(text) = event {
            on_event(StorageEvent::AssistantDelta { token: text });
        }
    }

    output
}
