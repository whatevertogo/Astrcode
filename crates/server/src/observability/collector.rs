//! 运行时可观测性指标收集器。
//!
//! `RuntimeObservabilityCollector` 实现 `RuntimeMetricsRecorder` 和
//! `ObservabilitySnapshotProvider` 两个 trait，在内存中聚合运行时指标：
//! - 子代理执行计时与终态统计
//! - 父级 reactivation 成功/失败计数
//! - delivery buffer 队列状态
//! - agent 协作事实追踪
//!
//! 快照通过 `snapshot()` 返回不可变结构供 API 层消费。

use std::{
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use astrcode_core::{
    AgentCollaborationActionKind, AgentCollaborationFact, AgentCollaborationOutcomeKind,
    AgentTurnOutcome, RuntimeMetricsRecorder, SubRunStorageMode,
};

use crate::{
    ObservabilitySnapshotProvider,
    observability::{
        AgentCollaborationScorecardSnapshot, ExecutionDiagnosticsSnapshot,
        OperationMetricsSnapshot, ReplayMetricsSnapshot, RuntimeObservabilitySnapshot,
        SubRunExecutionMetricsSnapshot,
    },
};

/// scorecard 中的比例统一使用 basis points，避免浮点数在跨层传输时失真。
const BASIS_POINTS_SCALE: u64 = 10_000;
/// 这些诊断字段对应的埋点尚未接入当前运行时，快照层先显式保留 0 占位。
const UNTRACKED_DIAGNOSTIC_COUNTER: u64 = 0;

#[derive(Default)]
struct OperationMetrics {
    total: AtomicU64,
    failures: AtomicU64,
    total_duration_ms: AtomicU64,
    last_duration_ms: AtomicU64,
    max_duration_ms: AtomicU64,
}

impl OperationMetrics {
    fn record(&self, duration_ms: u64, success: bool) {
        self.total.fetch_add(1, Ordering::Relaxed);
        if !success {
            self.failures.fetch_add(1, Ordering::Relaxed);
        }
        self.total_duration_ms
            .fetch_add(duration_ms, Ordering::Relaxed);
        self.last_duration_ms.store(duration_ms, Ordering::Relaxed);
        self.max_duration_ms
            .fetch_max(duration_ms, Ordering::Relaxed);
    }

    fn snapshot(&self) -> OperationMetricsSnapshot {
        OperationMetricsSnapshot {
            total: self.total.load(Ordering::Relaxed),
            failures: self.failures.load(Ordering::Relaxed),
            total_duration_ms: self.total_duration_ms.load(Ordering::Relaxed),
            last_duration_ms: self.last_duration_ms.load(Ordering::Relaxed),
            max_duration_ms: self.max_duration_ms.load(Ordering::Relaxed),
        }
    }
}

#[derive(Default)]
struct ReplayMetrics {
    totals: OperationMetrics,
    cache_hits: AtomicU64,
    disk_fallbacks: AtomicU64,
    recovered_events: AtomicU64,
}

impl ReplayMetrics {
    fn record(
        &self,
        duration_ms: u64,
        success: bool,
        used_disk_fallback: bool,
        recovered_events: u64,
    ) {
        self.totals.record(duration_ms, success);
        if used_disk_fallback {
            self.disk_fallbacks.fetch_add(1, Ordering::Relaxed);
        } else {
            self.cache_hits.fetch_add(1, Ordering::Relaxed);
        }
        self.recovered_events
            .fetch_add(recovered_events, Ordering::Relaxed);
    }

    fn snapshot(&self) -> ReplayMetricsSnapshot {
        ReplayMetricsSnapshot {
            totals: self.totals.snapshot(),
            cache_hits: self.cache_hits.load(Ordering::Relaxed),
            disk_fallbacks: self.disk_fallbacks.load(Ordering::Relaxed),
            recovered_events: self.recovered_events.load(Ordering::Relaxed),
        }
    }
}

#[derive(Default)]
struct SubRunMetrics {
    total: AtomicU64,
    failures: AtomicU64,
    completed: AtomicU64,
    cancelled: AtomicU64,
    independent_session_total: AtomicU64,
    total_duration_ms: AtomicU64,
    last_duration_ms: AtomicU64,
    total_steps: AtomicU64,
    last_step_count: AtomicU64,
    total_estimated_tokens: AtomicU64,
    last_estimated_tokens: AtomicU64,
}

impl SubRunMetrics {
    fn record(
        &self,
        duration_ms: u64,
        outcome: AgentTurnOutcome,
        step_count: Option<u32>,
        estimated_tokens: Option<u64>,
        storage_mode: Option<SubRunStorageMode>,
    ) {
        self.total.fetch_add(1, Ordering::Relaxed);
        self.total_duration_ms
            .fetch_add(duration_ms, Ordering::Relaxed);
        self.last_duration_ms.store(duration_ms, Ordering::Relaxed);
        match outcome {
            AgentTurnOutcome::Completed => {
                self.completed.fetch_add(1, Ordering::Relaxed);
            },
            AgentTurnOutcome::Failed => {
                self.failures.fetch_add(1, Ordering::Relaxed);
            },
            AgentTurnOutcome::Cancelled => {
                self.cancelled.fetch_add(1, Ordering::Relaxed);
            },
        }
        if matches!(storage_mode, Some(SubRunStorageMode::IndependentSession)) {
            self.independent_session_total
                .fetch_add(1, Ordering::Relaxed);
        }
        if let Some(step_count) = step_count {
            let step_count = step_count as u64;
            self.total_steps.fetch_add(step_count, Ordering::Relaxed);
            self.last_step_count.store(step_count, Ordering::Relaxed);
        }
        if let Some(estimated_tokens) = estimated_tokens {
            self.total_estimated_tokens
                .fetch_add(estimated_tokens, Ordering::Relaxed);
            self.last_estimated_tokens
                .store(estimated_tokens, Ordering::Relaxed);
        }
    }

    fn snapshot(&self) -> SubRunExecutionMetricsSnapshot {
        SubRunExecutionMetricsSnapshot {
            total: self.total.load(Ordering::Relaxed),
            failures: self.failures.load(Ordering::Relaxed),
            completed: self.completed.load(Ordering::Relaxed),
            cancelled: self.cancelled.load(Ordering::Relaxed),
            independent_session_total: self.independent_session_total.load(Ordering::Relaxed),
            total_duration_ms: self.total_duration_ms.load(Ordering::Relaxed),
            last_duration_ms: self.last_duration_ms.load(Ordering::Relaxed),
            total_steps: self.total_steps.load(Ordering::Relaxed),
            last_step_count: self.last_step_count.load(Ordering::Relaxed),
            total_estimated_tokens: self.total_estimated_tokens.load(Ordering::Relaxed),
            last_estimated_tokens: self.last_estimated_tokens.load(Ordering::Relaxed),
        }
    }
}

#[derive(Default)]
struct ExecutionDiagnostics {
    child_spawned: AtomicU64,
    parent_reactivation_requested: AtomicU64,
    parent_reactivation_succeeded: AtomicU64,
    parent_reactivation_failed: AtomicU64,
    cache_reuse_hits: AtomicU64,
    cache_reuse_misses: AtomicU64,
    delivery_buffer_queued: AtomicU64,
    delivery_buffer_dequeued: AtomicU64,
    delivery_buffer_wake_requested: AtomicU64,
    delivery_buffer_wake_succeeded: AtomicU64,
    delivery_buffer_wake_failed: AtomicU64,
}

impl ExecutionDiagnostics {
    fn snapshot(&self) -> ExecutionDiagnosticsSnapshot {
        ExecutionDiagnosticsSnapshot {
            child_spawned: self.child_spawned.load(Ordering::Relaxed),
            child_started_persisted: UNTRACKED_DIAGNOSTIC_COUNTER,
            child_terminal_persisted: UNTRACKED_DIAGNOSTIC_COUNTER,
            parent_reactivation_requested: self
                .parent_reactivation_requested
                .load(Ordering::Relaxed),
            parent_reactivation_succeeded: self
                .parent_reactivation_succeeded
                .load(Ordering::Relaxed),
            parent_reactivation_failed: self.parent_reactivation_failed.load(Ordering::Relaxed),
            lineage_mismatch_parent_agent: UNTRACKED_DIAGNOSTIC_COUNTER,
            lineage_mismatch_parent_session: UNTRACKED_DIAGNOSTIC_COUNTER,
            lineage_mismatch_child_session: UNTRACKED_DIAGNOSTIC_COUNTER,
            lineage_mismatch_descriptor_missing: UNTRACKED_DIAGNOSTIC_COUNTER,
            cache_reuse_hits: self.cache_reuse_hits.load(Ordering::Relaxed),
            cache_reuse_misses: self.cache_reuse_misses.load(Ordering::Relaxed),
            delivery_buffer_queued: self.delivery_buffer_queued.load(Ordering::Relaxed),
            delivery_buffer_dequeued: self.delivery_buffer_dequeued.load(Ordering::Relaxed),
            delivery_buffer_wake_requested: self
                .delivery_buffer_wake_requested
                .load(Ordering::Relaxed),
            delivery_buffer_wake_succeeded: self
                .delivery_buffer_wake_succeeded
                .load(Ordering::Relaxed),
            delivery_buffer_wake_failed: self.delivery_buffer_wake_failed.load(Ordering::Relaxed),
        }
    }
}

#[derive(Default)]
struct ChildCollaborationState {
    had_reuse: bool,
    had_delivery: bool,
    closed: bool,
}

#[derive(Default)]
struct PendingObserveState {
    satisfied: bool,
}

#[derive(Default)]
struct CollaborationMetricsState {
    total_facts: u64,
    spawn_accepted: u64,
    spawn_rejected: u64,
    send_reused: u64,
    send_queued: u64,
    send_rejected: u64,
    observe_calls: u64,
    observe_rejected: u64,
    observe_followed_by_action: u64,
    close_calls: u64,
    close_rejected: u64,
    delivery_delivered: u64,
    delivery_consumed: u64,
    delivery_replayed: u64,
    delivery_latency_total_ms: u64,
    delivery_latency_samples: u64,
    max_delivery_latency_ms: u64,
    child_states: HashMap<String, ChildCollaborationState>,
    pending_observes: HashMap<String, PendingObserveState>,
}

impl CollaborationMetricsState {
    fn record(&mut self, fact: &AgentCollaborationFact) {
        self.total_facts = self.total_facts.saturating_add(1);
        match fact.action {
            AgentCollaborationActionKind::Spawn => self.record_spawn(fact),
            AgentCollaborationActionKind::Send => self.record_send(fact),
            AgentCollaborationActionKind::Observe => self.record_observe(fact),
            AgentCollaborationActionKind::Close => self.record_close(fact),
            AgentCollaborationActionKind::Delivery => self.record_delivery(fact),
        }
    }

    fn record_spawn(&mut self, fact: &AgentCollaborationFact) {
        match fact.outcome {
            AgentCollaborationOutcomeKind::Accepted => {
                self.spawn_accepted = self.spawn_accepted.saturating_add(1);
                if let Some(child_id) = fact.child_agent_id().map(|id| id.as_str()) {
                    self.child_states.entry(child_id.to_string()).or_default();
                }
            },
            AgentCollaborationOutcomeKind::Rejected | AgentCollaborationOutcomeKind::Failed => {
                self.spawn_rejected = self.spawn_rejected.saturating_add(1);
            },
            _ => {},
        }
    }

    fn record_send(&mut self, fact: &AgentCollaborationFact) {
        match fact.outcome {
            AgentCollaborationOutcomeKind::Reused => {
                self.send_reused = self.send_reused.saturating_add(1);
                self.mark_child_reused(fact.child_agent_id().map(|id| id.as_str()));
                self.satisfy_pending_observe(fact.child_agent_id().map(|id| id.as_str()));
            },
            AgentCollaborationOutcomeKind::Queued => {
                self.send_queued = self.send_queued.saturating_add(1);
                self.mark_child_reused(fact.child_agent_id().map(|id| id.as_str()));
                self.satisfy_pending_observe(fact.child_agent_id().map(|id| id.as_str()));
            },
            AgentCollaborationOutcomeKind::Rejected | AgentCollaborationOutcomeKind::Failed => {
                self.send_rejected = self.send_rejected.saturating_add(1);
            },
            _ => {},
        }
    }

    fn record_observe(&mut self, fact: &AgentCollaborationFact) {
        match fact.outcome {
            AgentCollaborationOutcomeKind::Accepted => {
                self.observe_calls = self.observe_calls.saturating_add(1);
                if let Some(child_id) = fact.child_agent_id().map(|id| id.as_str()) {
                    self.pending_observes
                        .entry(child_id.to_string())
                        .or_default();
                }
            },
            AgentCollaborationOutcomeKind::Rejected | AgentCollaborationOutcomeKind::Failed => {
                self.observe_rejected = self.observe_rejected.saturating_add(1);
            },
            _ => {},
        }
    }

    fn record_close(&mut self, fact: &AgentCollaborationFact) {
        match fact.outcome {
            AgentCollaborationOutcomeKind::Closed => {
                self.close_calls = self.close_calls.saturating_add(1);
                if let Some(child_id) = fact.child_agent_id().map(|id| id.as_str()) {
                    self.child_states
                        .entry(child_id.to_string())
                        .or_default()
                        .closed = true;
                    self.satisfy_pending_observe(Some(child_id));
                }
            },
            AgentCollaborationOutcomeKind::Rejected | AgentCollaborationOutcomeKind::Failed => {
                self.close_rejected = self.close_rejected.saturating_add(1);
            },
            _ => {},
        }
    }

    fn record_delivery(&mut self, fact: &AgentCollaborationFact) {
        match fact.outcome {
            AgentCollaborationOutcomeKind::Delivered => {
                self.delivery_delivered = self.delivery_delivered.saturating_add(1);
                if let Some(child_id) = fact.child_agent_id().map(|id| id.as_str()) {
                    self.child_states
                        .entry(child_id.to_string())
                        .or_default()
                        .had_delivery = true;
                }
            },
            AgentCollaborationOutcomeKind::Consumed => {
                self.delivery_consumed = self.delivery_consumed.saturating_add(1);
                self.record_delivery_latency(fact.latency_ms);
            },
            AgentCollaborationOutcomeKind::Replayed => {
                self.delivery_replayed = self.delivery_replayed.saturating_add(1);
            },
            _ => {},
        }
    }

    fn record_delivery_latency(&mut self, latency_ms: Option<u64>) {
        let Some(latency_ms) = latency_ms else {
            return;
        };
        self.delivery_latency_total_ms = self.delivery_latency_total_ms.saturating_add(latency_ms);
        self.delivery_latency_samples = self.delivery_latency_samples.saturating_add(1);
        self.max_delivery_latency_ms = self.max_delivery_latency_ms.max(latency_ms);
    }

    fn mark_child_reused(&mut self, child_id: Option<&str>) {
        if let Some(child_id) = child_id {
            self.child_states
                .entry(child_id.to_string())
                .or_default()
                .had_reuse = true;
        }
    }

    fn satisfy_pending_observe(&mut self, child_id: Option<&str>) {
        let Some(child_id) = child_id else {
            return;
        };
        if let Some(observe) = self.pending_observes.get_mut(child_id) {
            if !observe.satisfied {
                observe.satisfied = true;
                self.observe_followed_by_action = self.observe_followed_by_action.saturating_add(1);
            }
        }
    }

    fn snapshot(&self) -> AgentCollaborationScorecardSnapshot {
        let orphan_child_count = self
            .child_states
            .values()
            .filter(|state| !state.had_reuse && !state.had_delivery && !state.closed)
            .count() as u64;
        let reuse_numerator = self.send_reused.saturating_add(self.send_queued);
        let delivery_ratio_denominator = self.spawn_accepted;

        AgentCollaborationScorecardSnapshot {
            total_facts: self.total_facts,
            spawn_accepted: self.spawn_accepted,
            spawn_rejected: self.spawn_rejected,
            send_reused: self.send_reused,
            send_queued: self.send_queued,
            send_rejected: self.send_rejected,
            observe_calls: self.observe_calls,
            observe_rejected: self.observe_rejected,
            observe_followed_by_action: self.observe_followed_by_action,
            close_calls: self.close_calls,
            close_rejected: self.close_rejected,
            delivery_delivered: self.delivery_delivered,
            delivery_consumed: self.delivery_consumed,
            delivery_replayed: self.delivery_replayed,
            orphan_child_count,
            child_reuse_ratio_bps: ratio_bps(
                reuse_numerator,
                self.spawn_accepted.saturating_add(reuse_numerator),
            ),
            observe_to_action_ratio_bps: ratio_bps(
                self.observe_followed_by_action,
                self.observe_calls,
            ),
            spawn_to_delivery_ratio_bps: ratio_bps(
                self.delivery_delivered,
                delivery_ratio_denominator,
            ),
            orphan_child_ratio_bps: ratio_bps(orphan_child_count, self.spawn_accepted),
            avg_delivery_latency_ms: self
                .delivery_latency_total_ms
                .checked_div(self.delivery_latency_samples),
            max_delivery_latency_ms: if self.delivery_latency_samples > 0 {
                Some(self.max_delivery_latency_ms)
            } else {
                None
            },
        }
    }
}

fn ratio_bps(numerator: u64, denominator: u64) -> Option<u64> {
    numerator
        .saturating_mul(BASIS_POINTS_SCALE)
        .checked_div(denominator)
}

/// 真实运行时观测采集器。
#[derive(Default)]
pub struct RuntimeObservabilityCollector {
    session_rehydrate: OperationMetrics,
    sse_catch_up: ReplayMetrics,
    turn_execution: OperationMetrics,
    subrun_execution: SubRunMetrics,
    diagnostics: ExecutionDiagnostics,
    collaboration: Mutex<CollaborationMetricsState>,
}

impl RuntimeObservabilityCollector {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ObservabilitySnapshotProvider for RuntimeObservabilityCollector {
    fn snapshot(&self) -> RuntimeObservabilitySnapshot {
        RuntimeObservabilitySnapshot {
            session_rehydrate: self.session_rehydrate.snapshot(),
            sse_catch_up: self.sse_catch_up.snapshot(),
            turn_execution: self.turn_execution.snapshot(),
            subrun_execution: self.subrun_execution.snapshot(),
            execution_diagnostics: self.diagnostics.snapshot(),
            agent_collaboration: self
                .collaboration
                .lock()
                .expect("collaboration metrics mutex")
                .snapshot(),
        }
    }
}

impl RuntimeMetricsRecorder for RuntimeObservabilityCollector {
    fn record_session_rehydrate(&self, duration_ms: u64, success: bool) {
        self.session_rehydrate.record(duration_ms, success);
    }

    fn record_sse_catch_up(
        &self,
        duration_ms: u64,
        success: bool,
        used_disk_fallback: bool,
        recovered_events: u64,
    ) {
        self.sse_catch_up
            .record(duration_ms, success, used_disk_fallback, recovered_events);
    }

    fn record_turn_execution(&self, duration_ms: u64, success: bool) {
        self.turn_execution.record(duration_ms, success);
    }

    fn record_subrun_execution(
        &self,
        duration_ms: u64,
        outcome: AgentTurnOutcome,
        step_count: Option<u32>,
        estimated_tokens: Option<u64>,
        storage_mode: Option<SubRunStorageMode>,
    ) {
        self.subrun_execution.record(
            duration_ms,
            outcome,
            step_count,
            estimated_tokens,
            storage_mode,
        );
    }

    fn record_child_spawned(&self) {
        self.diagnostics
            .child_spawned
            .fetch_add(1, Ordering::Relaxed);
    }

    fn record_parent_reactivation_requested(&self) {
        self.diagnostics
            .parent_reactivation_requested
            .fetch_add(1, Ordering::Relaxed);
    }

    fn record_parent_reactivation_succeeded(&self) {
        self.diagnostics
            .parent_reactivation_succeeded
            .fetch_add(1, Ordering::Relaxed);
    }

    fn record_parent_reactivation_failed(&self) {
        self.diagnostics
            .parent_reactivation_failed
            .fetch_add(1, Ordering::Relaxed);
    }

    fn record_delivery_buffer_queued(&self) {
        self.diagnostics
            .delivery_buffer_queued
            .fetch_add(1, Ordering::Relaxed);
    }

    fn record_delivery_buffer_dequeued(&self) {
        self.diagnostics
            .delivery_buffer_dequeued
            .fetch_add(1, Ordering::Relaxed);
    }

    fn record_delivery_buffer_wake_requested(&self) {
        self.diagnostics
            .delivery_buffer_wake_requested
            .fetch_add(1, Ordering::Relaxed);
    }

    fn record_delivery_buffer_wake_succeeded(&self) {
        self.diagnostics
            .delivery_buffer_wake_succeeded
            .fetch_add(1, Ordering::Relaxed);
    }

    fn record_delivery_buffer_wake_failed(&self) {
        self.diagnostics
            .delivery_buffer_wake_failed
            .fetch_add(1, Ordering::Relaxed);
    }

    fn record_cache_reuse_hit(&self) {
        self.diagnostics
            .cache_reuse_hits
            .fetch_add(1, Ordering::Relaxed);
    }

    fn record_cache_reuse_miss(&self) {
        self.diagnostics
            .cache_reuse_misses
            .fetch_add(1, Ordering::Relaxed);
    }

    fn record_agent_collaboration_fact(&self, fact: &AgentCollaborationFact) {
        self.collaboration
            .lock()
            .expect("collaboration metrics mutex")
            .record(fact);
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{
        AgentCollaborationActionKind, AgentCollaborationFact, AgentCollaborationOutcomeKind,
        AgentCollaborationPolicyContext, AgentTurnOutcome, ChildExecutionIdentity,
        RuntimeMetricsRecorder, SubRunStorageMode,
    };

    use super::RuntimeObservabilityCollector;
    use crate::ObservabilitySnapshotProvider;

    #[test]
    fn collector_snapshot_reflects_recorded_activity() {
        let collector = RuntimeObservabilityCollector::new();

        collector.record_session_rehydrate(15, true);
        collector.record_sse_catch_up(20, false, true, 7);
        collector.record_turn_execution(30, true);
        collector.record_subrun_execution(
            12,
            AgentTurnOutcome::Completed,
            Some(3),
            Some(1200),
            Some(SubRunStorageMode::IndependentSession),
        );
        collector.record_child_spawned();
        collector.record_parent_reactivation_requested();
        collector.record_parent_reactivation_succeeded();
        collector.record_delivery_buffer_queued();
        collector.record_delivery_buffer_dequeued();
        collector.record_delivery_buffer_wake_requested();
        collector.record_delivery_buffer_wake_succeeded();

        let snapshot = collector.snapshot();
        assert_eq!(snapshot.session_rehydrate.total, 1);
        assert_eq!(snapshot.sse_catch_up.totals.failures, 1);
        assert_eq!(snapshot.sse_catch_up.disk_fallbacks, 1);
        assert_eq!(snapshot.sse_catch_up.recovered_events, 7);
        assert_eq!(snapshot.turn_execution.total_duration_ms, 30);
        assert_eq!(snapshot.subrun_execution.completed, 1);
        assert_eq!(snapshot.subrun_execution.total_steps, 3);
        assert_eq!(snapshot.subrun_execution.total_estimated_tokens, 1200);
        assert_eq!(snapshot.execution_diagnostics.child_spawned, 1);
        assert_eq!(
            snapshot.execution_diagnostics.parent_reactivation_requested,
            1
        );
        assert_eq!(
            snapshot.execution_diagnostics.parent_reactivation_succeeded,
            1
        );
        assert_eq!(snapshot.execution_diagnostics.delivery_buffer_queued, 1);
        assert_eq!(snapshot.execution_diagnostics.delivery_buffer_dequeued, 1);
        assert_eq!(
            snapshot
                .execution_diagnostics
                .delivery_buffer_wake_requested,
            1
        );
        assert_eq!(
            snapshot
                .execution_diagnostics
                .delivery_buffer_wake_succeeded,
            1
        );
    }

    #[test]
    fn collector_snapshot_derives_collaboration_scorecard() {
        let collector = RuntimeObservabilityCollector::new();
        let policy = AgentCollaborationPolicyContext {
            policy_revision: "governance-surface-v1".to_string(),
            max_subrun_depth: 3,
            max_spawn_per_turn: 3,
        };

        collector.record_agent_collaboration_fact(&AgentCollaborationFact {
            fact_id: "fact-spawn".to_string().into(),
            action: AgentCollaborationActionKind::Spawn,
            outcome: AgentCollaborationOutcomeKind::Accepted,
            parent_session_id: "session-parent".to_string().into(),
            turn_id: "turn-1".to_string().into(),
            parent_agent_id: Some("agent-root".to_string().into()),
            child_identity: Some(ChildExecutionIdentity {
                agent_id: "agent-child".to_string().into(),
                session_id: "session-child".to_string().into(),
                sub_run_id: "subrun-child".to_string().into(),
            }),
            delivery_id: None,
            reason_code: None,
            summary: Some("spawned".to_string()),
            latency_ms: None,
            source_tool_call_id: Some("call-1".to_string().into()),
            governance_revision: Some("governance-surface-v1".to_string()),
            mode_id: Some(astrcode_governance_contract::ModeId::code()),
            policy: policy.clone(),
        });
        collector.record_agent_collaboration_fact(&AgentCollaborationFact {
            fact_id: "fact-observe".to_string().into(),
            action: AgentCollaborationActionKind::Observe,
            outcome: AgentCollaborationOutcomeKind::Accepted,
            parent_session_id: "session-parent".to_string().into(),
            turn_id: "turn-1".to_string().into(),
            parent_agent_id: Some("agent-root".to_string().into()),
            child_identity: Some(ChildExecutionIdentity {
                agent_id: "agent-child".to_string().into(),
                session_id: "session-child".to_string().into(),
                sub_run_id: "subrun-child".to_string().into(),
            }),
            delivery_id: None,
            reason_code: None,
            summary: Some("observe".to_string()),
            latency_ms: None,
            source_tool_call_id: Some("call-2".to_string().into()),
            governance_revision: Some("governance-surface-v1".to_string()),
            mode_id: Some(astrcode_governance_contract::ModeId::code()),
            policy: policy.clone(),
        });
        collector.record_agent_collaboration_fact(&AgentCollaborationFact {
            fact_id: "fact-send".to_string().into(),
            action: AgentCollaborationActionKind::Send,
            outcome: AgentCollaborationOutcomeKind::Reused,
            parent_session_id: "session-parent".to_string().into(),
            turn_id: "turn-1".to_string().into(),
            parent_agent_id: Some("agent-root".to_string().into()),
            child_identity: Some(ChildExecutionIdentity {
                agent_id: "agent-child".to_string().into(),
                session_id: "session-child".to_string().into(),
                sub_run_id: "subrun-child".to_string().into(),
            }),
            delivery_id: None,
            reason_code: None,
            summary: Some("reused".to_string()),
            latency_ms: None,
            source_tool_call_id: Some("call-3".to_string().into()),
            governance_revision: Some("governance-surface-v1".to_string()),
            mode_id: Some(astrcode_governance_contract::ModeId::code()),
            policy: policy.clone(),
        });
        collector.record_agent_collaboration_fact(&AgentCollaborationFact {
            fact_id: "fact-delivery".to_string().into(),
            action: AgentCollaborationActionKind::Delivery,
            outcome: AgentCollaborationOutcomeKind::Consumed,
            parent_session_id: "session-parent".to_string().into(),
            turn_id: "turn-2".to_string().into(),
            parent_agent_id: Some("agent-root".to_string().into()),
            child_identity: Some(ChildExecutionIdentity {
                agent_id: "agent-child".to_string().into(),
                session_id: "session-child".to_string().into(),
                sub_run_id: "subrun-child".to_string().into(),
            }),
            delivery_id: Some("delivery-1".to_string().into()),
            reason_code: None,
            summary: Some("consumed".to_string()),
            latency_ms: Some(250),
            source_tool_call_id: None,
            governance_revision: Some("governance-surface-v1".to_string()),
            mode_id: Some(astrcode_governance_contract::ModeId::code()),
            policy,
        });

        let snapshot = collector.snapshot();
        assert_eq!(snapshot.agent_collaboration.total_facts, 4);
        assert_eq!(snapshot.agent_collaboration.spawn_accepted, 1);
        assert_eq!(snapshot.agent_collaboration.send_reused, 1);
        assert_eq!(snapshot.agent_collaboration.observe_calls, 1);
        assert_eq!(snapshot.agent_collaboration.observe_followed_by_action, 1);
        assert_eq!(snapshot.agent_collaboration.delivery_consumed, 1);
        assert_eq!(
            snapshot.agent_collaboration.child_reuse_ratio_bps,
            Some(5000)
        );
        assert_eq!(
            snapshot.agent_collaboration.observe_to_action_ratio_bps,
            Some(10000)
        );
        assert_eq!(
            snapshot.agent_collaboration.avg_delivery_latency_ms,
            Some(250)
        );
    }
}
