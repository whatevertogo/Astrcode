use std::collections::HashMap;

use astrcode_core::{
    AgentState, AgentStateProjector, ChildSessionNode, InputQueueProjection, ModeId, Phase,
    ProjectionRegistrySnapshot, Result, SessionEventRecord, StorageEventPayload, StoredEvent,
    TaskSnapshot, TurnProjectionSnapshot, TurnTerminalKind, event::PhaseTracker,
};
use chrono::{DateTime, Utc};

use super::{
    cache::{RecentSessionEvents, RecentStoredEvents},
    child_sessions::{child_node_from_stored_event, rebuild_child_nodes},
    input_queue::apply_input_queue_event_to_index,
    tasks::{apply_snapshot_to_map, rebuild_active_tasks, task_snapshot_from_stored_event},
};

#[derive(Debug, Clone, Default)]
pub(crate) struct TurnProjection {
    terminal_kind: Option<TurnTerminalKind>,
    last_error: Option<String>,
}

impl TurnProjection {
    fn apply(&mut self, stored: &StoredEvent) {
        match &stored.event.payload {
            StorageEventPayload::TurnDone {
                terminal_kind,
                reason,
                ..
            } => {
                self.terminal_kind = terminal_kind
                    .clone()
                    .or_else(|| TurnTerminalKind::from_legacy_reason(reason.as_deref()));
            },
            StorageEventPayload::Error { message, .. } => {
                let message = message.trim();
                if !message.is_empty() {
                    self.last_error = Some(message.to_string());
                }
            },
            _ => {},
        }
    }

    fn snapshot(&self) -> TurnProjectionSnapshot {
        TurnProjectionSnapshot {
            terminal_kind: self.terminal_kind.clone(),
            last_error: self.last_error.clone(),
        }
    }
}

pub(crate) struct ProjectionRegistry {
    phase_tracker: PhaseTracker,
    agent_projection: AgentStateProjector,
    current_mode_id: ModeId,
    last_mode_changed_at: Option<DateTime<Utc>>,
    child_nodes: HashMap<String, ChildSessionNode>,
    active_tasks: HashMap<String, TaskSnapshot>,
    input_queue_projection_index: HashMap<String, InputQueueProjection>,
    turn_projections: HashMap<String, TurnProjection>,
    recent_records: RecentSessionEvents,
    recent_stored: RecentStoredEvents,
}

impl ProjectionRegistry {
    pub(crate) fn new(
        phase: Phase,
        projector: AgentStateProjector,
        recent_records: Vec<SessionEventRecord>,
        recent_stored: Vec<StoredEvent>,
    ) -> Self {
        let projected = projector.snapshot();
        let snapshot = ProjectionRegistrySnapshot {
            last_mode_changed_at: recent_stored.iter().rev().find_map(|stored| {
                match &stored.event.payload {
                    StorageEventPayload::ModeChanged { timestamp, .. } => Some(*timestamp),
                    _ => None,
                }
            }),
            child_nodes: rebuild_child_nodes(&recent_stored),
            active_tasks: rebuild_active_tasks(&recent_stored),
            input_queue_projection_index: InputQueueProjection::replay_index(&recent_stored),
            turn_projections: rebuild_turn_projections(&recent_stored),
        };
        Self::from_snapshot(
            phase,
            projector,
            recent_records,
            recent_stored,
            snapshot,
            projected.mode_id,
        )
    }

    pub(crate) fn from_recovery(
        phase: Phase,
        checkpoint_agent_state: &AgentState,
        checkpoint_snapshot: ProjectionRegistrySnapshot,
        recent_records: Vec<SessionEventRecord>,
        recent_stored: Vec<StoredEvent>,
    ) -> Self {
        Self::from_snapshot(
            phase,
            AgentStateProjector::from_snapshot(checkpoint_agent_state.clone()),
            recent_records,
            recent_stored,
            checkpoint_snapshot,
            checkpoint_agent_state.mode_id.clone(),
        )
    }

    fn from_snapshot(
        phase: Phase,
        projector: AgentStateProjector,
        recent_records: Vec<SessionEventRecord>,
        recent_stored: Vec<StoredEvent>,
        snapshot: ProjectionRegistrySnapshot,
        current_mode_id: ModeId,
    ) -> Self {
        let mut cached_records = RecentSessionEvents::default();
        cached_records.replace(recent_records);
        let mut cached_stored = RecentStoredEvents::default();
        cached_stored.replace(recent_stored);

        Self {
            phase_tracker: PhaseTracker::new(phase),
            agent_projection: projector,
            current_mode_id,
            last_mode_changed_at: snapshot.last_mode_changed_at,
            child_nodes: snapshot.child_nodes,
            active_tasks: snapshot.active_tasks,
            input_queue_projection_index: snapshot.input_queue_projection_index,
            turn_projections: snapshot
                .turn_projections
                .into_iter()
                .map(|(turn_id, snapshot)| {
                    (
                        turn_id,
                        TurnProjection {
                            terminal_kind: snapshot.terminal_kind,
                            last_error: snapshot.last_error,
                        },
                    )
                })
                .collect(),
            recent_records: cached_records,
            recent_stored: cached_stored,
        }
    }

