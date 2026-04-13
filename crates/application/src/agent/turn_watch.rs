use std::time::Duration;

use astrcode_core::{Phase, SessionId, StorageEventPayload, StoredEvent};
use tokio::time::sleep;

use super::{AgentOrchestrationError, AgentOrchestrationService};

pub(super) struct TurnTerminalSnapshot {
    pub(super) phase: Phase,
    pub(super) events: Vec<StoredEvent>,
}

impl AgentOrchestrationService {
    pub(super) async fn wait_for_turn_terminal_snapshot(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> Result<TurnTerminalSnapshot, AgentOrchestrationError> {
        let session_id =
            SessionId::from(astrcode_session_runtime::normalize_session_id(session_id));
        loop {
            let state = self
                .session_runtime
                .get_session_state(&session_id)
                .await
                .map_err(AgentOrchestrationError::from)?;
            let phase = state
                .current_phase()
                .map_err(AgentOrchestrationError::from)?;
            if matches!(phase, Phase::Idle | Phase::Interrupted | Phase::Done) {
                let events = self
                    .session_runtime
                    .replay_stored_events(&session_id)
                    .await
                    .map_err(AgentOrchestrationError::from)?
                    .into_iter()
                    .filter(|stored| stored.event.turn_id() == Some(turn_id))
                    .collect::<Vec<_>>();
                if has_terminal_turn_signal(&events) || matches!(phase, Phase::Interrupted) {
                    return Ok(TurnTerminalSnapshot { phase, events });
                }
            }
            sleep(Duration::from_millis(20)).await;
        }
    }
}

fn has_terminal_turn_signal(events: &[StoredEvent]) -> bool {
    events.iter().any(|stored| {
        matches!(
            stored.event.payload,
            StorageEventPayload::TurnDone { .. } | StorageEventPayload::Error { .. }
        )
    })
}
