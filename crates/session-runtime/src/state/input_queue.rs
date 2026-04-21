use std::collections::{HashMap, HashSet};

use astrcode_core::{InputQueueProjection, Result, StorageEventPayload, StoredEvent, support};

use super::SessionState;

pub(crate) fn input_queue_projection_target_agent_id(
    payload: &StorageEventPayload,
) -> Option<&str> {
    match payload {
        StorageEventPayload::AgentInputQueued { payload } => Some(&payload.envelope.to_agent_id),
        StorageEventPayload::AgentInputBatchStarted { payload } => Some(&payload.target_agent_id),
        StorageEventPayload::AgentInputBatchAcked { payload } => Some(&payload.target_agent_id),
        StorageEventPayload::AgentInputDiscarded { payload } => Some(&payload.target_agent_id),
        _ => None,
    }
}

impl SessionState {
    /// 读取指定 agent 的 input queue durable 投影。
    pub fn input_queue_projection_for_agent(&self, agent_id: &str) -> Result<InputQueueProjection> {
        Ok(
            support::lock_anyhow(&self.projection_registry, "session projection registry")?
                .input_queue_projection_for_agent(agent_id),
        )
    }
}

pub(crate) fn apply_input_queue_event_to_index(
    index: &mut HashMap<String, InputQueueProjection>,
    stored: &StoredEvent,
) {
    let Some(target_agent_id) = input_queue_projection_target_agent_id(&stored.event.payload)
    else {
        return;
    };
    let projection = index.entry(target_agent_id.to_string()).or_default();
    apply_input_queue_event_for_agent(projection, stored, target_agent_id);
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn replay_input_queue_projection_for_agent(
    events: &[StoredEvent],
    target_agent_id: &str,
) -> InputQueueProjection {
    let mut projection = InputQueueProjection::default();
    for stored in events {
        apply_input_queue_event_for_agent(&mut projection, stored, target_agent_id);
    }
    projection
}

pub(crate) fn replay_input_queue_projection_index(
    events: &[StoredEvent],
) -> HashMap<String, InputQueueProjection> {
    let mut index = HashMap::new();
    for stored in events {
        match &stored.event.payload {
            StorageEventPayload::AgentInputQueued { payload } => {
                let target_agent_id = payload.envelope.to_agent_id.as_str();
                let projection = index.entry(target_agent_id.to_string()).or_default();
                apply_input_queue_event_for_agent(projection, stored, target_agent_id);
            },
            StorageEventPayload::AgentInputBatchStarted { payload } => {
                let target_agent_id = payload.target_agent_id.as_str();
                let projection = index.entry(target_agent_id.to_string()).or_default();
                apply_input_queue_event_for_agent(projection, stored, target_agent_id);
            },
            StorageEventPayload::AgentInputBatchAcked { payload } => {
                let target_agent_id = payload.target_agent_id.as_str();
                let projection = index.entry(target_agent_id.to_string()).or_default();
                apply_input_queue_event_for_agent(projection, stored, target_agent_id);
            },
            StorageEventPayload::AgentInputDiscarded { payload } => {
                let target_agent_id = payload.target_agent_id.as_str();
                let projection = index.entry(target_agent_id.to_string()).or_default();
                apply_input_queue_event_for_agent(projection, stored, target_agent_id);
            },
            _ => {},
        }
    }
    index
}

pub(crate) fn apply_input_queue_event_for_agent(
    projection: &mut InputQueueProjection,
    stored: &StoredEvent,
    target_agent_id: &str,
) {
    match &stored.event.payload {
        StorageEventPayload::AgentInputQueued { payload } => {
            if payload.envelope.to_agent_id != target_agent_id {
                return;
            }
            let id = &payload.envelope.delivery_id;
            if !projection.discarded_delivery_ids.contains(id)
                && !projection.pending_delivery_ids.contains(id)
            {
                projection.pending_delivery_ids.push(id.clone());
            }
        },
        StorageEventPayload::AgentInputBatchStarted { payload } => {
            if payload.target_agent_id != target_agent_id {
                return;
            }
            projection.active_batch_id = Some(payload.batch_id.clone());
            projection.active_delivery_ids = payload.delivery_ids.clone();
        },
        StorageEventPayload::AgentInputBatchAcked { payload } => {
            if payload.target_agent_id != target_agent_id {
                return;
            }
            let acked_set: HashSet<_> = payload.delivery_ids.iter().collect();
            projection.pending_delivery_ids.retain(|id| {
                !acked_set.contains(id) && !projection.discarded_delivery_ids.contains(id)
            });
            if projection.active_batch_id.as_deref() == Some(payload.batch_id.as_str()) {
                projection.active_batch_id = None;
                projection.active_delivery_ids.clear();
            }
        },
        StorageEventPayload::AgentInputDiscarded { payload } => {
            if payload.target_agent_id != target_agent_id {
                return;
            }
            for id in &payload.delivery_ids {
                if !projection.discarded_delivery_ids.contains(id) {
                    projection.discarded_delivery_ids.push(id.clone());
                }
            }
            projection
                .pending_delivery_ids
                .retain(|id| !projection.discarded_delivery_ids.contains(id));
            let discarded_set: HashSet<_> = projection.discarded_delivery_ids.iter().collect();
            if projection
                .active_delivery_ids
                .iter()
                .any(|id| discarded_set.contains(id))
            {
                projection.active_batch_id = None;
                projection.active_delivery_ids.clear();
            }
        },
        _ => {},
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{
        AgentEventContext, AgentLifecycleStatus, AgentTurnOutcome, InputBatchAckedPayload,
        InputBatchStartedPayload, InputDiscardedPayload, InputQueuedPayload, QueuedInputEnvelope,
        StorageEvent, StorageEventPayload,
    };

    use super::*;

    #[test]
    fn input_queue_projection_target_agent_id_reads_supported_payloads() {
        let payload = StorageEventPayload::AgentInputBatchStarted {
            payload: astrcode_core::InputBatchStartedPayload {
                target_agent_id: "agent-child".to_string(),
                turn_id: "turn-1".to_string(),
                batch_id: "batch-1".to_string(),
                delivery_ids: vec!["delivery-1".to_string().into()],
            },
        };

        assert_eq!(
            input_queue_projection_target_agent_id(&payload),
            Some("agent-child")
        );
    }

    #[test]
    fn replay_for_agent_tracks_full_lifecycle() {
        let agent = AgentEventContext::default();
        let queued = StoredEvent {
            storage_seq: 1,
            event: StorageEvent {
                turn_id: Some("t1".into()),
                agent: agent.clone(),
                payload: StorageEventPayload::AgentInputQueued {
                    payload: InputQueuedPayload {
                        envelope: QueuedInputEnvelope {
                            delivery_id: "d1".into(),
                            from_agent_id: "parent".into(),
                            to_agent_id: "child".into(),
                            message: "hello".into(),
                            queued_at: chrono::Utc::now(),
                            sender_lifecycle_status: AgentLifecycleStatus::Running,
                            sender_last_turn_outcome: None,
                            sender_open_session_id: "s-parent".into(),
                        },
                    },
                },
            },
        };
        let started = StoredEvent {
            storage_seq: 2,
            event: StorageEvent {
                turn_id: Some("t2".into()),
                agent: agent.clone(),
                payload: StorageEventPayload::AgentInputBatchStarted {
                    payload: InputBatchStartedPayload {
                        target_agent_id: "child".into(),
                        turn_id: "t2".into(),
                        batch_id: "b1".into(),
                        delivery_ids: vec!["d1".into()],
                    },
                },
            },
        };
        let acked = StoredEvent {
            storage_seq: 3,
            event: StorageEvent {
                turn_id: Some("t2".into()),
                agent,
                payload: StorageEventPayload::AgentInputBatchAcked {
                    payload: InputBatchAckedPayload {
                        target_agent_id: "child".into(),
                        turn_id: "t2".into(),
                        batch_id: "b1".into(),
                        delivery_ids: vec!["d1".into()],
                    },
                },
            },
        };

        let projection =
            replay_input_queue_projection_for_agent(&[queued, started, acked], "child");
        assert!(projection.pending_delivery_ids.is_empty());
        assert!(projection.active_batch_id.is_none());
        assert!(projection.active_delivery_ids.is_empty());
        assert_eq!(projection.pending_input_count(), 0);
    }

    #[test]
    fn replay_for_agent_tracks_discarded_entries() {
        let agent = AgentEventContext::default();
        let events = vec![
            StoredEvent {
                storage_seq: 1,
                event: StorageEvent {
                    turn_id: Some("t1".into()),
                    agent: agent.clone(),
                    payload: StorageEventPayload::AgentInputQueued {
                        payload: InputQueuedPayload {
                            envelope: QueuedInputEnvelope {
                                delivery_id: "d1".into(),
                                from_agent_id: "parent".into(),
                                to_agent_id: "child".into(),
                                message: "hello".into(),
                                queued_at: chrono::Utc::now(),
                                sender_lifecycle_status: AgentLifecycleStatus::Running,
                                sender_last_turn_outcome: None,
                                sender_open_session_id: "s-parent".into(),
                            },
                        },
                    },
                },
            },
            StoredEvent {
                storage_seq: 2,
                event: StorageEvent {
                    turn_id: Some("t1".into()),
                    agent,
                    payload: StorageEventPayload::AgentInputDiscarded {
                        payload: InputDiscardedPayload {
                            target_agent_id: "child".into(),
                            delivery_ids: vec!["d1".into()],
                        },
                    },
                },
            },
        ];

        let projection = replay_input_queue_projection_for_agent(&events, "child");
        assert!(projection.pending_delivery_ids.is_empty());
        assert!(projection.discarded_delivery_ids.contains(&"d1".into()));
    }

    #[test]
    fn replay_for_agent_keeps_started_but_unacked_delivery_pending() {
        let agent = AgentEventContext::default();
        let events = vec![
            StoredEvent {
                storage_seq: 1,
                event: StorageEvent {
                    turn_id: Some("t1".into()),
                    agent: agent.clone(),
                    payload: StorageEventPayload::AgentInputQueued {
                        payload: InputQueuedPayload {
                            envelope: QueuedInputEnvelope {
                                delivery_id: "d1".into(),
                                from_agent_id: "parent".into(),
                                to_agent_id: "child".into(),
                                message: "hello".into(),
                                queued_at: chrono::Utc::now(),
                                sender_lifecycle_status: AgentLifecycleStatus::Running,
                                sender_last_turn_outcome: None,
                                sender_open_session_id: "s-parent".into(),
                            },
                        },
                    },
                },
            },
            StoredEvent {
                storage_seq: 2,
                event: StorageEvent {
                    turn_id: Some("t2".into()),
                    agent,
                    payload: StorageEventPayload::AgentInputBatchStarted {
                        payload: InputBatchStartedPayload {
                            target_agent_id: "child".into(),
                            turn_id: "t2".into(),
                            batch_id: "b1".into(),
                            delivery_ids: vec!["d1".into()],
                        },
                    },
                },
            },
        ];

        let projection = replay_input_queue_projection_for_agent(&events, "child");
        assert!(projection.pending_delivery_ids.contains(&"d1".into()));
        assert_eq!(projection.active_batch_id.as_deref(), Some("b1"));
        assert_eq!(projection.pending_input_count(), 1);
    }

    #[test]
    fn replay_index_isolates_agents() {
        let agent = AgentEventContext::default();
        let events = vec![
            StoredEvent {
                storage_seq: 1,
                event: StorageEvent {
                    turn_id: Some("t1".into()),
                    agent: agent.clone(),
                    payload: StorageEventPayload::AgentInputQueued {
                        payload: InputQueuedPayload {
                            envelope: QueuedInputEnvelope {
                                delivery_id: "d-a".into(),
                                from_agent_id: "parent".into(),
                                to_agent_id: "agent-a".into(),
                                message: "for a".into(),
                                queued_at: chrono::Utc::now(),
                                sender_lifecycle_status: AgentLifecycleStatus::Running,
                                sender_last_turn_outcome: Some(AgentTurnOutcome::Completed),
                                sender_open_session_id: "s-parent".into(),
                            },
                        },
                    },
                },
            },
            StoredEvent {
                storage_seq: 2,
                event: StorageEvent {
                    turn_id: Some("t1".into()),
                    agent,
                    payload: StorageEventPayload::AgentInputQueued {
                        payload: InputQueuedPayload {
                            envelope: QueuedInputEnvelope {
                                delivery_id: "d-b".into(),
                                from_agent_id: "parent".into(),
                                to_agent_id: "agent-b".into(),
                                message: "for b".into(),
                                queued_at: chrono::Utc::now(),
                                sender_lifecycle_status: AgentLifecycleStatus::Running,
                                sender_last_turn_outcome: None,
                                sender_open_session_id: "s-parent".into(),
                            },
                        },
                    },
                },
            },
        ];

        let projection_index = replay_input_queue_projection_index(&events);
        assert_eq!(
            projection_index
                .get("agent-a")
                .expect("agent-a projection")
                .pending_delivery_ids,
            vec!["d-a".into()]
        );
        assert_eq!(
            projection_index
                .get("agent-b")
                .expect("agent-b projection")
                .pending_delivery_ids,
            vec!["d-b".into()]
        );
        assert!(!projection_index.contains_key("agent-c"));
    }
}
