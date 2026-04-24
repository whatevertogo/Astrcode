use std::path::Path;

use astrcode_core::{
    AstrError, SessionId, SessionTurnAcquireResult, SessionTurnLease, StorageEvent,
    StorageEventPayload, StoredEvent, event::generate_session_id,
};
use chrono::Utc;

use crate::{SessionCatalog, SessionCatalogEvent};

pub struct SubmitTarget {
    pub session_id: SessionId,
    pub branched_from_session_id: Option<String>,
    pub turn_lease: Box<dyn SessionTurnLease>,
}

impl SessionCatalog {
    pub async fn resolve_submit_target(
        &self,
        session_id: &SessionId,
        turn_id: &str,
        max_branch_depth: usize,
    ) -> astrcode_core::Result<SubmitTarget> {
        self.ensure_session_exists(session_id).await?;

        let mut target_session_id = session_id.clone();
        let mut branched_from_session_id = None;
        let mut branch_depth = 0usize;
        let max_branch_depth = max_branch_depth.max(1);

        loop {
            match self
                .event_store
                .try_acquire_turn(&target_session_id, turn_id)
                .await?
            {
                SessionTurnAcquireResult::Acquired(turn_lease) => {
                    self.ensure_loaded_session(&target_session_id).await?;
                    return Ok(SubmitTarget {
                        session_id: target_session_id,
                        branched_from_session_id,
                        turn_lease,
                    });
                },
                SessionTurnAcquireResult::Busy(active_turn) => {
                    ensure_branch_depth_within_limit(branch_depth, max_branch_depth)?;
                    let source_session_id = target_session_id.clone();
                    target_session_id = self
                        .branch_session_from_busy_turn(&source_session_id, &active_turn.turn_id)
                        .await?;
                    branched_from_session_id = Some(source_session_id.to_string());
                    branch_depth += 1;
                },
            }
        }
    }

    pub async fn try_resolve_submit_target_without_branch(
        &self,
        session_id: &SessionId,
        turn_id: &str,
    ) -> astrcode_core::Result<Option<SubmitTarget>> {
        self.ensure_session_exists(session_id).await?;

        match self
            .event_store
            .try_acquire_turn(session_id, turn_id)
            .await?
        {
            SessionTurnAcquireResult::Acquired(turn_lease) => {
                self.ensure_loaded_session(session_id).await?;
                Ok(Some(SubmitTarget {
                    session_id: session_id.clone(),
                    branched_from_session_id: None,
                    turn_lease,
                }))
            },
            SessionTurnAcquireResult::Busy(_) => Ok(None),
        }
    }

    async fn branch_session_from_busy_turn(
        &self,
        source_session_id: &SessionId,
        active_turn_id: &str,
    ) -> astrcode_core::Result<SessionId> {
        let source_events = self.event_store.replay(source_session_id).await?;
        let stable_events = stable_events_before_active_turn(&source_events, active_turn_id);
        let source = self.ensure_loaded_session(source_session_id).await?;
        let parent_storage_seq = stable_events.last().map(|event| event.storage_seq);

        self.fork_events_up_to(
            source_session_id,
            &source.working_dir,
            &stable_events,
            parent_storage_seq,
        )
        .await
    }

    pub(crate) async fn fork_events_up_to(
        &self,
        source_session_id: &SessionId,
        working_dir: &Path,
        source_events: &[StoredEvent],
        parent_storage_seq: Option<u64>,
    ) -> astrcode_core::Result<SessionId> {
        let branched_session_id: SessionId = generate_session_id().into();
        self.event_store
            .ensure_session(&branched_session_id, working_dir)
            .await?;

        let session_start = session_start_event(
            branched_session_id.to_string(),
            working_dir.display().to_string(),
            Some(source_session_id.to_string()),
            parent_storage_seq,
        );
        self.event_store
            .append(&branched_session_id, &session_start)
            .await?;

        for stored in source_events {
            if matches!(
                stored.event.payload,
                StorageEventPayload::SessionStart { .. }
            ) {
                continue;
            }
            self.event_store
                .append(&branched_session_id, &stored.event)
                .await?;
        }

        let _ = self
            .catalog_events
            .send(SessionCatalogEvent::SessionBranched {
                session_id: branched_session_id.to_string(),
                source_session_id: source_session_id.to_string(),
            });
        let _ = self.ensure_loaded_session(&branched_session_id).await?;
        Ok(branched_session_id)
    }
}

pub(crate) fn session_start_event(
    session_id: String,
    working_dir: String,
    parent_session_id: Option<String>,
    parent_storage_seq: Option<u64>,
) -> StorageEvent {
    StorageEvent {
        turn_id: None,
        agent: astrcode_core::AgentEventContext::default(),
        payload: StorageEventPayload::SessionStart {
            session_id,
            timestamp: Utc::now(),
            working_dir,
            parent_session_id,
            parent_storage_seq,
        },
    }
}

fn ensure_branch_depth_within_limit(
    branch_depth: usize,
    max_branch_depth: usize,
) -> astrcode_core::Result<()> {
    if branch_depth >= max_branch_depth {
        return Err(AstrError::Validation(format!(
            "too many concurrent branch attempts (limit: {max_branch_depth})"
        )));
    }
    Ok(())
}

pub(crate) fn stable_events_before_active_turn(
    events: &[StoredEvent],
    active_turn_id: &str,
) -> Vec<StoredEvent> {
    let cutoff = events
        .iter()
        .position(|stored| stored.event.turn_id() == Some(active_turn_id))
        .unwrap_or(events.len());
    events[..cutoff].to_vec()
}

#[cfg(test)]
mod tests {
    use astrcode_core::{AgentEventContext, StorageEventPayload, StoredEvent};
    use chrono::Utc;

    use super::{session_start_event, stable_events_before_active_turn};

    fn stored(
        storage_seq: u64,
        turn_id: Option<&str>,
        payload: StorageEventPayload,
    ) -> StoredEvent {
        StoredEvent {
            storage_seq,
            event: astrcode_core::StorageEvent {
                turn_id: turn_id.map(str::to_string),
                agent: AgentEventContext::default(),
                payload,
            },
        }
    }

    #[test]
    fn stable_events_excludes_active_turn_tail() {
        let events = vec![
            StoredEvent {
                storage_seq: 1,
                event: session_start_event("session-1".into(), "/tmp".into(), None, None),
            },
            stored(
                2,
                Some("turn-stable"),
                StorageEventPayload::UserMessage {
                    content: "stable".to_string(),
                    origin: astrcode_core::UserMessageOrigin::User,
                    timestamp: Utc::now(),
                },
            ),
            stored(
                3,
                Some("turn-active"),
                StorageEventPayload::UserMessage {
                    content: "active".to_string(),
                    origin: astrcode_core::UserMessageOrigin::User,
                    timestamp: Utc::now(),
                },
            ),
        ];

        let stable = stable_events_before_active_turn(&events, "turn-active");

        assert_eq!(stable.len(), 2);
        assert_eq!(stable[0].storage_seq, 1);
        assert_eq!(stable[1].storage_seq, 2);
    }
}
