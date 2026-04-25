use std::{
    collections::{HashSet, VecDeque},
    sync::atomic::{AtomicU64, Ordering},
};

use astrcode_core::{AgentId, AgentLifecycleStatus, AgentTurnOutcome};
use astrcode_host_session::SubRunHandle;

use super::state::{AgentRegistryState, entry_children};

#[derive(Debug, Clone, Copy)]
struct TerminateTreePolicy {
    clear_inbox: bool,
    bump_inbox_version: bool,
    only_if_active: bool,
}

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
    if clear_inbox {
        entry.inbox.clear();
    }
    if bump_inbox_version {
        let current = *entry.inbox_version.borrow();
        entry.inbox_version.send_replace(current + 1);
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

pub(super) fn discard_parent_deliveries_locked(
    state: &mut AgentRegistryState,
    terminated_agent_ids: &HashSet<AgentId>,
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
            if terminated_agent_ids.contains(entry.delivery.notification.child_ref.agent_id()) {
                removed_delivery_ids.push(entry.delivery.delivery_id.clone());
            } else {
                retained.push_back(entry);
            }
        }

        for delivery_id in &removed_delivery_ids {
            queue.known_delivery_ids.remove(delivery_id.as_str());
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
            state.agent_index.remove(entry.handle.agent_id.as_str());
        }
    }
}
