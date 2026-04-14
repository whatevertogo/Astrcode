use std::{
    cmp::Reverse,
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};

use astrcode_application::{App, AppGovernance, ApplicationError};
use astrcode_core::{
    AgentLifecycleStatus, ChildSessionNode, Phase, StorageEventPayload, StoredEvent,
};
use chrono::{DateTime, Utc};

use crate::models::{
    DebugAgentNodeKind, RuntimeDebugOverview, RuntimeDebugTimeline, RuntimeDebugTimelineSample,
    SessionDebugAgentNode, SessionDebugAgents, SessionDebugTrace, SessionDebugTraceItem,
    SessionDebugTraceItemKind,
};

const DEFAULT_TIMELINE_WINDOW: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Default)]
struct TimelineStore {
    samples: VecDeque<RuntimeDebugTimelineSample>,
}

impl TimelineStore {
    fn record(
        &mut self,
        collected_at: DateTime<Utc>,
        spawn_rejection_ratio_bps: Option<u64>,
        observe_to_action_ratio_bps: Option<u64>,
        child_reuse_ratio_bps: Option<u64>,
        window: Duration,
    ) {
        self.samples.push_back(RuntimeDebugTimelineSample {
            collected_at,
            spawn_rejection_ratio_bps,
            observe_to_action_ratio_bps,
            child_reuse_ratio_bps,
        });
        self.trim(collected_at, window);
    }

    fn snapshot(
        &mut self,
        now: DateTime<Utc>,
        window: Duration,
    ) -> Vec<RuntimeDebugTimelineSample> {
        self.trim(now, window);
        self.samples.iter().cloned().collect()
    }

    fn trim(&mut self, now: DateTime<Utc>, window: Duration) {
        let Ok(window_delta) = chrono::Duration::from_std(window) else {
            return;
        };
        let cutoff = now - window_delta;
        while self
            .samples
            .front()
            .is_some_and(|sample| sample.collected_at < cutoff)
        {
            self.samples.pop_front();
        }
    }
}

pub struct DebugWorkbenchService {
    app: Arc<App>,
    governance: Arc<AppGovernance>,
    timeline: Mutex<TimelineStore>,
    timeline_window: Duration,
}

impl DebugWorkbenchService {
    pub fn new(app: Arc<App>, governance: Arc<AppGovernance>) -> Self {
        Self {
            app,
            governance,
            timeline: Mutex::new(TimelineStore::default()),
            timeline_window: DEFAULT_TIMELINE_WINDOW,
        }
    }

    pub fn runtime_overview(&self) -> RuntimeDebugOverview {
        let collected_at = Utc::now();
        let metrics = self.governance.observability_snapshot();
        let spawn_rejection_ratio_bps = derive_spawn_rejection_ratio_bps(&metrics);
        self.timeline
            .lock()
            .expect("debug workbench timeline mutex")
            .record(
                collected_at,
                spawn_rejection_ratio_bps,
                metrics.agent_collaboration.observe_to_action_ratio_bps,
                metrics.agent_collaboration.child_reuse_ratio_bps,
                self.timeline_window,
            );
        RuntimeDebugOverview {
            collected_at,
            metrics,
            spawn_rejection_ratio_bps,
        }
    }

    pub fn runtime_timeline(&self) -> RuntimeDebugTimeline {
        let overview = self.runtime_overview();
        let mut timeline = self
            .timeline
            .lock()
            .expect("debug workbench timeline mutex");
        let samples = timeline.snapshot(overview.collected_at, self.timeline_window);
        let window_started_at = samples
            .first()
            .map(|sample| sample.collected_at)
            .unwrap_or(overview.collected_at);
        RuntimeDebugTimeline {
            window_started_at,
            window_ended_at: overview.collected_at,
            samples,
        }
    }

    pub async fn session_trace(
        &self,
        session_id: &str,
    ) -> Result<SessionDebugTrace, ApplicationError> {
        let meta = find_session_meta(&self.app, session_id).await?;
        let stored_events = self.app.session_stored_events(session_id).await?;
        let phase = self
            .app
            .session_view(session_id)
            .await
            .map(|view| view.phase)
            .unwrap_or(meta.phase);

        let mut items = stored_events
            .iter()
            .filter_map(build_trace_item)
            .collect::<Vec<_>>();
        items.sort_by_key(|item| Reverse(item.storage_seq));

        Ok(SessionDebugTrace {
            session_id: meta.session_id,
            title: meta.title,
            phase,
            parent_session_id: meta.parent_session_id,
            items,
        })
    }

