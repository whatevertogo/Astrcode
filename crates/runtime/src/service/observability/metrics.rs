//! # 可观测性 (Observability)
//!
//! 收集运行时服务的操作指标，包括：
//! - 会话重水合（Session Rehydrate）：加载已有会话的成功率和耗时
//! - SSE 追赶（SSE Catch-up）：客户端重连时回放历史的路径和恢复事件数
//! - Turn 执行（Turn Execution）：Turn 执行的成功率和耗时
//!
//! ## 设计
//!
//! 使用原子计数器（`AtomicU64`）记录指标，避免锁竞争。
//! 所有记录操作都是无锁的，适合高频调用。
//! 快照（`snapshot()`）返回当前指标的只读副本，供外部查询。

use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use astrcode_core::{AgentLifecycleStatus, AgentTurnOutcome, SubRunStorageMode};
use astrcode_runtime_execution::{
    ChildLifecycleStage, DeliveryBufferStage, LegacyRejectionKind, LineageMismatchKind,
};

/// 回放路径：优先缓存，不足时回退到磁盘。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayPath {
    /// 从内存缓存读取（快速路径）
    Cache,
    /// 从磁盘 JSONL 文件加载（慢速回退路径）
    DiskFallback,
}

/// 单一操作的指标快照。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OperationMetricsSnapshot {
    /// 总操作次数
    pub total: u64,
    /// 失败次数
    pub failures: u64,
    /// 累计耗时（毫秒）
    pub total_duration_ms: u64,
    /// 最近一次操作的耗时（毫秒）
    pub last_duration_ms: u64,
    /// 历史最大单次操作耗时（毫秒）
    pub max_duration_ms: u64,
}

/// SSE 回放操作的指标快照。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReplayMetricsSnapshot {
    /// 基础操作指标（总次数、失败率、耗时等）
    pub totals: OperationMetricsSnapshot,
    /// 缓存命中次数
    pub cache_hits: u64,
    /// 磁盘回退次数（说明缓存不足的情况）
    pub disk_fallbacks: u64,
    /// 成功恢复的事件总数
    pub recovered_events: u64,
}

/// 运行时可观测性快照，包含三类操作的指标。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeObservabilitySnapshot {
    /// 会话重水合（从磁盘加载已有会话）的指标
    pub session_rehydrate: OperationMetricsSnapshot,
    /// SSE 追赶（客户端重连时回放历史）的指标
    pub sse_catch_up: ReplayMetricsSnapshot,
    /// Turn 执行的指标
    pub turn_execution: OperationMetricsSnapshot,
    /// 子执行域共享观测指标
    pub subrun_execution: SubRunExecutionMetricsSnapshot,
    /// 子会话/缓存/legacy cutover 的结构化观测指标
    pub execution_diagnostics: ExecutionDiagnosticsSnapshot,
}

#[derive(Default)]
pub struct RuntimeObservability {
    session_rehydrate: OperationMetrics,
    sse_catch_up: ReplayMetrics,
    turn_execution: OperationMetrics,
    subrun_execution: SubRunExecutionMetrics,
    execution_diagnostics: ExecutionDiagnosticsMetrics,
}

impl RuntimeObservability {
    pub fn record_session_rehydrate(&self, duration: Duration, ok: bool) {
        self.session_rehydrate.record(duration, ok);
    }

    pub fn record_sse_catch_up(
        &self,
        duration: Duration,
        ok: bool,
        path: ReplayPath,
        recovered_events: usize,
    ) {
        self.sse_catch_up
            .record(duration, ok, path, recovered_events as u64);
    }

    pub fn record_turn_execution(&self, duration: Duration, ok: bool) {
        self.turn_execution.record(duration, ok);
    }

    pub fn record_subrun_execution(
        &self,
        duration: Duration,
        lifecycle: &AgentLifecycleStatus,
        last_turn_outcome: &Option<AgentTurnOutcome>,
        storage_mode: SubRunStorageMode,
        step_count: u32,
        estimated_tokens: u64,
    ) {
        self.subrun_execution.record(
            duration,
            lifecycle,
            last_turn_outcome,
            storage_mode,
            u64::from(step_count),
            estimated_tokens,
        );
    }

