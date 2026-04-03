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
}

#[derive(Default)]
pub struct RuntimeObservability {
    session_rehydrate: OperationMetrics,
    sse_catch_up: ReplayMetrics,
    turn_execution: OperationMetrics,
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

    pub fn snapshot(&self) -> RuntimeObservabilitySnapshot {
        RuntimeObservabilitySnapshot {
            session_rehydrate: self.session_rehydrate.snapshot(),
            sse_catch_up: self.sse_catch_up.snapshot(),
            turn_execution: self.turn_execution.snapshot(),
        }
    }
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
