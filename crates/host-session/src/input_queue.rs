use std::collections::{HashMap, HashSet};

use astrcode_core::{StorageEventPayload, StoredEvent};
use serde::{Deserialize, Serialize};

/// `host-session` 对外暴露的 input queue owner bridge。
///
/// 新调用方必须从 `host-session` 导入它，避免把 input queue read-model 合同挂在 core 顶层。
pub type InputQueueProjection = astrcode_core::agent::input_queue::InputQueueProjection;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum InputKind {
    #[default]
    User,
    ParentSubrun,
    FollowUp,
}

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

pub(crate) fn replay_input_queue_projection_index(
    events: &[StoredEvent],
) -> HashMap<String, InputQueueProjection> {
    let mut index = HashMap::new();
    for stored in events {
        apply_input_queue_event_to_index(&mut index, stored);
    }
    index
}

fn apply_input_queue_event_for_agent(
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
        AgentEventContext, AgentLifecycleStatus, InputQueuedPayload, QueuedInputEnvelope,
        StorageEvent, StorageEventPayload, StoredEvent,
    };

    use super::{InputQueueProjection, replay_input_queue_projection_index};

    #[test]
    fn owner_bridge_exposes_durable_projection_shape() {
        let projection = InputQueueProjection::default();

        assert_eq!(projection.pending_input_count(), 0);
        assert!(projection.pending_delivery_ids.is_empty());
        assert!(projection.active_batch_id.is_none());
    }

    #[test]
    fn replay_index_tracks_pending_inputs_by_agent() {
        let event = StoredEvent {
            storage_seq: 1,
            event: StorageEvent {
                turn_id: Some("turn-1".into()),
                agent: AgentEventContext::default(),
                payload: StorageEventPayload::AgentInputQueued {
                    payload: InputQueuedPayload {
                        envelope: QueuedInputEnvelope {
                            delivery_id: "delivery-1".into(),
                            from_agent_id: "parent".into(),
                            to_agent_id: "child".into(),
                            message: "hello".into(),
                            queued_at: chrono::Utc::now(),
                            sender_lifecycle_status: AgentLifecycleStatus::Running,
                            sender_last_turn_outcome: None,
                            sender_open_session_id: "session-parent".into(),
                        },
                    },
                },
            },
        };

        let index = replay_input_queue_projection_index(&[event]);

        assert_eq!(
            index
                .get("child")
                .expect("child queue should exist")
                .pending_delivery_ids,
            vec!["delivery-1".into()]
        );
    }
}
