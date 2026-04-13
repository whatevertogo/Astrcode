use astrcode_core::{
    AgentEventContext, EventTranslator, Phase, Result, SessionId, StorageEvent, StorageEventPayload,
};
use chrono::Utc;

use crate::{SessionRuntime, state::append_and_broadcast};

impl SessionRuntime {
    pub async fn interrupt_session(&self, session_id: &str) -> Result<()> {
        let session_id = SessionId::from(crate::state::normalize_session_id(session_id));
        let actor = self.ensure_loaded_session(&session_id).await?;
        let is_running = actor
            .state()
            .running
            .load(std::sync::atomic::Ordering::SeqCst);
        let active_turn_id = actor
            .state()
            .active_turn_id
            .lock()
            .map_err(|_| astrcode_core::AstrError::LockPoisoned("session active turn".to_string()))?
            .clone();

        if !is_running || active_turn_id.is_none() {
            return Ok(());
        }

        let cancel = actor
            .state()
            .cancel
            .lock()
            .map_err(|_| astrcode_core::AstrError::LockPoisoned("session cancel".to_string()))?
            .clone();
        cancel.cancel();

        if let Some(active_turn_id) = active_turn_id.as_deref() {
            let cancelled = self.kernel.cancel_subruns_for_turn(active_turn_id).await;
            if !cancelled.is_empty() {
                log::info!(
                    "cancelled {} subruns for interrupted turn '{}'",
                    cancelled.len(),
                    active_turn_id
                );
            }
        }

        let mut translator = EventTranslator::new(actor.state().current_phase()?);
        let event = StorageEvent {
            turn_id: active_turn_id,
            agent: AgentEventContext::default(),
            payload: StorageEventPayload::Error {
                message: "interrupted".to_string(),
                timestamp: Some(Utc::now()),
            },
        };
        append_and_broadcast(actor.state(), &event, &mut translator).await?;
        crate::state::complete_session_execution(actor.state(), Phase::Interrupted);
        Ok(())
    }
}
