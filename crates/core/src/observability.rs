use crate::{AgentCollaborationFact, AgentTurnOutcome, SubRunStorageMode};

/// 统一的运行时观测记录接口。
///
/// 只暴露窄写入方法，避免业务层反向依赖具体快照实现。
pub trait RuntimeMetricsRecorder: Send + Sync {
    fn record_session_rehydrate(&self, duration_ms: u64, success: bool);

    fn record_sse_catch_up(
        &self,
        duration_ms: u64,
        success: bool,
        used_disk_fallback: bool,
        recovered_events: u64,
    );

    fn record_turn_execution(&self, duration_ms: u64, success: bool);

    fn record_subrun_execution(
        &self,
        duration_ms: u64,
        outcome: AgentTurnOutcome,
        step_count: Option<u32>,
        estimated_tokens: Option<u64>,
        storage_mode: Option<SubRunStorageMode>,
    );

    fn record_child_spawned(&self);
    fn record_parent_reactivation_requested(&self);
    fn record_parent_reactivation_succeeded(&self);
    fn record_parent_reactivation_failed(&self);
    fn record_delivery_buffer_queued(&self);
    fn record_delivery_buffer_dequeued(&self);
    fn record_delivery_buffer_wake_requested(&self);
    fn record_delivery_buffer_wake_succeeded(&self);
    fn record_delivery_buffer_wake_failed(&self);
    fn record_cache_reuse_hit(&self);
    fn record_cache_reuse_miss(&self);
    fn record_agent_collaboration_fact(&self, fact: &AgentCollaborationFact);
}
