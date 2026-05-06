//! server-owned runtime bootstrap bridge。
//!
//! 把 builtin mode seed、任务注册表、可观测性采集器收敛到 server 本地模块，
//! 避免组合根直接依赖 application runtime 类型。

use std::sync::Arc;

use astrcode_core::{
    AgentCollaborationFact, AgentTurnOutcome, Result, RuntimeMetricsRecorder,
    RuntimeObservabilitySnapshot, SubRunStorageMode, mode::GovernanceModeSpec,
};

use crate::{
    ObservabilitySnapshotProvider, RuntimeObservabilityCollector, TaskRegistry,
    mode::builtin_mode_specs,
};

#[derive(Debug, Clone)]
pub(crate) struct ServerTaskRegistry {
    inner: Arc<TaskRegistry>,
}

impl ServerTaskRegistry {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Arc::new(TaskRegistry::new()),
        })
    }

    pub(crate) fn inner(&self) -> Arc<TaskRegistry> {
        Arc::clone(&self.inner)
    }
}

#[derive(Clone, Default)]
pub(crate) struct ServerRuntimeObservability {
    inner: Arc<RuntimeObservabilityCollector>,
}

impl ServerRuntimeObservability {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Arc::new(RuntimeObservabilityCollector::new()),
        })
    }

    pub(crate) fn snapshot(&self) -> RuntimeObservabilitySnapshot {
        self.inner.snapshot()
    }
}

impl RuntimeMetricsRecorder for ServerRuntimeObservability {
    fn record_session_rehydrate(&self, duration_ms: u64, success: bool) {
        self.inner.record_session_rehydrate(duration_ms, success);
    }

    fn record_sse_catch_up(
        &self,
        duration_ms: u64,
        success: bool,
        used_disk_fallback: bool,
        recovered_events: u64,
    ) {
        self.inner
            .record_sse_catch_up(duration_ms, success, used_disk_fallback, recovered_events);
    }

    fn record_turn_execution(&self, duration_ms: u64, success: bool) {
        self.inner.record_turn_execution(duration_ms, success);
    }

    fn record_subrun_execution(
        &self,
        duration_ms: u64,
        outcome: AgentTurnOutcome,
        step_count: Option<u32>,
        estimated_tokens: Option<u64>,
        storage_mode: Option<SubRunStorageMode>,
    ) {
        self.inner.record_subrun_execution(
            duration_ms,
            outcome,
            step_count,
            estimated_tokens,
            storage_mode,
        );
    }

    fn record_child_spawned(&self) {
        self.inner.record_child_spawned();
    }

    fn record_parent_reactivation_requested(&self) {
        self.inner.record_parent_reactivation_requested();
    }

    fn record_parent_reactivation_succeeded(&self) {
        self.inner.record_parent_reactivation_succeeded();
    }

    fn record_parent_reactivation_failed(&self) {
        self.inner.record_parent_reactivation_failed();
    }

    fn record_delivery_buffer_queued(&self) {
        self.inner.record_delivery_buffer_queued();
    }

    fn record_delivery_buffer_dequeued(&self) {
        self.inner.record_delivery_buffer_dequeued();
    }

    fn record_delivery_buffer_wake_requested(&self) {
        self.inner.record_delivery_buffer_wake_requested();
    }

    fn record_delivery_buffer_wake_succeeded(&self) {
        self.inner.record_delivery_buffer_wake_succeeded();
    }

    fn record_delivery_buffer_wake_failed(&self) {
        self.inner.record_delivery_buffer_wake_failed();
    }

    fn record_cache_reuse_hit(&self) {
        self.inner.record_cache_reuse_hit();
    }

    fn record_cache_reuse_miss(&self) {
        self.inner.record_cache_reuse_miss();
    }

    fn record_agent_collaboration_fact(&self, fact: &AgentCollaborationFact) {
        self.inner.record_agent_collaboration_fact(fact);
    }
}

impl ObservabilitySnapshotProvider for ServerRuntimeObservability {
    fn snapshot(&self) -> RuntimeObservabilitySnapshot {
        self.snapshot()
    }
}

impl std::fmt::Debug for ServerRuntimeObservability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerRuntimeObservability")
            .finish_non_exhaustive()
    }
}

pub(crate) fn builtin_server_mode_specs() -> Result<Vec<GovernanceModeSpec>> {
    Ok(builtin_mode_specs())
}
