use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayPath {
    Cache,
    DiskFallback,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OperationMetricsSnapshot {
    pub total: u64,
    pub failures: u64,
    pub total_duration_ms: u64,
    pub last_duration_ms: u64,
    pub max_duration_ms: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReplayMetricsSnapshot {
    pub totals: OperationMetricsSnapshot,
    pub cache_hits: u64,
    pub disk_fallbacks: u64,
    pub recovered_events: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeObservabilitySnapshot {
    pub session_rehydrate: OperationMetricsSnapshot,
    pub sse_catch_up: ReplayMetricsSnapshot,
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

#[derive(Default)]
struct OperationMetrics {
    total: AtomicU64,
    failures: AtomicU64,
    total_duration_ms: AtomicU64,
    last_duration_ms: AtomicU64,
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

        let _ = self.max_duration_ms.fetch_update(
            Ordering::Relaxed,
            Ordering::Relaxed,
            |current| (elapsed_ms > current).then_some(elapsed_ms),
        );
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
    fn record(&self, duration: Duration, ok: bool, path: ReplayPath, recovered_events: u64) {
        self.totals.record(duration, ok);
        match path {
            ReplayPath::Cache => {
                self.cache_hits.fetch_add(1, Ordering::Relaxed);
            }
            ReplayPath::DiskFallback => {
                self.disk_fallbacks.fetch_add(1, Ordering::Relaxed);
            }
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