    pub fn record_child_lifecycle(&self, stage: ChildLifecycleStage) {
        self.execution_diagnostics.record_child_lifecycle(stage);
    }

    pub fn record_lineage_mismatch(&self, kind: LineageMismatchKind) {
        self.execution_diagnostics.record_lineage_mismatch(kind);
    }

    pub fn record_cache_reuse_hits(&self, count: u64) {
        self.execution_diagnostics.record_cache_reuse_hits(count);
    }

    pub fn record_cache_reuse_misses(&self, count: u64) {
        self.execution_diagnostics.record_cache_reuse_misses(count);
    }

    pub fn record_delivery_buffer(&self, stage: DeliveryBufferStage) {
        self.execution_diagnostics.record_delivery_buffer(stage);
    }

    #[allow(dead_code)]
    pub fn record_legacy_rejection(&self, kind: LegacyRejectionKind) {
        log::warn!(
            "execution diagnostics: legacy rejection recorded ({})",
            kind.as_str()
        );
        self.execution_diagnostics.record_legacy_rejection(kind);
    }

    pub fn snapshot(&self) -> RuntimeObservabilitySnapshot {
        RuntimeObservabilitySnapshot {
            session_rehydrate: self.session_rehydrate.snapshot(),
            sse_catch_up: self.sse_catch_up.snapshot(),
            turn_execution: self.turn_execution.snapshot(),
            subrun_execution: self.subrun_execution.snapshot(),
            execution_diagnostics: self.execution_diagnostics.snapshot(),
        }
    }
}

/// 结构化执行诊断快照。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecutionDiagnosticsSnapshot {
    pub child_spawned: u64,
    pub child_started_persisted: u64,
    pub child_terminal_persisted: u64,
    pub parent_reactivation_requested: u64,
    pub parent_reactivation_succeeded: u64,
    pub parent_reactivation_failed: u64,
    pub lineage_mismatch_parent_agent: u64,
    pub lineage_mismatch_parent_session: u64,
    pub lineage_mismatch_child_session: u64,
    pub lineage_mismatch_descriptor_missing: u64,
    pub cache_reuse_hits: u64,
    pub cache_reuse_misses: u64,
    pub delivery_buffer_queued: u64,
    pub delivery_buffer_dequeued: u64,
    pub delivery_buffer_wake_requested: u64,
    pub delivery_buffer_wake_succeeded: u64,
    pub delivery_buffer_wake_failed: u64,
    pub legacy_shared_history_rejections: u64,
}

/// 子执行域共享观测指标快照。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubRunExecutionMetricsSnapshot {
    pub total: u64,
    pub failures: u64,
    pub completed: u64,
    pub aborted: u64,
    pub token_exceeded: u64,
    pub shared_session_total: u64,
    pub independent_session_total: u64,
    pub total_duration_ms: u64,
    pub last_duration_ms: u64,
    pub total_steps: u64,
    pub last_step_count: u64,
    pub total_estimated_tokens: u64,
    pub last_estimated_tokens: u64,
}

/// 单一操作的指标收集器，使用原子计数器避免锁竞争。
#[derive(Default)]
struct OperationMetrics {
    /// 总操作次数
    total: AtomicU64,
    /// 失败次数
    failures: AtomicU64,
    /// 累计耗时（毫秒）
    total_duration_ms: AtomicU64,
    /// 最近一次操作的耗时（毫秒）
    last_duration_ms: AtomicU64,
    /// 历史最大单次操作耗时（毫秒）
    max_duration_ms: AtomicU64,
}

impl OperationMetrics {
    fn record(&self, duration: Duration, ok: bool) {
        let elapsed_ms = saturating_duration_ms(duration);
        self.total.fetch_add(1, Ordering::Relaxed);
        if !ok {
            self.failures.fetch_add(1, Ordering::Relaxed);
        }
        self.total_duration_ms
            .fetch_add(elapsed_ms, Ordering::Relaxed);
        self.last_duration_ms.store(elapsed_ms, Ordering::Relaxed);

        // 故意忽略：并发竞争时 CAS 失败是正常的，取最大值不需要精确
        let _ =
            self.max_duration_ms
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                    (elapsed_ms > current).then_some(elapsed_ms)
                });
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

