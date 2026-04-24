use std::collections::{HashMap, VecDeque};

use astrcode_core::{
    ChildSessionNode, Phase, Result, SessionEventRecord, StorageEventPayload, StoredEvent,
    TaskSnapshot, event::PhaseTracker, mode::ModeId,
};
use chrono::{DateTime, Utc};

use crate::{
    AgentState, AgentStateProjector, InputQueueProjection, ProjectionRegistrySnapshot,
    TurnProjectionSnapshot,
    child_sessions::{child_node_from_stored_event, rebuild_child_nodes},
    event_cache::{RecentSessionEvents, RecentStoredEvents},
    input_queue::{apply_input_queue_event_to_index, replay_input_queue_projection_index},
    tasks::{apply_snapshot_to_map, rebuild_active_tasks, task_snapshot_from_stored_event},
    turn_projection::{apply_turn_projection_event, project_turn_projection},
};

#[derive(Debug, Clone, Default)]
struct TurnProjection {
    snapshot: TurnProjectionSnapshot,
}

impl TurnProjection {
    fn apply(&mut self, stored: &StoredEvent) {
        apply_turn_projection_event(&mut self.snapshot, stored);
    }

    fn snapshot(&self) -> TurnProjectionSnapshot {
        self.snapshot.clone()
    }
}

#[derive(Debug, Clone)]
struct ModeProjectionState {
    current_mode_id: ModeId,
    last_mode_changed_at: Option<DateTime<Utc>>,
}

impl ModeProjectionState {
    fn new(current_mode_id: ModeId, last_mode_changed_at: Option<DateTime<Utc>>) -> Self {
        Self {
            current_mode_id,
            last_mode_changed_at,
        }
    }

