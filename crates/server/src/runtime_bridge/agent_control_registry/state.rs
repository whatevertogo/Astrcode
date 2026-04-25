use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use astrcode_core::{AgentInboxEnvelope, AgentLifecycleStatus, AgentTurnOutcome, CancelToken};
use astrcode_host_session::SubRunHandle;
use tokio::sync::watch;

use super::PendingParentDelivery;

#[derive(Default)]
pub(super) struct AgentRegistryState {
    pub(super) entries: HashMap<String, AgentRegistryEntry>,
    pub(super) agent_index: HashMap<String, String>,
    pub(super) active_count: usize,
    pub(super) parent_delivery_queues: HashMap<String, ParentDeliveryQueue>,
}

pub(super) struct AgentRegistryEntry {
    pub(super) handle: SubRunHandle,
    pub(super) cancel: CancelToken,
    pub(super) status_tx: watch::Sender<AgentLifecycleStatus>,
    pub(super) parent_agent_id: Option<String>,
    pub(super) children: BTreeSet<String>,
    pub(super) finalized_seq: Option<u64>,
    pub(super) inbox: VecDeque<AgentInboxEnvelope>,
    pub(super) inbox_version: watch::Sender<u64>,
    pub(super) lifecycle_status: AgentLifecycleStatus,
    pub(super) last_turn_outcome: Option<AgentTurnOutcome>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PendingParentDeliveryState {
    Queued,
    WakingParent,
}

#[derive(Debug, Clone)]
pub(super) struct PendingParentDeliveryEntry {
    pub(super) delivery: PendingParentDelivery,
    pub(super) state: PendingParentDeliveryState,
}

#[derive(Default)]
pub(super) struct ParentDeliveryQueue {
    pub(super) deliveries: VecDeque<PendingParentDeliveryEntry>,
    pub(super) known_delivery_ids: HashSet<String>,
}

pub(super) fn resolve_entry_key<'a>(
    state: &'a AgentRegistryState,
    sub_run_or_agent_id: &'a str,
) -> Option<&'a str> {
    if state.entries.contains_key(sub_run_or_agent_id) {
        return Some(sub_run_or_agent_id);
    }
    state
        .agent_index
        .get(sub_run_or_agent_id)
        .map(String::as_str)
}

pub(super) fn entry_children(state: &AgentRegistryState, sub_run_id: &str) -> Option<Vec<String>> {
    state
        .entries
        .get(sub_run_id)
        .map(|entry| entry.children.iter().cloned().collect())
}
