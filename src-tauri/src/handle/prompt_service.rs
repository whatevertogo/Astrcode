use std::sync::{Arc, Mutex as StdMutex};

use tauri::ipc::Channel;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use astrcode_core::llm::{EventSink, LlmEvent};
use ::ipc::AgentEvent;

use super::ipc::TurnEventBridge;
use super::AgentHandle;

impl AgentHandle {
    pub async fn submit_prompt(
        &self,
        text: String,
        channel: Channel<AgentEvent>,
    ) -> Result<(), String> {
        {
            let mut guard = self.cancel.lock().await;
            if let Some(prev) = guard.take() {
                prev.cancel();
            }
        }

        let turn_id = Uuid::new_v4().to_string();
        let cancel_token = CancellationToken::new();

        {
            let mut guard = self.cancel.lock().await;
            *guard = Some(cancel_token.clone());
        }

        let session_id = self.get_session_id().await;
        let local_cache = self
            .reasoning_cache
            .lock()
            .await
            .get(&session_id)
            .cloned()
            .unwrap_or_default();

        let mut runtime = self.runtime.lock().await;
        let cancel = cancel_token;
        runtime.replace_reasoning_cache(local_cache);
        let bridge = Arc::new(StdMutex::new(TurnEventBridge::new(turn_id)));
        let channel = Arc::new(StdMutex::new(channel));
        {
            let bridge = bridge.lock().expect("turn bridge lock");
            let channel = channel.lock().expect("agent event channel lock");
            bridge.emit_thinking(&channel);
        }

        let transient_bridge = bridge.clone();
        let transient_channel = channel.clone();
        let transient_sink: EventSink = Arc::new(move |event| {
            if matches!(event, LlmEvent::ThinkingDelta(_)) {
                let mut bridge = transient_bridge.lock().expect("turn bridge lock");
                let channel = transient_channel
                    .lock()
                    .expect("agent event channel lock");
                bridge.forward_llm_event(&channel, &event);
            }
        });
        runtime.set_transient_llm_sink(Some(transient_sink));

        let result = runtime
            .submit(text, cancel, |event| {
                let mut bridge = bridge.lock().expect("turn bridge lock");
                let channel = channel.lock().expect("agent event channel lock");
                bridge.forward_storage_event(&channel, event)
            })
            .await;
        let updated_cache = runtime.reasoning_cache_snapshot();
        runtime.set_transient_llm_sink(None);
        drop(runtime);

        self.reasoning_cache
            .lock()
            .await
            .insert(session_id, updated_cache);

        if let Err(error) = result {
            eprintln!("agent turn error: {error}");
            return Err(error.to_string());
        }

        Ok(())
    }
}
