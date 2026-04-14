use std::{
    collections::{HashSet, VecDeque},
    sync::atomic::{AtomicU64, Ordering},
};

use astrcode_core::{AgentLifecycleStatus, AgentTurnOutcome, SubRunHandle};

use super::state::{AgentRegistryState, entry_children};

#[derive(Debug, Clone, Copy)]
struct TerminateTreePolicy {
    clear_inbox: bool,
    bump_inbox_version: bool,
    only_if_active: bool,
}

/// 通用树遍历骨架，visitor 模式统一 cancel / terminate / collect 等操作。
///
/// 深度优先遍历以 `agent_id` 为根的子树，对每个节点调用 `visit` 闭包。
/// `visited` 防止环形引用导致无限递归；已访问节点直接返回缓存的 handle 而不再 visit。
pub(super) fn traverse_tree(
    state: &mut AgentRegistryState,
    agent_id: &str,
    visited: &mut HashSet<String>,
    next_finalized_seq: &AtomicU64,
    visit: &mut impl FnMut(&mut AgentRegistryState, &str, &AtomicU64) -> Option<SubRunHandle>,
) -> Option<SubRunHandle> {
    if !visited.insert(agent_id.to_string()) {
        return state
            .entries
            .get(agent_id)
            .map(|entry| entry.handle.clone());
    }

    let children = entry_children(state, agent_id)?;
    let handle = visit(state, agent_id, next_finalized_seq)?;
    for child_id in children {
        let _ = traverse_tree(state, &child_id, visited, next_finalized_seq, visit);
    }
    Some(handle)
}

/// 将单个 agent entry 标记为 Terminated 并执行所有终态副作用。
///
/// 副作用包括：更新 lifecycle 状态、分配 finalized_seq、触发 cancel token、
/// 递减 active_count（仅当之前占用 slot 时）。
/// `clear_inbox` / `bump_inbox_version` 由调用方按场景决定是否清理 inbox。
pub(super) fn mark_entry_terminated(
    state: &mut AgentRegistryState,
    agent_id: &str,
    next_finalized_seq: &AtomicU64,
    clear_inbox: bool,
    bump_inbox_version: bool,
) -> Option<SubRunHandle> {
    let entry = state.entries.get_mut(agent_id)?;
    let was_active = entry.handle.lifecycle.occupies_slot();

    entry.lifecycle_status = AgentLifecycleStatus::Terminated;
    entry.handle.lifecycle = AgentLifecycleStatus::Terminated;
    entry.handle.last_turn_outcome = Some(AgentTurnOutcome::Cancelled);
    entry.last_turn_outcome = Some(AgentTurnOutcome::Cancelled);
    entry.finalized_seq = Some(next_finalized_seq.fetch_add(1, Ordering::SeqCst));
    // finalized_seq 保证全局递增，用于 GC 按终结顺序回收。
    if clear_inbox {
        entry.inbox.clear();
    }
    if bump_inbox_version {
        let current_inbox_version = *entry.inbox_version.borrow();
        entry.inbox_version.send_replace(current_inbox_version + 1);
    }
    entry
        .status_tx
        .send_replace(AgentLifecycleStatus::Terminated);
    entry.cancel.cancel();

    if was_active {
        state.active_count = state.active_count.saturating_sub(1);
    }

    Some(entry.handle.clone())
}

/// 仅当 agent 当前占用活跃 slot 时才执行 terminate。
/// 返回 `Some(Some(handle))` 表示已终止，`Some(None)` 表示无需操作，`None` 表示 agent 不存在。
pub(super) fn mark_entry_terminated_if_active(
    state: &mut AgentRegistryState,
    agent_id: &str,
    next_finalized_seq: &AtomicU64,
) -> Option<Option<SubRunHandle>> {
    let is_active = state
        .entries
        .get(agent_id)
        .is_some_and(|entry| entry.handle.lifecycle.occupies_slot());
    if !is_active {
        return Some(None);
    }
    mark_entry_terminated(state, agent_id, next_finalized_seq, false, false).map(Some)
}

fn traverse_terminated_tree(
    state: &mut AgentRegistryState,
    agent_id: &str,
    visited: &mut HashSet<String>,
    next_finalized_seq: &AtomicU64,
    policy: TerminateTreePolicy,
    mut on_terminated: impl FnMut(SubRunHandle),
) -> Option<SubRunHandle> {
    traverse_tree(
        state,
        agent_id,
        visited,
        next_finalized_seq,
        &mut |state, agent_id, seq| {
            if policy.only_if_active {
                match mark_entry_terminated_if_active(state, agent_id, seq)? {
                    Some(handle) => {
                        on_terminated(handle.clone());
                        Some(handle)
                    },
                    None => state
                        .entries
                        .get(agent_id)
                        .map(|entry| entry.handle.clone()),
                }
            } else {
                let handle = mark_entry_terminated(
                    state,
                    agent_id,
                    seq,
                    policy.clear_inbox,
                    policy.bump_inbox_version,
                )?;
                on_terminated(handle.clone());
                Some(handle)
            }
        },
    )
}

