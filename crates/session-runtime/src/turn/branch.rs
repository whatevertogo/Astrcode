use std::{path::PathBuf, sync::Arc};

use astrcode_core::{
    AstrError, SessionId, SessionTurnAcquireResult, SessionTurnLease, StorageEventPayload,
    StoredEvent, event::generate_session_id,
};
use chrono::Utc;

use crate::{
    SessionRuntime, actor::SessionActor, catalog::SessionCatalogEvent,
    state::normalize_working_dir, turn::events::session_start_event,
};

pub(crate) struct SubmitTarget {
    pub(crate) session_id: SessionId,
    pub(crate) branched_from_session_id: Option<String>,
    pub(crate) actor: Arc<SessionActor>,
    pub(crate) turn_lease: Box<dyn SessionTurnLease>,
}

impl SessionRuntime {
    pub(crate) async fn resolve_submit_target(
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
                    let actor = self.ensure_loaded_session(&target_session_id).await?;
                    return Ok(SubmitTarget {
                        session_id: target_session_id,
                        branched_from_session_id,
                        actor,
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

    pub(crate) async fn try_resolve_submit_target_without_branch(
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
                let actor = self.ensure_loaded_session(session_id).await?;
                Ok(Some(SubmitTarget {
                    session_id: session_id.clone(),
                    branched_from_session_id: None,
                    actor,
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
        let source_actor = self.ensure_loaded_session(source_session_id).await?;
        let working_dir = normalize_working_dir(PathBuf::from(source_actor.working_dir()))?;
        let source_events = self.event_store.replay(source_session_id).await?;
        let stable_events = stable_events_before_active_turn(&source_events, active_turn_id);
        let parent_storage_seq = stable_events.last().map(|event| event.storage_seq);

        let branched_session_id: SessionId = generate_session_id().into();
        self.event_store
            .ensure_session(&branched_session_id, &working_dir)
            .await?;

        let session_start = session_start_event(
            branched_session_id.to_string(),
            working_dir.display().to_string(),
            Some(source_session_id.to_string()),
            parent_storage_seq,
            Utc::now(),
        );
        self.event_store
            .append(&branched_session_id, &session_start)
            .await?;

        // 为什么只复制稳定历史：活跃 turn 的半截输出不应污染新分支，
        // 否则 replay/context window 会同时看到未完成与新分支的事件。
        for stored in stable_events {
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
        Ok(branched_session_id)
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

fn stable_events_before_active_turn(
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
    use std::sync::Arc;

    use astrcode_core::{AstrError, SessionId, StorageEventPayload, StoredEvent};
    use chrono::Utc;

    use super::stable_events_before_active_turn;
    use crate::turn::{
        events::session_start_event,
        test_support::{BranchingTestEventStore, root_turn_event, test_runtime},
    };

    fn stored(
        storage_seq: u64,
        turn_id: Option<&str>,
        payload: StorageEventPayload,
    ) -> StoredEvent {
        StoredEvent {
            storage_seq,
            event: root_turn_event(turn_id, payload),
        }
    }
    #[test]
    fn stable_events_excludes_active_turn_tail() {
        let events = vec![
            StoredEvent {
                storage_seq: 1,
                event: session_start_event("session-1", "/tmp", None, None, Utc::now()),
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

    #[tokio::test]
    async fn resolve_submit_target_rejects_when_branch_depth_limit_is_exceeded() {
        let event_store = Arc::new(BranchingTestEventStore::default());
        let runtime = test_runtime(event_store.clone());
        let session = runtime
            .create_session(".")
            .await
            .expect("test session should be created");
        event_store.push_busy("turn-busy-1");
        event_store.push_busy("turn-busy-2");

        let error = match runtime
            .resolve_submit_target(&SessionId::from(session.session_id.clone()), "turn-new", 1)
            .await
        {
            Ok(_) => panic!("branch depth overflow should return validation error"),
            Err(error) => error,
        };

        match error {
            AstrError::Validation(message) => {
                assert!(message.contains("too many concurrent branch attempts"));
                assert!(message.contains("limit: 1"));
            },
            other => panic!("unexpected error: {other:?}"),
        }

        assert_eq!(
            runtime
                .list_session_metas()
                .await
                .expect("durable session metas should be readable")
                .len(),
            2,
            "first busy submit should still create one durable branched session before the depth \
             limit stops recursion"
        );
    }
}
