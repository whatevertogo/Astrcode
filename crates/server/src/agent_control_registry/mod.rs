mod delivery_queue;
mod state;
mod tree_ops;

use std::{
    collections::{BTreeSet, HashSet, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use astrcode_core::{
    AgentInboxEnvelope, AgentLifecycleStatus, AgentProfile, AgentTurnOutcome, CancelToken,
    ChildSessionLineageKind, DelegationMetadata, ResolvedExecutionLimitsSnapshot,
    SubRunStorageMode,
};
use astrcode_host_session::SubRunHandle;
use delivery_queue::{
    checkout_parent_delivery_batch_locked, consume_parent_delivery_batch_locked,
    enqueue_parent_delivery_locked, pending_parent_delivery_count_locked,
    requeue_parent_delivery_batch_locked,
};
use state::{AgentRegistryEntry, AgentRegistryState, resolve_entry_key};
use thiserror::Error;
use tokio::sync::{RwLock, watch};
use tree_ops::{
    discard_parent_deliveries_locked, prune_finalized_agents_locked, terminate_tree_collect,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingParentDelivery {
    pub(crate) delivery_id: String,
    pub(crate) parent_session_id: String,
    pub(crate) parent_turn_id: String,
    pub(crate) queued_at_ms: i64,
    pub(crate) notification: astrcode_core::ChildSessionNotification,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub(crate) enum AgentControlError {
    #[error("parent agent '{agent_id}' does not exist")]
    ParentAgentNotFound { agent_id: String },
    #[error("agent depth {current} exceeds max depth {max}")]
    MaxDepthExceeded { current: usize, max: usize },
    #[error("active agent count {current} exceeds max concurrent {max}")]
    MaxConcurrentExceeded { current: usize, max: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AgentControlLimits {
    pub(crate) max_depth: usize,
    pub(crate) max_concurrent: usize,
    pub(crate) finalized_retain_limit: usize,
    pub(crate) inbox_capacity: usize,
    pub(crate) parent_delivery_capacity: usize,
}

#[derive(Clone)]
pub(crate) struct AgentControlRegistry {
    next_id: Arc<AtomicU64>,
    next_finalized_seq: Arc<AtomicU64>,
    max_depth: usize,
    max_concurrent: usize,
    finalized_retain_limit: usize,
    inbox_capacity: usize,
    parent_delivery_capacity: usize,
    state: Arc<RwLock<AgentRegistryState>>,
}

impl AgentControlRegistry {
    pub(crate) fn from_limits(limits: AgentControlLimits) -> Self {
        Self {
            next_id: Arc::new(AtomicU64::new(0)),
            next_finalized_seq: Arc::new(AtomicU64::new(0)),
            max_depth: limits.max_depth,
            max_concurrent: limits.max_concurrent,
            finalized_retain_limit: limits.finalized_retain_limit,
            inbox_capacity: limits.inbox_capacity,
            parent_delivery_capacity: limits.parent_delivery_capacity,
            state: Arc::new(RwLock::new(AgentRegistryState::default())),
        }
    }

    pub(crate) async fn get(&self, sub_run_or_agent_id: &str) -> Option<SubRunHandle> {
        let state = self.state.read().await;
        let key = resolve_entry_key(&state, sub_run_or_agent_id)?;
        state.entries.get(key).map(|entry| entry.handle.clone())
    }

    pub(crate) async fn find_root_agent_for_session(
        &self,
        session_id: &str,
    ) -> Option<SubRunHandle> {
        let state = self.state.read().await;
        state
            .entries
            .values()
            .find(|entry| entry.handle.depth == 0 && entry.handle.session_id.as_str() == session_id)
            .map(|entry| entry.handle.clone())
    }

    pub(crate) async fn register_root_agent(
        &self,
        agent_id: String,
        session_id: String,
        profile_id: String,
    ) -> Result<SubRunHandle, AgentControlError> {
        let mut state = self.state.write().await;
        if let Some(existing_key) = state.agent_index.get(&agent_id) {
            if let Some(entry) = state.entries.get(existing_key) {
                return Ok(entry.handle.clone());
            }
        }
        let sub_run_id = format!("root-{agent_id}");
        let handle = SubRunHandle {
            sub_run_id: sub_run_id.clone().into(),
            agent_id: agent_id.clone().into(),
            session_id: session_id.into(),
            child_session_id: None,
            depth: 0,
            parent_turn_id: String::new().into(),
            parent_agent_id: None,
            parent_sub_run_id: None,
            lineage_kind: ChildSessionLineageKind::Spawn,
            agent_profile: profile_id,
            storage_mode: SubRunStorageMode::IndependentSession,
            lifecycle: AgentLifecycleStatus::Running,
            last_turn_outcome: None,
            resolved_limits: ResolvedExecutionLimitsSnapshot,
            delegation: None,
        };
        let cancel = CancelToken::new();
        let (status_tx, _status_rx) = watch::channel(handle.lifecycle);
        state.entries.insert(
            sub_run_id.clone(),
            AgentRegistryEntry {
                handle: handle.clone(),
                cancel,
                status_tx,
                parent_agent_id: None,
                children: BTreeSet::new(),
                finalized_seq: None,
                inbox: VecDeque::new(),
                inbox_version: watch::channel(0).0,
                lifecycle_status: AgentLifecycleStatus::Running,
                last_turn_outcome: None,
            },
        );
        state.agent_index.insert(agent_id, sub_run_id);
        state.active_count += 1;
        Ok(handle)
    }

    pub(crate) async fn spawn_with_storage(
        &self,
        profile: &AgentProfile,
        session_id: String,
        child_session_id: Option<String>,
        parent_turn_id: String,
        parent_agent_id: Option<String>,
        storage_mode: SubRunStorageMode,
    ) -> Result<SubRunHandle, AgentControlError> {
        let mut state = self.state.write().await;
        let depth = match parent_agent_id.as_ref() {
            Some(parent_agent_id) => {
                let Some(parent_sub_run_id) = state.agent_index.get(parent_agent_id) else {
                    return Err(AgentControlError::ParentAgentNotFound {
                        agent_id: parent_agent_id.clone(),
                    });
                };
                let Some(parent) = state.entries.get(parent_sub_run_id) else {
                    return Err(AgentControlError::ParentAgentNotFound {
                        agent_id: parent_agent_id.clone(),
                    });
                };
                parent.handle.depth + 1
            },
            None => 1,
        };
        if depth > self.max_depth {
            return Err(AgentControlError::MaxDepthExceeded {
                current: depth,
                max: self.max_depth,
            });
        }
        if state.active_count >= self.max_concurrent {
            return Err(AgentControlError::MaxConcurrentExceeded {
                current: state.active_count,
                max: self.max_concurrent,
            });
        }

        let next_id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        let agent_id = format!("agent-{next_id}");
        let sub_run_id = format!("subrun-{next_id}");
        let parent_sub_run_id = parent_agent_id
            .as_ref()
            .and_then(|parent_agent_id| state.agent_index.get(parent_agent_id))
            .cloned();
        let handle = SubRunHandle {
            sub_run_id: sub_run_id.clone().into(),
            agent_id: agent_id.clone().into(),
            session_id: session_id.into(),
            child_session_id: child_session_id.map(Into::into),
            depth,
            parent_turn_id: parent_turn_id.into(),
            parent_agent_id: parent_agent_id.clone().map(Into::into),
            parent_sub_run_id: parent_sub_run_id.map(Into::into),
            lineage_kind: ChildSessionLineageKind::Spawn,
            agent_profile: profile.id.clone(),
            storage_mode,
            lifecycle: AgentLifecycleStatus::Pending,
            last_turn_outcome: None,
            resolved_limits: ResolvedExecutionLimitsSnapshot,
            delegation: None,
        };
        let cancel = CancelToken::new();
        let (status_tx, _status_rx) = watch::channel(handle.lifecycle);
        state.entries.insert(
            sub_run_id.clone(),
            AgentRegistryEntry {
                handle: handle.clone(),
                cancel,
                status_tx,
                parent_agent_id: parent_agent_id.clone(),
                children: BTreeSet::new(),
                finalized_seq: None,
                inbox: VecDeque::new(),
                inbox_version: watch::channel(0).0,
                lifecycle_status: AgentLifecycleStatus::Pending,
                last_turn_outcome: None,
            },
        );
        state.agent_index.insert(agent_id, sub_run_id.clone());
        state.active_count += 1;
        if let Some(parent_agent_id) = parent_agent_id {
            if let Some(parent_sub_run_id) = state.agent_index.get(&parent_agent_id).cloned() {
                if let Some(parent) = state.entries.get_mut(&parent_sub_run_id) {
                    parent.children.insert(sub_run_id);
                }
            }
        }
        prune_finalized_agents_locked(&mut state, self.finalized_retain_limit);
        Ok(handle)
    }

    pub(crate) async fn get_lifecycle(&self, id: &str) -> Option<AgentLifecycleStatus> {
        let state = self.state.read().await;
        let key = resolve_entry_key(&state, id)?;
        state.entries.get(key).map(|entry| entry.lifecycle_status)
    }

    pub(crate) async fn get_turn_outcome(&self, id: &str) -> Option<Option<AgentTurnOutcome>> {
        let state = self.state.read().await;
        let key = resolve_entry_key(&state, id)?;
        state.entries.get(key).map(|entry| entry.last_turn_outcome)
    }

    pub(crate) async fn set_lifecycle(
        &self,
        id: &str,
        new_status: AgentLifecycleStatus,
    ) -> Option<()> {
        let mut state = self.state.write().await;
        let key = resolve_entry_key(&state, id)?.to_string();
        let entry = state.entries.get_mut(&key)?;
        entry.lifecycle_status = new_status;
        entry.handle.lifecycle = new_status;
        entry.status_tx.send_replace(new_status);
        Some(())
    }

    pub(crate) async fn complete_turn(
        &self,
        id: &str,
        outcome: AgentTurnOutcome,
    ) -> Option<AgentLifecycleStatus> {
        let next_seq = self.next_finalized_seq.fetch_add(1, Ordering::SeqCst);
        let retain_limit = self.finalized_retain_limit;
        let mut state = self.state.write().await;
        let key = resolve_entry_key(&state, id)?.to_string();
        let was_active = {
            let entry = state.entries.get_mut(&key)?;
            let was_active = entry.handle.lifecycle.occupies_slot();
            entry.last_turn_outcome = Some(outcome);
            entry.lifecycle_status = AgentLifecycleStatus::Idle;
            entry.handle.lifecycle = AgentLifecycleStatus::Idle;
            entry.handle.last_turn_outcome = Some(outcome);
            entry.finalized_seq = Some(next_seq);
            entry.status_tx.send_replace(AgentLifecycleStatus::Idle);
            was_active
        };
        if was_active {
            state.active_count = state.active_count.saturating_sub(1);
        }
        prune_finalized_agents_locked(&mut state, retain_limit);
        Some(AgentLifecycleStatus::Idle)
    }

    pub(crate) async fn set_resolved_limits(
        &self,
        id: &str,
        resolved_limits: ResolvedExecutionLimitsSnapshot,
    ) -> Option<()> {
        let mut state = self.state.write().await;
        let key = resolve_entry_key(&state, id)?.to_string();
        let entry = state.entries.get_mut(&key)?;
        entry.handle.resolved_limits = resolved_limits;
        Some(())
    }

    pub(crate) async fn set_delegation(
        &self,
        id: &str,
        delegation: Option<DelegationMetadata>,
    ) -> Option<()> {
        let mut state = self.state.write().await;
        let key = resolve_entry_key(&state, id)?.to_string();
        let entry = state.entries.get_mut(&key)?;
        entry.handle.delegation = delegation;
        Some(())
    }

    pub(crate) async fn list(&self) -> Vec<SubRunHandle> {
        let state = self.state.read().await;
        let mut handles = state
            .entries
            .values()
            .map(|entry| entry.handle.clone())
            .collect::<Vec<_>>();
        handles.sort_by(|left, right| left.sub_run_id.cmp(&right.sub_run_id));
        handles
    }

    pub(crate) async fn resume(
        &self,
        sub_run_or_agent_id: &str,
        parent_turn_id: impl Into<String>,
    ) -> Option<SubRunHandle> {
        let mut state = self.state.write().await;
        let key = resolve_entry_key(&state, sub_run_or_agent_id)?.to_string();
        if state
            .entries
            .get(&key)
            .is_none_or(|entry| entry.handle.lifecycle.occupies_slot())
        {
            return None;
        }

        let old_entry = state.entries.get(&key)?;
        let old_handle = old_entry.handle.clone();
        let parent_agent_id = old_entry.parent_agent_id.clone();
        let children = old_entry.children.clone();

        let next_id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        let new_sub_run_id = format!("subrun-{next_id}");
        let mut new_handle = old_handle.clone();
        new_handle.sub_run_id = new_sub_run_id.clone().into();
        new_handle.parent_turn_id = parent_turn_id.into().into();
        new_handle.lineage_kind = ChildSessionLineageKind::Resume;
        new_handle.lifecycle = AgentLifecycleStatus::Running;
        new_handle.last_turn_outcome = None;

        let cancel = CancelToken::new();
        let (status_tx, _status_rx) = watch::channel(new_handle.lifecycle);
        let inbox_version = watch::channel(0).0;

        state.active_count += 1;
        state.entries.insert(
            new_sub_run_id.clone(),
            AgentRegistryEntry {
                handle: new_handle.clone(),
                cancel,
                status_tx,
                parent_agent_id: parent_agent_id.clone(),
                children: children.clone(),
                finalized_seq: None,
                inbox: VecDeque::new(),
                inbox_version,
                lifecycle_status: AgentLifecycleStatus::Running,
                last_turn_outcome: None,
            },
        );
        state
            .agent_index
            .insert(new_handle.agent_id.to_string(), new_sub_run_id.clone());

        if let Some(parent_agent_id) = parent_agent_id {
            if let Some(parent_sub_run_id) = state.agent_index.get(&parent_agent_id).cloned() {
                if let Some(parent) = state.entries.get_mut(&parent_sub_run_id) {
                    parent.children.remove(&key);
                    parent.children.insert(new_sub_run_id);
                }
            }
        }
        Some(new_handle)
    }

    pub(crate) async fn push_inbox(
        &self,
        sub_run_or_agent_id: &str,
        envelope: AgentInboxEnvelope,
    ) -> Option<()> {
        let mut state = self.state.write().await;
        let key = resolve_entry_key(&state, sub_run_or_agent_id)?.to_string();
        let entry = state.entries.get_mut(&key)?;
        if entry.inbox.len() >= self.inbox_capacity {
            return None;
        }
        entry.inbox.push_back(envelope);
        let current = *entry.inbox_version.borrow();
        entry.inbox_version.send_replace(current + 1);
        Some(())
    }

    pub(crate) async fn drain_inbox(
        &self,
        sub_run_or_agent_id: &str,
    ) -> Option<Vec<AgentInboxEnvelope>> {
        let mut state = self.state.write().await;
        let key = resolve_entry_key(&state, sub_run_or_agent_id)?.to_string();
        let entry = state.entries.get_mut(&key)?;
        Some(entry.inbox.drain(..).collect())
    }

    pub(crate) async fn enqueue_parent_delivery(
        &self,
        parent_session_id: String,
        parent_turn_id: String,
        notification: astrcode_core::ChildSessionNotification,
    ) -> bool {
        let mut state = self.state.write().await;
        enqueue_parent_delivery_locked(
            &mut state,
            self.parent_delivery_capacity,
            parent_session_id,
            parent_turn_id,
            notification,
        )
    }

    pub(crate) async fn checkout_parent_delivery_batch(
        &self,
        parent_session_id: &str,
    ) -> Option<Vec<PendingParentDelivery>> {
        let mut state = self.state.write().await;
        checkout_parent_delivery_batch_locked(&mut state, parent_session_id)
    }

    pub(crate) async fn pending_parent_delivery_count(&self, parent_session_id: &str) -> usize {
        let state = self.state.read().await;
        pending_parent_delivery_count_locked(&state, parent_session_id)
    }

    pub(crate) async fn requeue_parent_delivery_batch(
        &self,
        parent_session_id: &str,
        delivery_ids: &[String],
    ) {
        let mut state = self.state.write().await;
        requeue_parent_delivery_batch_locked(&mut state, parent_session_id, delivery_ids);
    }

    pub(crate) async fn consume_parent_delivery_batch(
        &self,
        parent_session_id: &str,
        delivery_ids: &[String],
    ) -> bool {
        let mut state = self.state.write().await;
        consume_parent_delivery_batch_locked(&mut state, parent_session_id, delivery_ids)
    }

    pub(crate) async fn terminate_subtree_and_collect_handles(
        &self,
        sub_run_or_agent_id: &str,
    ) -> Option<Vec<SubRunHandle>> {
        let mut state = self.state.write().await;
        let mut visited = HashSet::new();
        let mut terminated = Vec::new();
        let key = resolve_entry_key(&state, sub_run_or_agent_id)?.to_string();
        terminate_tree_collect(
            &mut state,
            &key,
            &mut visited,
            &mut terminated,
            &self.next_finalized_seq,
        )?;
        let terminated_agent_ids = terminated
            .iter()
            .map(|handle| handle.agent_id.clone())
            .collect::<HashSet<_>>();
        discard_parent_deliveries_locked(&mut state, &terminated_agent_ids);
        prune_finalized_agents_locked(&mut state, self.finalized_retain_limit);
        Some(terminated)
    }

    pub(crate) async fn terminate_subtree(
        &self,
        sub_run_or_agent_id: &str,
    ) -> Option<SubRunHandle> {
        self.terminate_subtree_and_collect_handles(sub_run_or_agent_id)
            .await
            .and_then(|mut handles| handles.drain(..).next())
    }

    pub(crate) async fn collect_subtree_handles(
        &self,
        sub_run_or_agent_id: &str,
    ) -> Vec<SubRunHandle> {
        let state = self.state.read().await;
        let mut result = Vec::new();
        let mut queue = std::collections::VecDeque::new();

        if let Some(key) = resolve_entry_key(&state, sub_run_or_agent_id) {
            if let Some(entry) = state.entries.get(key) {
                for child_sub_run_id in &entry.children {
                    queue.push_back(child_sub_run_id.clone());
                }
            }
        }

        while let Some(child_key) = queue.pop_front() {
            if let Some(entry) = state.entries.get(&child_key) {
                result.push(entry.handle.clone());
                for grandchild in &entry.children {
                    queue.push_back(grandchild.clone());
                }
            }
        }
        result
    }
}
