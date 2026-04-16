use std::collections::HashSet;

use super::{
    PendingParentDelivery,
    state::{
        AgentRegistryState, ParentDeliveryQueue, PendingParentDeliveryEntry,
        PendingParentDeliveryState,
    },
};

/// 将指定 delivery_ids 的状态从 WakingParent 重置为 Queued（requeue 批量场景）。
fn mark_parent_deliveries_queued(
    queue: &mut ParentDeliveryQueue,
    delivery_ids: &HashSet<&String>,
) -> usize {
    let mut updated = 0usize;
    for entry in &mut queue.deliveries {
        if delivery_ids.contains(&entry.delivery.delivery_id.to_string()) {
            entry.state = PendingParentDeliveryState::Queued;
            updated += 1;
        }
    }
    updated
}

/// 从队列头部依次消费指定 delivery_ids，要求严格按 FIFO 顺序匹配。
/// 如果队列头部的 delivery_id 与期望不符（顺序错乱），返回 false。
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

/// 入队一条 delivery，带去重（delivery_id）和容量保护。
/// 返回 true 表示入队成功，false 表示重复或队列已满。
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
        log::warn!(
            "parent_delivery_queue 已满 ({}/{}), 丢弃交付 {}",
            queue.deliveries.len(),
            parent_delivery_capacity,
            delivery_id
        );
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

/// 单条 checkout：取队列头部的 Queued delivery，状态转为 WakingParent。
pub(super) fn checkout_parent_delivery_locked(
    state: &mut AgentRegistryState,
    parent_session_id: &str,
) -> Option<PendingParentDelivery> {
    let queue = state.parent_delivery_queues.get_mut(parent_session_id)?;
    let entry = queue.deliveries.front_mut()?;
    if !matches!(entry.state, PendingParentDeliveryState::Queued) {
        return None;
    }
    entry.state = PendingParentDeliveryState::WakingParent;
    Some(entry.delivery.clone())
}

/// 批量 checkout：从队列头部连续取出同一 parent_agent_id 的 Queued delivery。
/// 只包含连续且同父的 delivery，确保一个 wake turn 不会混合不同父 agent 的投递。
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

/// 单条 requeue：将指定 delivery 重置为 Queued 状态（wake 失败时回退）。
pub(super) fn requeue_parent_delivery_locked(
    state: &mut AgentRegistryState,
    parent_session_id: &str,
    delivery_id: &str,
) -> bool {
    let Some(queue) = state.parent_delivery_queues.get_mut(parent_session_id) else {
        return false;
    };
    let Some(entry) = queue
        .deliveries
        .iter_mut()
        .find(|entry| entry.delivery.delivery_id.as_str() == delivery_id)
    else {
        return false;
    };
    entry.state = PendingParentDeliveryState::Queued;
    true
}

/// 批量 requeue：将指定 delivery_ids 的状态重置为 Queued。
/// 返回实际重置的条目数。
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

/// 单条消费：从队列头部移除指定 delivery。队列为空时同步清理 session 的 queue 条目。
pub(super) fn consume_parent_delivery_locked(
    state: &mut AgentRegistryState,
    parent_session_id: &str,
    delivery_id: &str,
) -> bool {
    let should_remove = {
        let Some(queue) = state.parent_delivery_queues.get_mut(parent_session_id) else {
            return false;
        };
        if !consume_front_deliveries(queue, &[delivery_id.to_string()]) {
            return false;
        }
        queue.deliveries.is_empty()
    };

    if should_remove {
        state.parent_delivery_queues.remove(parent_session_id);
    }
    true
}

/// 批量消费：按 FIFO 顺序从队列头部依次移除指定 delivery_ids。队列为空时清理 session 条目。
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

/// 查询指定 session 的待投递 delivery 数量。
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
