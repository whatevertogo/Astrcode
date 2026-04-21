use std::path::PathBuf;

use astrcode_core::{AstrError, SessionId, StorageEventPayload, StoredEvent};

use crate::{SessionRuntime, state::normalize_working_dir};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForkPoint {
    StorageSeq(u64),
    TurnEnd(String),
    Latest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForkResult {
    pub new_session_id: SessionId,
    pub fork_point_storage_seq: u64,
    pub events_copied: usize,
}

impl SessionRuntime {
    pub async fn fork_session(
        &self,
        source_session_id: &SessionId,
        fork_point: ForkPoint,
    ) -> astrcode_core::Result<ForkResult> {
        self.ensure_session_exists(source_session_id).await?;

        let source_events = self.event_store.replay(source_session_id).await?;
        let fork_point_storage_seq =
            resolve_fork_point_storage_seq(source_session_id, &source_events, &fork_point)?;
        let events_to_copy =
            stable_events_up_to_storage_seq(&source_events, fork_point_storage_seq)?;

        let source_actor = self.ensure_loaded_session(source_session_id).await?;
        let working_dir = normalize_working_dir(PathBuf::from(source_actor.working_dir()))?;
        let new_session_id = self
            .fork_events_up_to(
                source_session_id,
                &working_dir,
                &events_to_copy,
                Some(fork_point_storage_seq),
            )
            .await?;

        Ok(ForkResult {
            new_session_id,
            fork_point_storage_seq,
            events_copied: events_to_copy
                .iter()
                .filter(|stored| {
                    !matches!(
                        stored.event.payload,
                        StorageEventPayload::SessionStart { .. }
                    )
                })
                .count(),
        })
    }
}

fn resolve_fork_point_storage_seq(
    source_session_id: &SessionId,
    events: &[StoredEvent],
    fork_point: &ForkPoint,
) -> astrcode_core::Result<u64> {
    match fork_point {
        ForkPoint::Latest => latest_stable_storage_seq(events).ok_or_else(|| {
            AstrError::Validation(format!(
                "session '{}' has no stable fork point",
                source_session_id
            ))
        }),
        ForkPoint::StorageSeq(storage_seq) => {
            if !events
                .iter()
                .any(|stored| stored.storage_seq == *storage_seq)
            {
                return Err(AstrError::Validation(format!(
                    "storage_seq {} is out of range for session '{}'",
                    storage_seq, source_session_id
                )));
            }
            let _ = stable_events_up_to_storage_seq(events, *storage_seq)?;
            Ok(*storage_seq)
        },
        ForkPoint::TurnEnd(turn_id) => {
            resolve_turn_end_storage_seq(source_session_id, events, turn_id)
        },
    }
}

fn resolve_turn_end_storage_seq(
    source_session_id: &SessionId,
    events: &[StoredEvent],
    turn_id: &str,
) -> astrcode_core::Result<u64> {
    let turn_exists = events
        .iter()
        .any(|stored| stored.event.turn_id.as_deref() == Some(turn_id));
    if !turn_exists {
        return Err(AstrError::SessionNotFound(format!(
            "turn '{}' in session '{}'",
            turn_id, source_session_id
        )));
    }

    events
        .iter()
        .find_map(|stored| match &stored.event.payload {
            StorageEventPayload::TurnDone { .. }
                if stored.event.turn_id.as_deref() == Some(turn_id) =>
            {
                Some(stored.storage_seq)
            },
            _ => None,
        })
        .ok_or_else(|| {
            AstrError::Validation(format!(
                "turn '{}' has not completed and cannot be used as a fork point",
                turn_id
            ))
        })
}

fn latest_stable_storage_seq(events: &[StoredEvent]) -> Option<u64> {
    let mut latest = None;
    for stored in events {
        if matches!(
            stored.event.payload,
            StorageEventPayload::SessionStart { .. }
        ) {
            latest = Some(stored.storage_seq);
        }
        if matches!(
            stored.event.payload,
            StorageEventPayload::TurnDone { .. } | StorageEventPayload::Error { .. }
        ) {
            latest = Some(stored.storage_seq);
        }
    }
    latest
}

fn stable_events_up_to_storage_seq(
    events: &[StoredEvent],
    storage_seq: u64,
) -> astrcode_core::Result<Vec<StoredEvent>> {
    let cutoff = events
        .iter()
        .position(|stored| stored.storage_seq == storage_seq)
        .ok_or_else(|| {
            AstrError::Validation(format!("storage_seq {} is out of range", storage_seq))
        })?;
    let candidate = events[..=cutoff].to_vec();

    if is_stable_prefix(&candidate) {
        Ok(candidate)
    } else {
        Err(AstrError::Validation(format!(
            "storage_seq {} is inside an unfinished turn and cannot be used as a fork point",
            storage_seq
        )))
    }
}

fn is_stable_prefix(events: &[StoredEvent]) -> bool {
    let mut active_turn_id: Option<&str> = None;
    for stored in events {
        let Some(turn_id) = stored.event.turn_id.as_deref() else {
            continue;
        };
        match &stored.event.payload {
            StorageEventPayload::TurnDone { .. } | StorageEventPayload::Error { .. } => {
                if active_turn_id == Some(turn_id) {
                    active_turn_id = None;
                }
            },
            _ => {
                if active_turn_id.is_none() {
                    active_turn_id = Some(turn_id);
                }
            },
        }
    }
    active_turn_id.is_none()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use astrcode_core::{AstrError, SessionId, StoredEvent};
    use chrono::Utc;

    use super::{ForkPoint, latest_stable_storage_seq};
    use crate::{
        SessionRuntime,
        turn::{
            events::session_start_event,
            test_support::{
                BranchingTestEventStore, root_assistant_final_event, root_turn_done_event,
                root_turn_event, root_user_message_event, test_runtime,
            },
        },
    };

    fn seed_runtime_with_events(
        events: Vec<StoredEvent>,
    ) -> (SessionRuntime, Arc<BranchingTestEventStore>) {
        let event_store = Arc::new(BranchingTestEventStore::default());
        event_store.seed_session("source", ".", events);
        (test_runtime(event_store.clone()), event_store)
    }

    #[test]
    fn latest_stable_storage_seq_stops_before_active_turn() {
        let events = vec![
            StoredEvent {
                storage_seq: 1,
                event: session_start_event("source", ".", None, None, Utc::now()),
            },
            StoredEvent {
                storage_seq: 2,
                event: root_user_message_event("turn-1", "hello"),
            },
            StoredEvent {
                storage_seq: 3,
                event: root_turn_done_event("turn-1", Some("completed".to_string())),
            },
            StoredEvent {
                storage_seq: 4,
                event: root_user_message_event("turn-2", "still running"),
            },
        ];

        assert_eq!(latest_stable_storage_seq(&events), Some(3));
    }

    #[tokio::test]
    async fn fork_session_latest_on_idle_copies_all_stable_events() {
        let events = vec![
            StoredEvent {
                storage_seq: 1,
                event: session_start_event("source", ".", None, None, Utc::now()),
            },
            StoredEvent {
                storage_seq: 2,
                event: root_user_message_event("turn-1", "hello"),
            },
            StoredEvent {
                storage_seq: 3,
                event: root_assistant_final_event("turn-1", "world"),
            },
            StoredEvent {
                storage_seq: 4,
                event: root_turn_done_event("turn-1", Some("completed".to_string())),
            },
        ];
        let (runtime, event_store) = seed_runtime_with_events(events);

        let result = runtime
            .fork_session(&SessionId::from("source".to_string()), ForkPoint::Latest)
            .await
            .expect("fork latest should succeed");

        assert_eq!(result.fork_point_storage_seq, 4);
        assert_eq!(result.events_copied, 3);

        let new_events = event_store.stored_events_for(result.new_session_id.as_str());
        assert_eq!(new_events.len(), 4);
        let metas = runtime
            .list_session_metas()
            .await
            .expect("metas should be listable");
        let new_meta = metas
            .into_iter()
            .find(|meta| meta.session_id == result.new_session_id.as_str())
            .expect("new session meta should exist");
        assert_eq!(new_meta.parent_session_id.as_deref(), Some("source"));
        assert_eq!(new_meta.parent_storage_seq, Some(4));
        assert_eq!(new_meta.phase, astrcode_core::Phase::Idle);
    }

    #[tokio::test]
    async fn fork_session_accepts_completed_turn_end_and_stable_storage_seq() {
        let events = vec![
            StoredEvent {
                storage_seq: 1,
                event: session_start_event("source", ".", None, None, Utc::now()),
            },
            StoredEvent {
                storage_seq: 2,
                event: root_user_message_event("turn-1", "hello"),
            },
            StoredEvent {
                storage_seq: 3,
                event: root_assistant_final_event("turn-1", "world"),
            },
            StoredEvent {
                storage_seq: 4,
                event: root_turn_done_event("turn-1", Some("completed".to_string())),
            },
            StoredEvent {
                storage_seq: 5,
                event: root_user_message_event("turn-2", "next"),
            },
            StoredEvent {
                storage_seq: 6,
                event: root_turn_done_event("turn-2", Some("completed".to_string())),
            },
        ];
        let (runtime, _) = seed_runtime_with_events(events);

        let from_turn = runtime
            .fork_session(
                &SessionId::from("source".to_string()),
                ForkPoint::TurnEnd("turn-1".to_string()),
            )
            .await
            .expect("completed turn should be accepted");
        assert_eq!(from_turn.fork_point_storage_seq, 4);

        let from_seq = runtime
            .fork_session(
                &SessionId::from("source".to_string()),
                ForkPoint::StorageSeq(4),
            )
            .await
            .expect("stable storage seq should be accepted");
        assert_eq!(from_seq.fork_point_storage_seq, 4);
    }

    #[tokio::test]
    async fn fork_session_latest_on_thinking_truncates_to_last_stable_turn() {
        let events = vec![
            StoredEvent {
                storage_seq: 1,
                event: session_start_event("source", ".", None, None, Utc::now()),
            },
            StoredEvent {
                storage_seq: 2,
                event: root_user_message_event("turn-1", "hello"),
            },
            StoredEvent {
                storage_seq: 3,
                event: root_turn_done_event("turn-1", Some("completed".to_string())),
            },
            StoredEvent {
                storage_seq: 4,
                event: root_user_message_event("turn-2", "unfinished"),
            },
            StoredEvent {
                storage_seq: 5,
                event: root_turn_event(
                    Some("turn-2"),
                    astrcode_core::StorageEventPayload::AssistantFinal {
                        content: "partial".to_string(),
                        reasoning_content: None,
                        reasoning_signature: None,
                        step_index: None,
                        timestamp: Some(Utc::now()),
                    },
                ),
            },
        ];
        let (runtime, event_store) = seed_runtime_with_events(events);

        let result = runtime
            .fork_session(&SessionId::from("source".to_string()), ForkPoint::Latest)
            .await
            .expect("fork latest should succeed");

        assert_eq!(result.fork_point_storage_seq, 3);
        let new_events = event_store.stored_events_for(result.new_session_id.as_str());
        assert_eq!(new_events.len(), 3);
    }

    #[tokio::test]
    async fn fork_session_rejects_unfinished_turn_and_active_storage_seq() {
        let events = vec![
            StoredEvent {
                storage_seq: 1,
                event: session_start_event("source", ".", None, None, Utc::now()),
            },
            StoredEvent {
                storage_seq: 2,
                event: root_user_message_event("turn-1", "hello"),
            },
        ];
        let (runtime, _) = seed_runtime_with_events(events);

        let unfinished_turn = runtime
            .fork_session(
                &SessionId::from("source".to_string()),
                ForkPoint::TurnEnd("turn-1".to_string()),
            )
            .await
            .expect_err("unfinished turn should be rejected");
        assert!(matches!(unfinished_turn, AstrError::Validation(_)));

        let active_seq = runtime
            .fork_session(
                &SessionId::from("source".to_string()),
                ForkPoint::StorageSeq(2),
            )
            .await
            .expect_err("active storage seq should be rejected");
        assert!(matches!(active_seq, AstrError::Validation(_)));
    }

    #[tokio::test]
    async fn fork_session_rejects_unknown_turn_id() {
        let events = vec![StoredEvent {
            storage_seq: 1,
            event: session_start_event("source", ".", None, None, Utc::now()),
        }];
        let (runtime, _) = seed_runtime_with_events(events);

        let error = runtime
            .fork_session(
                &SessionId::from("source".to_string()),
                ForkPoint::TurnEnd("turn-missing".to_string()),
            )
            .await
            .expect_err("missing turn should be rejected");

        assert!(matches!(error, AstrError::SessionNotFound(_)));
    }
}