    pub async fn session_agents(
        &self,
        session_id: &str,
    ) -> Result<SessionDebugAgents, ApplicationError> {
        let meta = find_session_meta(&self.app, session_id).await?;
        let root_status = self.app.get_root_agent_status(session_id).await?;
        let child_nodes = self.app.session_child_nodes(session_id).await?;
        let child_depths = compute_child_depths(&child_nodes);
        let mut nodes = Vec::new();

        let (root_agent_id, root_lifecycle, root_outcome) = root_status
            .map(|status| (status.agent_id, status.lifecycle, status.last_turn_outcome))
            .unwrap_or_else(|| {
                (
                    format!("session-root:{}", meta.session_id),
                    phase_to_lifecycle(meta.phase),
                    None,
                )
            });

        nodes.push(SessionDebugAgentNode {
            node_id: format!("root:{}", meta.session_id),
            kind: DebugAgentNodeKind::SessionRoot,
            title: meta.title.clone(),
            agent_id: root_agent_id,
            session_id: meta.session_id.clone(),
            child_session_id: Some(meta.session_id.clone()),
            sub_run_id: None,
            parent_agent_id: None,
            parent_session_id: meta.parent_session_id.clone(),
            depth: 0,
            lifecycle: root_lifecycle,
            last_turn_outcome: root_outcome,
            status_source: None,
            lineage_kind: None,
        });

        for node in child_nodes {
            let depth = child_depths.get(&node.agent_id).copied().unwrap_or(1);
            nodes.push(map_child_session_node(node, depth));
        }

        nodes.sort_by(|left, right| {
            left.depth
                .cmp(&right.depth)
                .then_with(|| left.title.cmp(&right.title))
        });

        Ok(SessionDebugAgents {
            session_id: meta.session_id,
            title: meta.title,
            nodes,
        })
    }
}

async fn find_session_meta(
    app: &Arc<App>,
    session_id: &str,
) -> Result<astrcode_core::SessionMeta, ApplicationError> {
    let target_session_id = session_id.trim();
    app.list_sessions()
        .await?
        .into_iter()
        .find(|meta| meta.session_id == target_session_id)
        .ok_or_else(|| {
            ApplicationError::NotFound(format!("session '{}' not found", target_session_id))
        })
}

fn compute_child_depths(nodes: &[ChildSessionNode]) -> HashMap<String, usize> {
    let subrun_to_agent = nodes
        .iter()
        .map(|node| (node.sub_run_id.clone(), node.agent_id.clone()))
        .collect::<HashMap<_, _>>();
    let parent_agent_ids = nodes
        .iter()
        .filter_map(|node| {
            let parent_sub_run_id = node.parent_sub_run_id.as_ref()?;
            let parent_agent_id = subrun_to_agent.get(parent_sub_run_id)?;
            Some((node.agent_id.clone(), parent_agent_id.clone()))
        })
        .collect::<HashMap<_, _>>();
    let mut depths = HashMap::new();
    for node in nodes {
        let mut depth = 1usize;
        let mut cursor = node.agent_id.clone();
        while let Some(parent_agent_id) = parent_agent_ids.get(&cursor) {
            depth += 1;
            cursor = parent_agent_id.clone();
        }
        depths.insert(node.agent_id.clone(), depth);
    }
    depths
}

fn map_child_session_node(node: ChildSessionNode, depth: usize) -> SessionDebugAgentNode {
    SessionDebugAgentNode {
        node_id: format!("child:{}", node.sub_run_id),
        kind: DebugAgentNodeKind::ChildAgent,
        title: format!("{} · {}", node.sub_run_id, node.child_session_id),
        agent_id: node.agent_id,
        session_id: node.session_id,
        child_session_id: Some(node.child_session_id),
        sub_run_id: Some(node.sub_run_id),
        parent_agent_id: node.parent_agent_id,
        parent_session_id: Some(node.parent_session_id),
        depth,
        lifecycle: node.status,
        last_turn_outcome: None,
        status_source: Some(node.status_source),
        lineage_kind: Some(node.lineage_kind),
    }
}