    fn apply(&mut self, stored: &StoredEvent) {
        if let StorageEventPayload::ModeChanged { to, timestamp, .. } = &stored.event.payload {
            self.current_mode_id = to.clone();
            self.last_mode_changed_at = Some(*timestamp);
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ChildNodeProjection {
    nodes: HashMap<String, ChildSessionNode>,
}

impl ChildNodeProjection {
    fn rebuild(events: &[StoredEvent]) -> Self {
        Self {
            nodes: rebuild_child_nodes(events),
        }
    }

    fn from_snapshot(nodes: HashMap<String, ChildSessionNode>) -> Self {
        Self { nodes }
    }

    fn apply(&mut self, stored: &StoredEvent) {
        if let Some(node) = child_node_from_stored_event(stored) {
            self.nodes.insert(node.sub_run_id().to_string(), node);
        }
    }

    pub(crate) fn upsert(&mut self, node: ChildSessionNode) {
        self.nodes.insert(node.sub_run_id().to_string(), node);
    }

    fn get(&self, sub_run_id: &str) -> Option<ChildSessionNode> {
        self.nodes.get(sub_run_id).cloned()
    }

    fn list(&self) -> Vec<ChildSessionNode> {
        let mut result: Vec<_> = self.nodes.values().cloned().collect();
        result.sort_by(|a, b| a.sub_run_id().cmp(b.sub_run_id()));
        result
    }

    fn for_parent(&self, parent_agent_id: &str) -> Vec<ChildSessionNode> {
        let mut result: Vec<_> = self
            .nodes
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

    fn subtree(&self, root_agent_id: &str) -> Vec<ChildSessionNode> {
        let mut result = Vec::new();
        let mut queue = VecDeque::new();
        queue.push_back(root_agent_id.to_string());
        while let Some(agent_id) = queue.pop_front() {
            for node in self.nodes.values() {
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
}

#[derive(Debug, Clone, Default)]
struct ActiveTaskProjection {
    snapshots: HashMap<String, TaskSnapshot>,
}

impl ActiveTaskProjection {
    fn rebuild(events: &[StoredEvent]) -> Self {
        Self {
            snapshots: rebuild_active_tasks(events),
        }
    }

    fn from_snapshot(snapshots: HashMap<String, TaskSnapshot>) -> Self {
        Self { snapshots }
    }

    fn apply(&mut self, stored: &StoredEvent) {
        if let Some(snapshot) = task_snapshot_from_stored_event(stored) {
            apply_snapshot_to_map(&mut self.snapshots, snapshot);
        }
    }

    fn get(&self, owner: &str) -> Option<TaskSnapshot> {
        self.snapshots.get(owner).cloned()
    }
}

#[derive(Debug, Clone, Default)]
struct InputQueueProjectionIndex {
    by_agent: HashMap<String, InputQueueProjection>,
}

impl InputQueueProjectionIndex {
    fn rebuild(events: &[StoredEvent]) -> Self {
        Self {
            by_agent: replay_input_queue_projection_index(events),
        }
    }

    fn from_snapshot(by_agent: HashMap<String, InputQueueProjection>) -> Self {
        Self { by_agent }
    }

    fn apply(&mut self, stored: &StoredEvent) {
        apply_input_queue_event_to_index(&mut self.by_agent, stored);
    }

    fn get(&self, agent_id: &str) -> InputQueueProjection {
        self.by_agent.get(agent_id).cloned().unwrap_or_default()
    }
}

#[derive(Debug, Clone, Default)]
struct TurnProjectionIndex {
    by_turn: HashMap<String, TurnProjection>,
}

impl TurnProjectionIndex {
    fn rebuild(events: &[StoredEvent]) -> Self {
        let mut events_by_turn = HashMap::<String, Vec<StoredEvent>>::new();
        for stored in events {
            let Some(turn_id) = stored.event.turn_id().map(str::to_string) else {
                continue;
            };
            events_by_turn
                .entry(turn_id)
                .or_default()
                .push(stored.clone());
        }

        let by_turn = events_by_turn
            .into_iter()
            .filter_map(|(turn_id, turn_events)| {
                project_turn_projection(&turn_events)
                    .map(|snapshot| (turn_id, TurnProjection { snapshot }))
            })
            .collect();

        Self { by_turn }
    }

    fn from_snapshot(snapshot: HashMap<String, TurnProjectionSnapshot>) -> Self {
        Self {
            by_turn: snapshot
                .into_iter()
                .map(|(turn_id, snapshot)| (turn_id, TurnProjection { snapshot }))
                .collect(),
        }
    }

    fn apply(&mut self, stored: &StoredEvent) {
        let Some(turn_id) = stored.event.turn_id().map(str::to_string) else {
            return;
        };
        self.by_turn.entry(turn_id).or_default().apply(stored);
    }

    fn snapshot(&self) -> HashMap<String, TurnProjectionSnapshot> {
        self.by_turn
            .iter()
            .map(|(turn_id, projection)| (turn_id.clone(), projection.snapshot()))
            .collect()
    }

    fn get(&self, turn_id: &str) -> Option<TurnProjectionSnapshot> {
        self.by_turn.get(turn_id).map(TurnProjection::snapshot)
    }
}

#[derive(Default)]
struct RecentProjectionCache {
    records: RecentSessionEvents,
    stored: RecentStoredEvents,
}

impl RecentProjectionCache {
    fn new(records: Vec<SessionEventRecord>, stored: Vec<StoredEvent>) -> Self {
        let mut cache = Self::default();
        cache.records.replace(records);
        cache.stored.replace(stored);
        cache
    }

    fn push_stored(&mut self, stored: StoredEvent) {
        self.stored.push(stored);
    }

    fn push_records(&mut self, records: &[SessionEventRecord]) {
        self.records.push_batch(records);
    }
}

pub(crate) struct ProjectionRegistry {
    phase_tracker: PhaseTracker,
    agent_projection: AgentStateProjector,
    mode: ModeProjectionState,
    pub(crate) children: ChildNodeProjection,
    tasks: ActiveTaskProjection,
    input_queue: InputQueueProjectionIndex,
    turns: TurnProjectionIndex,
    cache: RecentProjectionCache,
}

impl ProjectionRegistry {
    pub(crate) fn new(
        phase: Phase,
        projector: AgentStateProjector,
        recent_records: Vec<SessionEventRecord>,
        recent_stored: Vec<StoredEvent>,
    ) -> Self {
        let projected = projector.snapshot();
        Self::from_snapshot(
            phase,
            projector,
            recent_records,
            recent_stored.clone(),
            ProjectionRegistrySnapshot {
                last_mode_changed_at: recent_stored.iter().rev().find_map(|stored| {
                    match &stored.event.payload {
                        StorageEventPayload::ModeChanged { timestamp, .. } => Some(*timestamp),
                        _ => None,
                    }
                }),
                child_nodes: ChildNodeProjection::rebuild(&recent_stored).nodes,
                active_tasks: ActiveTaskProjection::rebuild(&recent_stored).snapshots,
                input_queue_projection_index: InputQueueProjectionIndex::rebuild(&recent_stored)
                    .by_agent,
                turn_projections: TurnProjectionIndex::rebuild(&recent_stored).snapshot(),
            },
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
        Self {
            phase_tracker: PhaseTracker::new(phase),
            agent_projection: projector,
            mode: ModeProjectionState::new(current_mode_id, snapshot.last_mode_changed_at),
            children: ChildNodeProjection::from_snapshot(snapshot.child_nodes),
            tasks: ActiveTaskProjection::from_snapshot(snapshot.active_tasks),
            input_queue: InputQueueProjectionIndex::from_snapshot(
                snapshot.input_queue_projection_index,
            ),
            turns: TurnProjectionIndex::from_snapshot(snapshot.turn_projections),
            cache: RecentProjectionCache::new(recent_records, recent_stored),
        }
    }

    pub(crate) fn apply(&mut self, stored: &StoredEvent) -> Result<()> {
        let turn_id = stored.event.turn_id().map(str::to_string);
        let agent = stored.event.agent_context().cloned().unwrap_or_default();
        let _ = self.phase_tracker.on_event(&stored.event, turn_id, agent);
        self.agent_projection.apply(&stored.event);
        self.mode.apply(stored);
        self.children.apply(stored);
        self.tasks.apply(stored);
        self.input_queue.apply(stored);
        self.turns.apply(stored);
        self.cache.push_stored(stored.clone());
        Ok(())
    }

    pub(crate) fn cache_records(&mut self, records: &[SessionEventRecord]) {
        self.cache.push_records(records);
    }

    pub(crate) fn current_phase(&self) -> Phase {
        self.phase_tracker.current()
    }

    pub(crate) fn snapshot_projected_state(&self) -> AgentState {
        self.agent_projection.snapshot()
    }

    pub(crate) fn current_mode_id(&self) -> ModeId {
        self.mode.current_mode_id.clone()
    }

    pub(crate) fn last_mode_changed_at(&self) -> Option<DateTime<Utc>> {
        self.mode.last_mode_changed_at
    }

    pub(crate) fn projection_snapshot(&self) -> ProjectionRegistrySnapshot {
        ProjectionRegistrySnapshot {
            last_mode_changed_at: self.mode.last_mode_changed_at,
            child_nodes: self.children.nodes.clone(),
            active_tasks: self.tasks.snapshots.clone(),
            input_queue_projection_index: self.input_queue.by_agent.clone(),
            turn_projections: self.turns.snapshot(),
        }
    }

    pub(crate) fn child_session_node(&self, sub_run_id: &str) -> Option<ChildSessionNode> {
        self.children.get(sub_run_id)
    }

    pub(crate) fn list_child_session_nodes(&self) -> Vec<ChildSessionNode> {
        self.children.list()
    }

    pub(crate) fn child_nodes_for_parent(&self, parent_agent_id: &str) -> Vec<ChildSessionNode> {
        self.children.for_parent(parent_agent_id)
    }

    pub(crate) fn subtree_nodes(&self, root_agent_id: &str) -> Vec<ChildSessionNode> {
        self.children.subtree(root_agent_id)
    }

    pub(crate) fn active_tasks_for(&self, owner: &str) -> Option<TaskSnapshot> {
        self.tasks.get(owner)
    }

    pub(crate) fn input_queue_projection_for_agent(&self, agent_id: &str) -> InputQueueProjection {
        self.input_queue.get(agent_id)
    }

    pub(crate) fn turn_projection(&self, turn_id: &str) -> Option<TurnProjectionSnapshot> {
        self.turns.get(turn_id)
    }

    pub(crate) fn recent_records_after(
        &self,
        last_event_id: Option<&str>,
    ) -> Option<Vec<SessionEventRecord>> {
        self.cache.records.records_after(last_event_id)
    }

    pub(crate) fn snapshot_recent_stored_events(&self) -> Vec<StoredEvent> {
        self.cache.stored.snapshot()
    }
}