    pub(crate) fn apply(&mut self, stored: &StoredEvent) -> Result<()> {
        let turn_id = stored.event.turn_id().map(str::to_string);
        let agent = stored.event.agent_context().cloned().unwrap_or_default();
        let _ = self
            .phase_tracker
            .on_event(&stored.event, turn_id.clone(), agent);
        self.agent_projection.apply(&stored.event);

        if let StorageEventPayload::ModeChanged { to, timestamp, .. } = &stored.event.payload {
            self.current_mode_id = to.clone();
            self.last_mode_changed_at = Some(*timestamp);
        }
        if let Some(node) = child_node_from_stored_event(stored) {
            self.child_nodes.insert(node.sub_run_id().to_string(), node);
        }
        if let Some(snapshot) = task_snapshot_from_stored_event(stored) {
            apply_snapshot_to_map(&mut self.active_tasks, snapshot);
        }
        apply_input_queue_event_to_index(&mut self.input_queue_projection_index, stored);
        if let Some(turn_id) = turn_id {
            self.turn_projections
                .entry(turn_id)
                .or_default()
                .apply(stored);
        }
        self.recent_stored.push(stored.clone());
        Ok(())
    }

    pub(crate) fn cache_records(&mut self, records: &[SessionEventRecord]) {
        self.recent_records.push_batch(records);
    }

    pub(crate) fn current_phase(&self) -> Phase {
        self.phase_tracker.current()
    }

    pub(crate) fn snapshot_projected_state(&self) -> AgentState {
        self.agent_projection.snapshot()
    }

    pub(crate) fn current_mode_id(&self) -> ModeId {
        self.current_mode_id.clone()
    }

    pub(crate) fn last_mode_changed_at(&self) -> Option<DateTime<Utc>> {
        self.last_mode_changed_at
    }

    pub(crate) fn projection_snapshot(&self) -> ProjectionRegistrySnapshot {
        ProjectionRegistrySnapshot {
            last_mode_changed_at: self.last_mode_changed_at,
            child_nodes: self.child_nodes.clone(),
            active_tasks: self.active_tasks.clone(),
            input_queue_projection_index: self.input_queue_projection_index.clone(),
            turn_projections: self
                .turn_projections
                .iter()
                .map(|(turn_id, projection)| (turn_id.clone(), projection.snapshot()))
                .collect(),
        }
    }

    pub(crate) fn child_session_node(&self, sub_run_id: &str) -> Option<ChildSessionNode> {
        self.child_nodes.get(sub_run_id).cloned()
    }

    pub(crate) fn upsert_child_session_node(&mut self, node: ChildSessionNode) {
        self.child_nodes.insert(node.sub_run_id().to_string(), node);
    }

    pub(crate) fn list_child_session_nodes(&self) -> Vec<ChildSessionNode> {
        let mut result: Vec<_> = self.child_nodes.values().cloned().collect();
        result.sort_by(|a, b| a.sub_run_id().cmp(b.sub_run_id()));
        result
    }

    pub(crate) fn child_nodes_for_parent(&self, parent_agent_id: &str) -> Vec<ChildSessionNode> {
        let mut result: Vec<_> = self
            .child_nodes
            .values()
            .filter(|node| {
                node.parent_agent_id()
                    .is_some_and(|id| id.as_str() == parent_agent_id)
            })
            .cloned()
            .collect();
        result.sort_by(|a, b| a.sub_run_id().cmp(b.sub_run_id()));
        result
    }

    pub(crate) fn subtree_nodes(&self, root_agent_id: &str) -> Vec<ChildSessionNode> {
        let mut result = Vec::new();
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(root_agent_id.to_string());
        while let Some(agent_id) = queue.pop_front() {
            for node in self.child_nodes.values() {
                if node
                    .parent_agent_id()
                    .is_some_and(|id| id.as_str() == agent_id)
                {
                    queue.push_back(node.agent_id().to_string());
                    result.push(node.clone());
                }
            }
        }
        result.sort_by(|a, b| a.sub_run_id().cmp(b.sub_run_id()));
        result
    }

    #[cfg(test)]
    pub(crate) fn replace_active_task_snapshot(&mut self, snapshot: TaskSnapshot) {
        apply_snapshot_to_map(&mut self.active_tasks, snapshot);
    }

    pub(crate) fn active_tasks_for(&self, owner: &str) -> Option<TaskSnapshot> {
        self.active_tasks.get(owner).cloned()
    }

    pub(crate) fn input_queue_projection_for_agent(&self, agent_id: &str) -> InputQueueProjection {
        self.input_queue_projection_index
            .get(agent_id)
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn turn_projection(&self, turn_id: &str) -> Option<TurnProjectionSnapshot> {
        self.turn_projections
            .get(turn_id)
            .map(TurnProjection::snapshot)
    }

    pub(crate) fn recent_records_after(
        &self,
        last_event_id: Option<&str>,
    ) -> Option<Vec<SessionEventRecord>> {
        self.recent_records.records_after(last_event_id)
    }

    pub(crate) fn snapshot_recent_stored_events(&self) -> Vec<StoredEvent> {
        self.recent_stored.snapshot()
    }
}

fn rebuild_turn_projections(events: &[StoredEvent]) -> HashMap<String, TurnProjectionSnapshot> {
    let mut projections = HashMap::<String, TurnProjection>::new();
    for stored in events {
        let Some(turn_id) = stored.event.turn_id().map(str::to_string) else {
            continue;
        };
        projections.entry(turn_id).or_default().apply(stored);
    }
    projections
        .into_iter()
        .map(|(turn_id, projection)| (turn_id, projection.snapshot()))
        .collect()
}
