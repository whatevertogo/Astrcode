use astrcode_core::{
    AstrError, EventTranslator, Phase, StorageEvent, StoredEvent, UserMessageOrigin,
};
use astrcode_runtime_agent_loop::CompactionTailSnapshot;
use astrcode_runtime_session::append_and_broadcast;

use crate::service::{RuntimeService, ServiceError, ServiceResult, blocking_bridge::lock_anyhow};

impl RuntimeService {
    pub async fn compact_session(&self, session_id: &str) -> ServiceResult<()> {
        let session_id = astrcode_runtime_session::normalize_session_id(session_id);
        let session = self.ensure_session_loaded(&session_id).await?;
        if session.running.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(ServiceError::Conflict(format!(
                "session '{}' is busy; manual compact is only allowed while idle",
                session_id
            )));
        }

        let loop_ = self.current_loop().await;
        let projected = session
            .snapshot_projected_state()
            .map_err(|error| ServiceError::Internal(AstrError::Internal(error.to_string())))?;
        let recent_stored_events = session
            .snapshot_recent_stored_events()
            .map_err(|error| ServiceError::Internal(AstrError::Internal(error.to_string())))?;
        let compaction_tail = CompactionTailSnapshot::from_seed(recent_turn_event_tail(
            &recent_stored_events,
            loop_.compact_keep_recent_turns(),
        ));
        let compact_event = loop_
            .manual_compact_event(&projected, compaction_tail, Some(&recent_stored_events))
            .await
            .map_err(ServiceError::from)?;

        let Some(compact_event) = compact_event else {
            if let Ok(mut failures) =
                lock_anyhow(&session.compact_failure_count, "compact failures")
            {
                *failures = 0;
            }
            return Err(ServiceError::InvalidInput(
                "manual compact found no compressible history; the session needs at least 2 user \
                 turns before it can be compacted"
                    .to_string(),
            ));
        };

        let initial_phase = lock_anyhow(&session.phase, "session phase")
            .map(|guard| *guard)
            .unwrap_or(Phase::Idle);
        let mut translator = EventTranslator::new(initial_phase);
        append_and_broadcast(&session, &compact_event, &mut translator)
            .await
            .map_err(|error| ServiceError::Internal(AstrError::Internal(error.to_string())))?;
        if let Ok(mut phase) = lock_anyhow(&session.phase, "session phase") {
            *phase = translator.phase();
        }
        if let Ok(mut failures) = lock_anyhow(&session.compact_failure_count, "compact failures") {
            *failures = 0;
        }
        Ok(())
    }
}

/// Manual / auto compact 都应该基于 durable tail，而不是投影后的消息列表。
pub(super) fn recent_turn_event_tail(
    events: &[StoredEvent],
    keep_recent_turns: usize,
) -> Vec<StoredEvent> {
    let tail_events = events
        .iter()
        .filter(|stored| should_record_compaction_tail_event(&stored.event))
        .cloned()
        .collect::<Vec<_>>();

    let user_turn_indices = tail_events
        .iter()
        .enumerate()
        .filter_map(|(index, stored)| match &stored.event {
            StorageEvent::UserMessage {
                origin: UserMessageOrigin::User,
                ..
            } => Some(index),
            _ => None,
        })
        .collect::<Vec<_>>();

    let Some(&keep_start) = user_turn_indices
        .iter()
        .rev()
        .nth(keep_recent_turns.saturating_sub(1))
    else {
        return tail_events;
    };

    tail_events[keep_start..].to_vec()
}

fn should_record_compaction_tail_event(event: &StorageEvent) -> bool {
    matches!(
        event,
        StorageEvent::UserMessage { .. }
            | StorageEvent::AssistantFinal { .. }
            | StorageEvent::ToolResult { .. }
    )
}