fn phase_to_lifecycle(phase: Phase) -> AgentLifecycleStatus {
    match phase {
        Phase::Idle | Phase::Done => AgentLifecycleStatus::Idle,
        Phase::Interrupted => AgentLifecycleStatus::Terminated,
        Phase::Thinking | Phase::CallingTool | Phase::Streaming => AgentLifecycleStatus::Running,
    }
}

fn derive_spawn_rejection_ratio_bps(
    metrics: &astrcode_application::RuntimeObservabilitySnapshot,
) -> Option<u64> {
    let denominator =
        metrics.agent_collaboration.spawn_accepted + metrics.agent_collaboration.spawn_rejected;
    if denominator == 0 {
        return None;
    }
    Some((metrics.agent_collaboration.spawn_rejected * 10_000) / denominator)
}

fn build_trace_item(stored: &StoredEvent) -> Option<SessionDebugTraceItem> {
    let event = &stored.event;
    let agent_id = event.agent.agent_id.clone();
    let sub_run_id = event.agent.sub_run_id.clone();
    let turn_id = event.turn_id.clone();
    match &event.payload {
        StorageEventPayload::ToolCall {
            tool_call_id,
            tool_name,
            ..
        } => Some(SessionDebugTraceItem {
            id: format!("trace:{}", stored.storage_seq),
            storage_seq: stored.storage_seq,
            turn_id,
            recorded_at: None,
            kind: SessionDebugTraceItemKind::ToolCall,
            title: tool_name.clone(),
            summary: format!("tool call started: {tool_name}"),
            agent_id,
            sub_run_id,
            child_agent_id: None,
            delivery_id: None,
            tool_call_id: Some(tool_call_id.clone()),
            tool_name: Some(tool_name.clone()),
            lifecycle: None,
            last_turn_outcome: None,
        }),
        StorageEventPayload::ToolResult {
            tool_call_id,
            tool_name,
            success,
            error,
            duration_ms,
            ..
        } => Some(SessionDebugTraceItem {
            id: format!("trace:{}", stored.storage_seq),
            storage_seq: stored.storage_seq,
            turn_id,
            recorded_at: None,
            kind: SessionDebugTraceItemKind::ToolResult,
            title: tool_name.clone(),
            summary: if *success {
                format!("tool call completed in {duration_ms}ms")
            } else {
                format!(
                    "tool call failed in {duration_ms}ms{}",
                    error
                        .as_ref()
                        .map(|value| format!(": {value}"))
                        .unwrap_or_default()
                )
            },
            agent_id,
            sub_run_id,
            child_agent_id: None,
            delivery_id: None,
            tool_call_id: Some(tool_call_id.clone()),
            tool_name: Some(tool_name.clone()),
            lifecycle: None,
            last_turn_outcome: None,
        }),
        StorageEventPayload::PromptMetrics { metrics } => Some(SessionDebugTraceItem {
            id: format!("trace:{}", stored.storage_seq),
            storage_seq: stored.storage_seq,
            turn_id,
            recorded_at: None,
            kind: SessionDebugTraceItemKind::PromptMetrics,
            title: "prompt metrics".to_string(),
            summary: format!(
                "step={} tokens={} truncatedToolResults={}",
                metrics.step_index, metrics.estimated_tokens, metrics.truncated_tool_results
            ),
            agent_id,
            sub_run_id,
            child_agent_id: None,
            delivery_id: None,
            tool_call_id: None,
            tool_name: None,
            lifecycle: None,
            last_turn_outcome: None,
        }),
        StorageEventPayload::SubRunStarted {
            tool_call_id,
            timestamp,
            ..
        } => Some(SessionDebugTraceItem {
            id: format!("trace:{}", stored.storage_seq),
            storage_seq: stored.storage_seq,
            turn_id,
            recorded_at: *timestamp,
            kind: SessionDebugTraceItemKind::SubRunStarted,
            title: "sub-run started".to_string(),
            summary: "child execution accepted".to_string(),
            agent_id,
            sub_run_id,
            child_agent_id: None,
            delivery_id: None,
            tool_call_id: tool_call_id.clone(),
            tool_name: None,
            lifecycle: None,
            last_turn_outcome: None,
        }),
        StorageEventPayload::SubRunFinished {
            tool_call_id,
            result,
            step_count,
            estimated_tokens,
            timestamp,
        } => Some(SessionDebugTraceItem {
            id: format!("trace:{}", stored.storage_seq),
            storage_seq: stored.storage_seq,
            turn_id,
            recorded_at: *timestamp,
            kind: SessionDebugTraceItemKind::SubRunFinished,
            title: "sub-run finished".to_string(),
            summary: format!(
                "lifecycle={:?} outcome={:?} steps={} estTokens={}",
                result.lifecycle, result.last_turn_outcome, step_count, estimated_tokens
            ),
            agent_id,
            sub_run_id,
            child_agent_id: None,
            delivery_id: None,
            tool_call_id: tool_call_id.clone(),
            tool_name: None,
            lifecycle: Some(result.lifecycle),
            last_turn_outcome: result.last_turn_outcome,
        }),
        StorageEventPayload::ChildSessionNotification {
            notification,
            timestamp,
        } => Some(SessionDebugTraceItem {
            id: format!("trace:{}", stored.storage_seq),
            storage_seq: stored.storage_seq,
            turn_id,
            recorded_at: *timestamp,
            kind: SessionDebugTraceItemKind::ChildNotification,
            title: format!("{:?}", notification.kind).to_lowercase(),
            summary: notification.summary.clone(),
            agent_id,
            sub_run_id,
            child_agent_id: Some(notification.child_ref.agent_id.clone()),
            delivery_id: None,
            tool_call_id: notification.source_tool_call_id.clone(),
            tool_name: None,
            lifecycle: Some(notification.status),
            last_turn_outcome: None,
        }),
        StorageEventPayload::AgentCollaborationFact { fact, timestamp } => {
            Some(SessionDebugTraceItem {
                id: format!("trace:{}", stored.storage_seq),
                storage_seq: stored.storage_seq,
                turn_id,
                recorded_at: *timestamp,
                kind: SessionDebugTraceItemKind::CollaborationFact,
                title: format!("{:?}", fact.action).to_lowercase(),
                summary: collaboration_summary(fact),
                agent_id,
                sub_run_id,
                child_agent_id: fact.child_agent_id.clone(),
                delivery_id: fact.delivery_id.clone(),
                tool_call_id: fact.source_tool_call_id.clone(),
                tool_name: None,
                lifecycle: None,
                last_turn_outcome: None,
            })
        },
        StorageEventPayload::AgentMailboxQueued { payload } => Some(SessionDebugTraceItem {
            id: format!("trace:{}", stored.storage_seq),
            storage_seq: stored.storage_seq,
            turn_id,
            recorded_at: Some(payload.envelope.queued_at),
            kind: SessionDebugTraceItemKind::MailboxQueued,
            title: "mailbox queued".to_string(),
            summary: payload.envelope.message.clone(),
            agent_id,
            sub_run_id,
            child_agent_id: Some(payload.envelope.to_agent_id.clone()),
            delivery_id: Some(payload.envelope.delivery_id.clone()),
            tool_call_id: None,
            tool_name: None,
            lifecycle: Some(payload.envelope.sender_lifecycle_status),
            last_turn_outcome: payload.envelope.sender_last_turn_outcome,
        }),
        StorageEventPayload::AgentMailboxBatchStarted { payload } => Some(SessionDebugTraceItem {
            id: format!("trace:{}", stored.storage_seq),
            storage_seq: stored.storage_seq,
            turn_id,
            recorded_at: None,
            kind: SessionDebugTraceItemKind::MailboxBatchStarted,
            title: "mailbox batch started".to_string(),
            summary: format!("{} deliveries", payload.delivery_ids.len()),
            agent_id,
            sub_run_id,
            child_agent_id: Some(payload.target_agent_id.clone()),
            delivery_id: payload.delivery_ids.first().cloned(),
            tool_call_id: None,
            tool_name: None,
            lifecycle: None,
            last_turn_outcome: None,
        }),
        StorageEventPayload::AgentMailboxBatchAcked { payload } => Some(SessionDebugTraceItem {
            id: format!("trace:{}", stored.storage_seq),
            storage_seq: stored.storage_seq,
            turn_id,
            recorded_at: None,
            kind: SessionDebugTraceItemKind::MailboxBatchAcked,
            title: "mailbox batch acked".to_string(),
            summary: format!("{} deliveries", payload.delivery_ids.len()),
            agent_id,
            sub_run_id,
            child_agent_id: Some(payload.target_agent_id.clone()),
            delivery_id: payload.delivery_ids.first().cloned(),
            tool_call_id: None,
            tool_name: None,
            lifecycle: None,
            last_turn_outcome: None,
        }),
        StorageEventPayload::AgentMailboxDiscarded { payload } => Some(SessionDebugTraceItem {
            id: format!("trace:{}", stored.storage_seq),
            storage_seq: stored.storage_seq,
            turn_id,
            recorded_at: None,
            kind: SessionDebugTraceItemKind::MailboxDiscarded,
            title: "mailbox discarded".to_string(),
            summary: format!("{} deliveries", payload.delivery_ids.len()),
            agent_id,
            sub_run_id,
            child_agent_id: Some(payload.target_agent_id.clone()),
            delivery_id: payload.delivery_ids.first().cloned(),
            tool_call_id: None,
            tool_name: None,
            lifecycle: None,
            last_turn_outcome: None,
        }),
        StorageEventPayload::TurnDone { timestamp, reason } => Some(SessionDebugTraceItem {
            id: format!("trace:{}", stored.storage_seq),
            storage_seq: stored.storage_seq,
            turn_id,
            recorded_at: Some(*timestamp),
            kind: SessionDebugTraceItemKind::TurnDone,
            title: "turn done".to_string(),
            summary: reason
                .clone()
                .unwrap_or_else(|| "turn completed".to_string()),
            agent_id,
            sub_run_id,
            child_agent_id: None,
            delivery_id: None,
            tool_call_id: None,
            tool_name: None,
            lifecycle: None,
            last_turn_outcome: None,
        }),
        StorageEventPayload::Error { message, timestamp } => Some(SessionDebugTraceItem {
            id: format!("trace:{}", stored.storage_seq),
            storage_seq: stored.storage_seq,
            turn_id,
            recorded_at: *timestamp,
            kind: SessionDebugTraceItemKind::Error,
            title: "error".to_string(),
            summary: message.clone(),
            agent_id,
            sub_run_id,
            child_agent_id: None,
            delivery_id: None,
            tool_call_id: None,
            tool_name: None,
            lifecycle: None,
            last_turn_outcome: None,
        }),
        _ => None,
    }
}

