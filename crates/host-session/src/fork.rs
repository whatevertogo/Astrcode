use astrcode_core::{AstrError, SessionId, StorageEventPayload, StoredEvent};

use crate::SessionCatalog;

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

impl SessionCatalog {
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

        let source = self.ensure_loaded_session(source_session_id).await?;
        let new_session_id = self
            .fork_events_up_to(
                source_session_id,
                &source.working_dir,
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
    use astrcode_core::{AgentEventContext, StorageEvent, StorageEventPayload, StoredEvent};
    use chrono::Utc;

    use super::{ForkPoint, latest_stable_storage_seq, resolve_fork_point_storage_seq};
    use crate::branch::session_start_event;

    fn stored(
        storage_seq: u64,
        turn_id: Option<&str>,
        payload: StorageEventPayload,
    ) -> StoredEvent {
        StoredEvent {
            storage_seq,
            event: StorageEvent {
                turn_id: turn_id.map(str::to_string),
                agent: AgentEventContext::default(),
                payload,
            },
        }
    }

    #[test]
    fn latest_stable_storage_seq_stops_before_active_turn() {
        let events = vec![
            StoredEvent {
                storage_seq: 1,
                event: session_start_event("source".into(), ".".into(), None, None),
            },
            stored(
                2,
                Some("turn-1"),
                StorageEventPayload::UserMessage {
                    content: "hello".to_string(),
                    origin: astrcode_core::UserMessageOrigin::User,
                    timestamp: Utc::now(),
                },
            ),
            stored(
                3,
                Some("turn-1"),
                StorageEventPayload::TurnDone {
                    timestamp: Utc::now(),
                    terminal_kind: Some(astrcode_core::TurnTerminalKind::Completed),
                    reason: Some("completed".to_string()),
                },
            ),
            stored(
                4,
                Some("turn-2"),
                StorageEventPayload::UserMessage {
                    content: "still running".to_string(),
                    origin: astrcode_core::UserMessageOrigin::User,
                    timestamp: Utc::now(),
                },
            ),
        ];

        assert_eq!(latest_stable_storage_seq(&events), Some(3));
    }

    #[test]
    fn fork_point_accepts_completed_turn_end() {
        let events = vec![
            StoredEvent {
                storage_seq: 1,
                event: session_start_event("source".into(), ".".into(), None, None),
            },
            stored(
                2,
                Some("turn-1"),
                StorageEventPayload::UserMessage {
                    content: "hello".to_string(),
                    origin: astrcode_core::UserMessageOrigin::User,
                    timestamp: Utc::now(),
                },
            ),
            stored(
                3,
                Some("turn-1"),
                StorageEventPayload::TurnDone {
                    timestamp: Utc::now(),
                    terminal_kind: Some(astrcode_core::TurnTerminalKind::Completed),
                    reason: None,
                },
            ),
        ];

        assert_eq!(
            resolve_fork_point_storage_seq(
                &"source".to_string().into(),
                &events,
                &ForkPoint::TurnEnd("turn-1".to_string()),
            )
            .expect("completed turn should resolve"),
            3
        );
    }
}
