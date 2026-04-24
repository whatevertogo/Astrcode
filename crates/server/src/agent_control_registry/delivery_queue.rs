use std::collections::HashSet;

use super::{
    PendingParentDelivery,
    state::{
        AgentRegistryState, ParentDeliveryQueue, PendingParentDeliveryEntry,
        PendingParentDeliveryState,
    },
};

fn mark_parent_deliveries_queued(
    queue: &mut ParentDeliveryQueue,
    delivery_ids: &HashSet<&String>,
) -> usize {
    let mut updated = 0usize;
    for entry in &mut queue.deliveries {
        if delivery_ids.contains(&entry.delivery.delivery_id) {
            entry.state = PendingParentDeliveryState::Queued;
            updated += 1;
        }
    }
    updated
}

fn consume_front_deliveries(queue: &mut ParentDeliveryQueue, delivery_ids: &[String]) -> bool {
    for delivery_id in delivery_ids {
        let Some(front) = queue.deliveries.front() else {
            return false;
        };
        if front.delivery.delivery_id.as_str() != delivery_id {
            return false;
        }
        if let Some(removed) = queue.deliveries.pop_front() {
            queue
                .known_delivery_ids
                .remove(removed.delivery.delivery_id.as_str());
        }
    }
    true
}

pub(super) fn enqueue_parent_delivery_locked(
    state: &mut AgentRegistryState,
    parent_delivery_capacity: usize,
    parent_session_id: String,
    parent_turn_id: String,
    notification: astrcode_core::ChildSessionNotification,
) -> bool {
    let delivery_id = notification.notification_id.clone();
    let queue = state
        .parent_delivery_queues
        .entry(parent_session_id.clone())
        .or_default();
    if !queue.known_delivery_ids.insert(delivery_id.to_string()) {
        return false;
    }
    if queue.deliveries.len() >= parent_delivery_capacity {
        queue.known_delivery_ids.remove(delivery_id.as_str());
        return false;
    }
    queue.deliveries.push_back(PendingParentDeliveryEntry {
        delivery: PendingParentDelivery {
            delivery_id: delivery_id.into(),
            parent_session_id,
            parent_turn_id,
            queued_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_millis() as i64)
                .unwrap_or_default(),
            notification,
        },
        state: PendingParentDeliveryState::Queued,
    });
    true
}

pub(super) fn checkout_parent_delivery_batch_locked(
    state: &mut AgentRegistryState,
    parent_session_id: &str,
) -> Option<Vec<PendingParentDelivery>> {
    let queue = state.parent_delivery_queues.get_mut(parent_session_id)?;
    let first = queue.deliveries.front()?;
    if !matches!(first.state, PendingParentDeliveryState::Queued) {
        return None;
    }

    let target_parent_agent_id = first
        .delivery
        .notification
        .child_ref
        .parent_agent_id()
        .cloned();
    let mut batch_len = 0usize;
    for entry in &queue.deliveries {
        if !matches!(entry.state, PendingParentDeliveryState::Queued) {
            break;
        }
        if entry
            .delivery
            .notification
            .child_ref
            .parent_agent_id()
            .cloned()
            != target_parent_agent_id
        {
            break;
        }
        batch_len += 1;
    }

    if batch_len == 0 {
        return None;
    }

    let mut deliveries = Vec::with_capacity(batch_len);
    for entry in queue.deliveries.iter_mut().take(batch_len) {
        entry.state = PendingParentDeliveryState::WakingParent;
        deliveries.push(entry.delivery.clone());
    }
    Some(deliveries)
}

pub(super) fn requeue_parent_delivery_batch_locked(
    state: &mut AgentRegistryState,
    parent_session_id: &str,
    delivery_ids: &[String],
) -> usize {
    let Some(queue) = state.parent_delivery_queues.get_mut(parent_session_id) else {
        return 0;
    };
    let target_ids = delivery_ids.iter().collect::<HashSet<_>>();
    mark_parent_deliveries_queued(queue, &target_ids)
}

pub(super) fn consume_parent_delivery_batch_locked(
    state: &mut AgentRegistryState,
    parent_session_id: &str,
    delivery_ids: &[String],
) -> bool {
    let should_remove = {
        let Some(queue) = state.parent_delivery_queues.get_mut(parent_session_id) else {
            return false;
        };
        if !consume_front_deliveries(queue, delivery_ids) {
            return false;
        }
        queue.deliveries.is_empty()
    };

    if should_remove {
        state.parent_delivery_queues.remove(parent_session_id);
    }
    true
}

pub(super) fn pending_parent_delivery_count_locked(
    state: &AgentRegistryState,
    parent_session_id: &str,
) -> usize {
    state
        .parent_delivery_queues
        .get(parent_session_id)
        .map(|queue| queue.deliveries.len())
        .unwrap_or(0)
}