/// SSE 回放指标收集器，在基础操作指标之上增加缓存/磁盘路径统计。
#[derive(Default)]
struct ReplayMetrics {
    /// 基础操作指标
    totals: OperationMetrics,
    /// 缓存命中次数
    cache_hits: AtomicU64,
    /// 磁盘回退次数
    disk_fallbacks: AtomicU64,
    /// 成功恢复的事件总数
    recovered_events: AtomicU64,
}

impl ReplayMetrics {
    fn record(&self, duration: Duration, ok: bool, path: ReplayPath, recovered_events: u64) {
        self.totals.record(duration, ok);
        match path {
            ReplayPath::Cache => {
                self.cache_hits.fetch_add(1, Ordering::Relaxed);
            },
            ReplayPath::DiskFallback => {
                self.disk_fallbacks.fetch_add(1, Ordering::Relaxed);
            },
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

fn saturating_duration_ms(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

#[derive(Default)]
struct SubRunExecutionMetrics {
    total: AtomicU64,
    failures: AtomicU64,
    completed: AtomicU64,
    aborted: AtomicU64,
    token_exceeded: AtomicU64,
    shared_session_total: AtomicU64,
    independent_session_total: AtomicU64,
    total_duration_ms: AtomicU64,
    last_duration_ms: AtomicU64,
    total_steps: AtomicU64,
    last_step_count: AtomicU64,
    total_estimated_tokens: AtomicU64,
    last_estimated_tokens: AtomicU64,
}

impl SubRunExecutionMetrics {
    fn record(
        &self,
        duration: Duration,
        lifecycle: &AgentLifecycleStatus,
        last_turn_outcome: &Option<AgentTurnOutcome>,
        storage_mode: SubRunStorageMode,
        step_count: u64,
        estimated_tokens: u64,
    ) {
        let elapsed_ms = saturating_duration_ms(duration);
        self.total.fetch_add(1, Ordering::Relaxed);
        self.total_duration_ms
            .fetch_add(elapsed_ms, Ordering::Relaxed);
        self.last_duration_ms.store(elapsed_ms, Ordering::Relaxed);
        self.total_steps.fetch_add(step_count, Ordering::Relaxed);
        self.last_step_count.store(step_count, Ordering::Relaxed);
        self.total_estimated_tokens
            .fetch_add(estimated_tokens, Ordering::Relaxed);
        self.last_estimated_tokens
            .store(estimated_tokens, Ordering::Relaxed);

        match storage_mode {
            SubRunStorageMode::SharedSession => {
                self.shared_session_total.fetch_add(1, Ordering::Relaxed);
            },
            SubRunStorageMode::IndependentSession => {
                self.independent_session_total
                    .fetch_add(1, Ordering::Relaxed);
            },
        }

        match last_turn_outcome {
            None => match lifecycle {
                AgentLifecycleStatus::Pending | AgentLifecycleStatus::Running => {},
                AgentLifecycleStatus::Idle => {
                    // Idle 但无 outcome——不计入具体分类
                },
                AgentLifecycleStatus::Terminated => {
                    self.aborted.fetch_add(1, Ordering::Relaxed);
                },
            },
            Some(AgentTurnOutcome::Completed) => {
                self.completed.fetch_add(1, Ordering::Relaxed);
            },
            Some(AgentTurnOutcome::Cancelled) => {
                self.aborted.fetch_add(1, Ordering::Relaxed);
            },
            Some(AgentTurnOutcome::TokenExceeded) => {
                self.token_exceeded.fetch_add(1, Ordering::Relaxed);
            },
            Some(AgentTurnOutcome::Failed) => {
                self.failures.fetch_add(1, Ordering::Relaxed);
            },
        }
    }

    fn snapshot(&self) -> SubRunExecutionMetricsSnapshot {
        SubRunExecutionMetricsSnapshot {
            total: self.total.load(Ordering::Relaxed),
            failures: self.failures.load(Ordering::Relaxed),
            completed: self.completed.load(Ordering::Relaxed),
            aborted: self.aborted.load(Ordering::Relaxed),
            token_exceeded: self.token_exceeded.load(Ordering::Relaxed),
            shared_session_total: self.shared_session_total.load(Ordering::Relaxed),
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
struct ExecutionDiagnosticsMetrics {
    child_spawned: AtomicU64,
    child_started_persisted: AtomicU64,
    child_terminal_persisted: AtomicU64,
    parent_reactivation_requested: AtomicU64,
    parent_reactivation_succeeded: AtomicU64,
    parent_reactivation_failed: AtomicU64,
    lineage_mismatch_parent_agent: AtomicU64,
    lineage_mismatch_parent_session: AtomicU64,
    lineage_mismatch_child_session: AtomicU64,
    lineage_mismatch_descriptor_missing: AtomicU64,
    cache_reuse_hits: AtomicU64,
    cache_reuse_misses: AtomicU64,
    delivery_buffer_queued: AtomicU64,
    delivery_buffer_dequeued: AtomicU64,
    delivery_buffer_wake_requested: AtomicU64,
    delivery_buffer_wake_succeeded: AtomicU64,
    delivery_buffer_wake_failed: AtomicU64,
    legacy_shared_history_rejections: AtomicU64,
}

impl ExecutionDiagnosticsMetrics {
    fn record_child_lifecycle(&self, stage: ChildLifecycleStage) {
        let counter = match stage {
            ChildLifecycleStage::Spawned => &self.child_spawned,
            ChildLifecycleStage::StartedPersisted => &self.child_started_persisted,
            ChildLifecycleStage::TerminalPersisted => &self.child_terminal_persisted,
            ChildLifecycleStage::ReactivationRequested => &self.parent_reactivation_requested,
            ChildLifecycleStage::ReactivationSucceeded => &self.parent_reactivation_succeeded,
            ChildLifecycleStage::ReactivationFailed => &self.parent_reactivation_failed,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    fn record_lineage_mismatch(&self, kind: LineageMismatchKind) {
        let counter = match kind {
            LineageMismatchKind::ParentAgent => &self.lineage_mismatch_parent_agent,
            LineageMismatchKind::ParentSession => &self.lineage_mismatch_parent_session,
            LineageMismatchKind::ChildSession => &self.lineage_mismatch_child_session,
            LineageMismatchKind::DescriptorMissing => &self.lineage_mismatch_descriptor_missing,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    fn record_cache_reuse_hits(&self, count: u64) {
        self.cache_reuse_hits.fetch_add(count, Ordering::Relaxed);
    }

    fn record_cache_reuse_misses(&self, count: u64) {
        self.cache_reuse_misses.fetch_add(count, Ordering::Relaxed);
    }

    fn record_delivery_buffer(&self, stage: DeliveryBufferStage) {
        let counter = match stage {
            DeliveryBufferStage::Queued => &self.delivery_buffer_queued,
            DeliveryBufferStage::Dequeued => &self.delivery_buffer_dequeued,
            DeliveryBufferStage::WakeRequested => &self.delivery_buffer_wake_requested,
            DeliveryBufferStage::WakeSucceeded => &self.delivery_buffer_wake_succeeded,
            DeliveryBufferStage::WakeFailed => &self.delivery_buffer_wake_failed,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    #[allow(dead_code)]
    fn record_legacy_rejection(&self, kind: LegacyRejectionKind) {
        match kind {
            LegacyRejectionKind::SharedHistoryUnsupported => {
                self.legacy_shared_history_rejections
                    .fetch_add(1, Ordering::Relaxed);
            },
        }
    }

    fn snapshot(&self) -> ExecutionDiagnosticsSnapshot {
        ExecutionDiagnosticsSnapshot {
            child_spawned: self.child_spawned.load(Ordering::Relaxed),
            child_started_persisted: self.child_started_persisted.load(Ordering::Relaxed),
            child_terminal_persisted: self.child_terminal_persisted.load(Ordering::Relaxed),
            parent_reactivation_requested: self
                .parent_reactivation_requested
                .load(Ordering::Relaxed),
            parent_reactivation_succeeded: self
                .parent_reactivation_succeeded
                .load(Ordering::Relaxed),
            parent_reactivation_failed: self.parent_reactivation_failed.load(Ordering::Relaxed),
            lineage_mismatch_parent_agent: self
                .lineage_mismatch_parent_agent
                .load(Ordering::Relaxed),
            lineage_mismatch_parent_session: self
                .lineage_mismatch_parent_session
                .load(Ordering::Relaxed),
            lineage_mismatch_child_session: self
                .lineage_mismatch_child_session
                .load(Ordering::Relaxed),
            lineage_mismatch_descriptor_missing: self
                .lineage_mismatch_descriptor_missing
                .load(Ordering::Relaxed),
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
            legacy_shared_history_rejections: self
                .legacy_shared_history_rejections
                .load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subrun_execution_metrics_record_outcomes_and_storage_modes() {
        let metrics = RuntimeObservability::default();
        metrics.record_subrun_execution(
            Duration::from_millis(10),
            &AgentLifecycleStatus::Idle,
            &Some(AgentTurnOutcome::Completed),
            SubRunStorageMode::SharedSession,
            3,
            120,
        );
        metrics.record_subrun_execution(
            Duration::from_millis(20),
            &AgentLifecycleStatus::Idle,
            &Some(AgentTurnOutcome::Failed),
            SubRunStorageMode::IndependentSession,
            5,
            240,
        );

        let snapshot = metrics.snapshot().subrun_execution;
        assert_eq!(snapshot.total, 2);
        assert_eq!(snapshot.completed, 1);
        assert_eq!(snapshot.failures, 1);
        assert_eq!(snapshot.shared_session_total, 1);
        assert_eq!(snapshot.independent_session_total, 1);
        assert_eq!(snapshot.total_steps, 8);
        assert_eq!(snapshot.total_estimated_tokens, 360);
        assert_eq!(snapshot.last_step_count, 5);
        assert_eq!(snapshot.last_estimated_tokens, 240);
    }

    #[test]
    fn execution_diagnostics_snapshot_tracks_structured_counters() {
        let metrics = RuntimeObservability::default();

        metrics.record_child_lifecycle(ChildLifecycleStage::Spawned);
        metrics.record_child_lifecycle(ChildLifecycleStage::StartedPersisted);
        metrics.record_child_lifecycle(ChildLifecycleStage::TerminalPersisted);
        metrics.record_child_lifecycle(ChildLifecycleStage::ReactivationRequested);
        metrics.record_child_lifecycle(ChildLifecycleStage::ReactivationSucceeded);
        metrics.record_child_lifecycle(ChildLifecycleStage::ReactivationFailed);
        metrics.record_lineage_mismatch(LineageMismatchKind::ParentAgent);
        metrics.record_lineage_mismatch(LineageMismatchKind::DescriptorMissing);
        metrics.record_cache_reuse_hits(2);
        metrics.record_cache_reuse_misses(3);
        metrics.record_delivery_buffer(DeliveryBufferStage::Queued);
        metrics.record_delivery_buffer(DeliveryBufferStage::WakeRequested);
        metrics.record_delivery_buffer(DeliveryBufferStage::WakeFailed);
        metrics.record_legacy_rejection(LegacyRejectionKind::SharedHistoryUnsupported);

        let snapshot = metrics.snapshot().execution_diagnostics;
        assert_eq!(snapshot.child_spawned, 1);
        assert_eq!(snapshot.child_started_persisted, 1);
        assert_eq!(snapshot.child_terminal_persisted, 1);
        assert_eq!(snapshot.parent_reactivation_requested, 1);
        assert_eq!(snapshot.parent_reactivation_succeeded, 1);
        assert_eq!(snapshot.parent_reactivation_failed, 1);
        assert_eq!(snapshot.lineage_mismatch_parent_agent, 1);
        assert_eq!(snapshot.lineage_mismatch_descriptor_missing, 1);
        assert_eq!(snapshot.cache_reuse_hits, 2);
        assert_eq!(snapshot.cache_reuse_misses, 3);
        assert_eq!(snapshot.delivery_buffer_queued, 1);
        assert_eq!(snapshot.delivery_buffer_wake_requested, 1);
        assert_eq!(snapshot.delivery_buffer_wake_failed, 1);
        assert_eq!(snapshot.legacy_shared_history_rejections, 1);
    }
}
