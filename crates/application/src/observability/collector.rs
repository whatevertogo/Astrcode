use std::sync::atomic::{AtomicU64, Ordering};

use astrcode_core::{RuntimeMetricsRecorder, SubRunExecutionOutcome, SubRunStorageMode};

use crate::{
    ObservabilitySnapshotProvider,
    observability::{
        ExecutionDiagnosticsSnapshot, OperationMetricsSnapshot, ReplayMetricsSnapshot,
        RuntimeObservabilitySnapshot, SubRunExecutionMetricsSnapshot,
    },
};

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
    aborted: AtomicU64,
    token_exceeded: AtomicU64,
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
        outcome: SubRunExecutionOutcome,
        step_count: Option<u32>,
        estimated_tokens: Option<u64>,
        storage_mode: Option<SubRunStorageMode>,
    ) {
        self.total.fetch_add(1, Ordering::Relaxed);
        self.total_duration_ms
            .fetch_add(duration_ms, Ordering::Relaxed);
        self.last_duration_ms.store(duration_ms, Ordering::Relaxed);
        match outcome {
            SubRunExecutionOutcome::Completed => {
                self.completed.fetch_add(1, Ordering::Relaxed);
            },
            SubRunExecutionOutcome::Failed => {
                self.failures.fetch_add(1, Ordering::Relaxed);
            },
            SubRunExecutionOutcome::Aborted => {
                self.aborted.fetch_add(1, Ordering::Relaxed);
            },
            SubRunExecutionOutcome::TokenExceeded => {
                self.token_exceeded.fetch_add(1, Ordering::Relaxed);
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
            aborted: self.aborted.load(Ordering::Relaxed),
            token_exceeded: self.token_exceeded.load(Ordering::Relaxed),
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
            child_started_persisted: 0,
            child_terminal_persisted: 0,
            parent_reactivation_requested: self
                .parent_reactivation_requested
                .load(Ordering::Relaxed),
            parent_reactivation_succeeded: self
                .parent_reactivation_succeeded
                .load(Ordering::Relaxed),
            parent_reactivation_failed: self.parent_reactivation_failed.load(Ordering::Relaxed),
            lineage_mismatch_parent_agent: 0,
            lineage_mismatch_parent_session: 0,
            lineage_mismatch_child_session: 0,
            lineage_mismatch_descriptor_missing: 0,
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

/// 真实运行时观测采集器。
#[derive(Default)]
pub struct RuntimeObservabilityCollector {
    session_rehydrate: OperationMetrics,
    sse_catch_up: ReplayMetrics,
    turn_execution: OperationMetrics,
    subrun_execution: SubRunMetrics,
    diagnostics: ExecutionDiagnostics,
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
        outcome: SubRunExecutionOutcome,
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
}

#[cfg(test)]
mod tests {
    use astrcode_core::{RuntimeMetricsRecorder, SubRunExecutionOutcome, SubRunStorageMode};

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
            SubRunExecutionOutcome::Completed,
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
}
