use std::sync::Arc;

use crate::service::RuntimeService;

mod metrics;

pub(crate) use metrics::RuntimeObservability;
pub use metrics::{
    ExecutionDiagnosticsSnapshot, OperationMetricsSnapshot, ReplayMetricsSnapshot, ReplayPath,
    RuntimeObservabilitySnapshot, SubRunExecutionMetricsSnapshot,
};

/// `runtime-observability` 的唯一 surface handle。
#[derive(Clone)]
pub struct ObservabilityServiceHandle {
    runtime: Arc<RuntimeService>,
}

impl ObservabilityServiceHandle {
    pub(super) fn new(runtime: Arc<RuntimeService>) -> Self {
        Self { runtime }
    }

    pub fn snapshot(&self) -> RuntimeObservabilitySnapshot {
        self.runtime.observability.snapshot()
    }
}
