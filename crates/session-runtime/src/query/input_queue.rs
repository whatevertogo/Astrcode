//! input queue 相关只读恢复投影。
//!
//! Why: input queue 的 durable 恢复规则属于只读投影，不应该散落在
//! `application` 的父子编排流程里重复实现。

use std::collections::{HashMap, HashSet};

use astrcode_core::{StorageEventPayload, StoredEvent};
use astrcode_kernel::PendingParentDelivery;

use crate::state::replay_input_queue_projection_index;

pub fn recoverable_parent_deliveries(events: &[StoredEvent]) -> Vec<PendingParentDelivery> {
    let projection_index = replay_input_queue_projection_index(events);
    let mut recoverable_by_agent = HashMap::<String, HashSet<String>>::new();
    for (agent_id, projection) in projection_index {
        let active_ids = projection
            .active_delivery_ids
            .into_iter()
            .collect::<HashSet<_>>();
        let recoverable = projection
            .pending_delivery_ids
            .into_iter()
            .filter(|delivery_id| !active_ids.contains(delivery_id))
            .map(|delivery_id| delivery_id.to_string())
            .collect::<HashSet<_>>();
        if !recoverable.is_empty() {
            recoverable_by_agent.insert(agent_id.to_string(), recoverable);
        }
    }

    let mut recovered = Vec::new();
    let mut seen = HashSet::new();
    let queued_at_by_delivery = events
        .iter()
        .filter_map(|stored| match &stored.event.payload {
            StorageEventPayload::AgentInputQueued { payload } => Some((
                payload.envelope.delivery_id.clone(),
                payload.envelope.queued_at,
            )),
            _ => None,
        })
        .collect::<HashMap<_, _>>();
    for stored in events {
        let StorageEventPayload::ChildSessionNotification { notification, .. } =
            &stored.event.payload
        else {
            continue;
        };
        let Some(parent_agent_id) = notification.child_ref.parent_agent_id() else {
            continue;
        };
        let Some(recoverable_ids) = recoverable_by_agent.get(parent_agent_id.as_str()) else {
            continue;
        };
        if !recoverable_ids.contains(notification.notification_id.as_str()) {
            continue;
        }
        if !seen.insert(notification.notification_id.clone()) {
            continue;
        }
        let Some(parent_turn_id) = stored.event.turn_id().map(ToString::to_string) else {
            continue;
        };
        recovered.push(PendingParentDelivery {
            delivery_id: notification.notification_id.to_string(),
            parent_session_id: notification.child_ref.session_id().to_string(),
            parent_turn_id,
            queued_at_ms: queued_at_by_delivery
                .get(&notification.notification_id)
                .map(|queued_at| queued_at.timestamp_millis())
                .unwrap_or_default(),
            notification: notification.clone(),
        });
    }
    recovered
}

#[cfg(test)]
mod tests {
    use astrcode_core::{
        AgentEventContext, AgentLifecycleStatus, AgentTurnOutcome, ChildAgentRef,
        ChildExecutionIdentity, ChildSessionLineageKind, ChildSessionNotification,
        ChildSessionNotificationKind, InputQueuedPayload, ParentExecutionRef, QueuedInputEnvelope,
        StorageEvent, StorageEventPayload, StoredEvent,
    };

    use super::recoverable_parent_deliveries;

    #[test]
    fn recoverable_parent_deliveries_skips_active_batch_entries() {
        let notification = ChildSessionNotification {
            notification_id: "delivery-1".to_string().into(),
            child_ref: ChildAgentRef {
                identity: ChildExecutionIdentity {
                    agent_id: "agent-child".to_string().into(),
                    session_id: "session-parent".to_string().into(),
                    sub_run_id: "subrun-child".to_string().into(),
                },
                parent: ParentExecutionRef {
                    parent_agent_id: Some("agent-parent".to_string().into()),
                    parent_sub_run_id: Some("subrun-parent".to_string().into()),
                },
                lineage_kind: ChildSessionLineageKind::Spawn,
                status: AgentLifecycleStatus::Idle,
                open_session_id: "session-child".to_string().into(),
            },
            kind: ChildSessionNotificationKind::Delivered,
            source_tool_call_id: None,
            delivery: Some(astrcode_core::ParentDelivery {
                idempotency_key: "delivery-1".to_string(),
                origin: astrcode_core::ParentDeliveryOrigin::Explicit,
                terminal_semantics: astrcode_core::ParentDeliveryTerminalSemantics::Terminal,
                source_turn_id: Some("turn-parent".to_string()),
                payload: astrcode_core::ParentDeliveryPayload::Completed(
                    astrcode_core::CompletedParentDeliveryPayload {
                        message: "done".to_string(),
                        findings: Vec::new(),
                        artifacts: Vec::new(),
                    },
                ),
            }),
        };
        let events = vec![
            StoredEvent {
                storage_seq: 1,
                event: StorageEvent {
                    turn_id: Some("turn-parent".to_string()),
                    agent: AgentEventContext::default(),
                    payload: StorageEventPayload::ChildSessionNotification {
                        notification: notification.clone(),
                        timestamp: Some(chrono::Utc::now()),
                    },
                },
            },
            StoredEvent {
                storage_seq: 2,
                event: StorageEvent {
                    turn_id: Some("turn-parent".to_string()),
                    agent: AgentEventContext::default(),
                    payload: StorageEventPayload::AgentInputQueued {
                        payload: InputQueuedPayload {
                            envelope: QueuedInputEnvelope {
                                delivery_id: notification.notification_id.clone(),
                                from_agent_id: notification.child_ref.agent_id().to_string(),
                                to_agent_id: "agent-parent".to_string(),
                                message: "done".to_string(),
                                queued_at: chrono::Utc::now(),
                                sender_lifecycle_status: AgentLifecycleStatus::Idle,
                                sender_last_turn_outcome: Some(AgentTurnOutcome::Completed),
                                sender_open_session_id: notification
                                    .child_ref
                                    .open_session_id
                                    .to_string(),
                            },
                        },
                    },
                },
            },
        ];

        let recovered = recoverable_parent_deliveries(&events);

        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].delivery_id, "delivery-1");
    }
}