/// 递归取消子树：不清理 inbox、不 bump inbox version（用于 cancel_for_parent_turn）。
pub(super) fn cancel_tree(
    state: &mut AgentRegistryState,
    agent_id: &str,
    visited: &mut HashSet<String>,
    next_finalized_seq: &AtomicU64,
) -> Option<SubRunHandle> {
    traverse_terminated_tree(
        state,
        agent_id,
        visited,
        next_finalized_seq,
        TerminateTreePolicy {
            clear_inbox: false,
            bump_inbox_version: false,
            only_if_active: false,
        },
        |_| {},
    )
}

/// 递归终止子树并收集所有被终止的 handle（用于 close_child 级联关闭）。
/// 会清理 inbox 并 bump inbox version，因为 close 是最终操作。
pub(super) fn terminate_tree_collect(
    state: &mut AgentRegistryState,
    agent_id: &str,
    visited: &mut HashSet<String>,
    terminated: &mut Vec<SubRunHandle>,
    next_finalized_seq: &AtomicU64,
) -> Option<SubRunHandle> {
    traverse_terminated_tree(
        state,
        agent_id,
        visited,
        next_finalized_seq,
        TerminateTreePolicy {
            clear_inbox: true,
            bump_inbox_version: true,
            only_if_active: false,
        },
        |handle| terminated.push(handle),
    )
}

/// 递归取消子树中仍然活跃的 agent，仅收集实际被终止的 handle。
/// 已处于 Terminated/Idle 的 agent 会被跳过（通过 mark_entry_terminated_if_active）。
pub(super) fn cancel_tree_collect(
    state: &mut AgentRegistryState,
    agent_id: &str,
    visited: &mut HashSet<String>,
    cancelled: &mut Vec<SubRunHandle>,
    next_finalized_seq: &AtomicU64,
) {
    let _ = traverse_terminated_tree(
        state,
        agent_id,
        visited,
        next_finalized_seq,
        TerminateTreePolicy {
            clear_inbox: false,
            bump_inbox_version: false,
            only_if_active: true,
        },
        |handle| cancelled.push(handle),
    );
}

/// 清理已终止 agent 在父级 delivery queue 中残留的待投递条目。
/// 如果 queue 被清空则移除整个 session 的 queue 条目，避免空 map 条目累积。
pub(super) fn discard_parent_deliveries_locked(
    state: &mut AgentRegistryState,
    terminated_agent_ids: &HashSet<String>,
) -> usize {
    if terminated_agent_ids.is_empty() {
        return 0;
    }

    let mut removed_count = 0usize;
    let mut empty_sessions = Vec::new();
    for (session_id, queue) in &mut state.parent_delivery_queues {
        let mut retained = VecDeque::new();
        let mut removed_delivery_ids = Vec::new();

        while let Some(entry) = queue.deliveries.pop_front() {
            if terminated_agent_ids.contains(&entry.delivery.notification.child_ref.agent_id) {
                removed_delivery_ids.push(entry.delivery.delivery_id.clone());
            } else {
                retained.push_back(entry);
            }
        }

        for delivery_id in &removed_delivery_ids {
            queue.known_delivery_ids.remove(delivery_id);
        }
        removed_count += removed_delivery_ids.len();
        queue.deliveries = retained;

        if queue.deliveries.is_empty() {
            empty_sessions.push(session_id.clone());
        }
    }

    for session_id in empty_sessions {
        state.parent_delivery_queues.remove(&session_id);
    }

    removed_count
}

/// GC：回收已终结（finalized）且无子节点的叶子 agent。
///
/// 每轮迭代找到 finalized_seq 最小的叶子 agent（最早终结的优先回收），
/// 从父节点的 children 集合中断开，然后从 registry 中移除。
/// 循环直到叶子终结数量 ≤ `finalized_retain_limit`。
/// `usize::MAX` 表示不执行任何回收。
pub(super) fn prune_finalized_agents_locked(
    state: &mut AgentRegistryState,
    finalized_retain_limit: usize,
) {
    if finalized_retain_limit == usize::MAX {
        return;
    }

    loop {
        let mut finalized_leaf_agents = state
            .entries
            .iter()
            .filter_map(|(agent_id, entry)| {
                entry
                    .finalized_seq
                    .filter(|_| entry.children.is_empty())
                    .map(|seq| (seq, agent_id.clone(), entry.parent_agent_id.clone()))
            })
            .collect::<Vec<_>>();
        if finalized_leaf_agents.len() <= finalized_retain_limit {
            break;
        }

        finalized_leaf_agents.sort_by_key(|(seq, agent_id, _)| (*seq, agent_id.clone()));
        // 取 finalized_seq 最小的叶子优先回收——最早终结的 agent 最不可能还需要回查。
        let Some((_, agent_id, parent_agent_id)) = finalized_leaf_agents.into_iter().next() else {
            break;
        };

        if let Some(parent_agent_id) = parent_agent_id {
            if let Some(parent_sub_run_id) = state.agent_index.get(&parent_agent_id).cloned() {
                if let Some(parent) = state.entries.get_mut(&parent_sub_run_id) {
                    parent.children.remove(&agent_id);
                }
            }
        }
        if let Some(entry) = state.entries.remove(&agent_id) {
            state.agent_index.remove(&entry.handle.agent_id);
        }
    }
}