fn collaboration_summary(fact: &astrcode_core::AgentCollaborationFact) -> String {
    let mut parts = vec![format!("{:?}->{:?}", fact.action, fact.outcome).to_lowercase()];
    if let Some(summary) = fact.summary.as_deref() {
        parts.push(summary.to_string());
    }
    if let Some(reason_code) = fact.reason_code.as_deref() {
        parts.push(format!("reason={reason_code}"));
    }
    if let Some(latency_ms) = fact.latency_ms {
        parts.push(format!("latency={}ms", latency_ms));
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::{Duration as ChronoDuration, Utc};

    use super::{RuntimeDebugTimelineSample, TimelineStore};

    #[test]
    fn timeline_store_discards_samples_outside_window() {
        let mut store = TimelineStore::default();
        let now = Utc::now();
        let old = now - ChronoDuration::minutes(10);
        store.record(
            old,
            Some(1_000),
            Some(2_000),
            Some(3_000),
            Duration::from_secs(300),
        );
        store.record(
            now,
            Some(1_100),
            Some(2_100),
            Some(3_100),
            Duration::from_secs(300),
        );

        let samples = store.snapshot(now, Duration::from_secs(300));
        assert_eq!(
            samples,
            vec![RuntimeDebugTimelineSample {
                collected_at: now,
                spawn_rejection_ratio_bps: Some(1_100),
                observe_to_action_ratio_bps: Some(2_100),
                child_reuse_ratio_bps: Some(3_100),
            }]
        );
    }
}
