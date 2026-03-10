use tauri::ipc::Channel;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

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

        let mut runtime = self.runtime.lock().await;
        let cancel = cancel_token;
        let mut bridge = TurnEventBridge::new(turn_id);
        bridge.emit_thinking(&channel);

        let result = runtime
            .submit(text, cancel, |event| {
                bridge.forward_storage_event(&channel, event)
            })
            .await;

        if let Err(error) = result {
            eprintln!("agent turn error: {error}");
            return Err(error.to_string());
        }

        Ok(())
    }
}
