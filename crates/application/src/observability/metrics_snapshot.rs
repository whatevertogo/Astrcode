//! # 可观测性指标快照类型
//!
//! 纯数据 DTO，从 `runtime::service::observability::metrics` 迁出的快照定义。
//! 这些类型不含任何 trait object 或运行时依赖，仅用于跨层传递指标数据。
//! 实际的指标收集器仍在旧 runtime 中，Phase 10 组合根接线时桥接。

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

/// 子执行域共享观测指标快照。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubRunExecutionMetricsSnapshot {
    pub total: u64,
    pub failures: u64,
    pub completed: u64,
    pub cancelled: u64,
    pub token_exceeded: u64,
    pub independent_session_total: u64,
    pub total_duration_ms: u64,
    pub last_duration_ms: u64,
    pub total_steps: u64,
    pub last_step_count: u64,
    pub total_estimated_tokens: u64,
    pub last_estimated_tokens: u64,
}

/// 子会话与缓存切换相关的结构化观测指标快照。
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
}

/// Agent collaboration 评估读模型。
///
/// 这些字段全部由 raw collaboration facts 派生，
/// 用于判断 agent-tool 是否真的创造了协作价值。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentCollaborationScorecardSnapshot {
    pub total_facts: u64,
    pub spawn_accepted: u64,
    pub spawn_rejected: u64,
    pub send_reused: u64,
    pub send_queued: u64,
    pub send_rejected: u64,
    pub observe_calls: u64,
    pub observe_rejected: u64,
    pub observe_followed_by_action: u64,
    pub close_calls: u64,
    pub close_rejected: u64,
    pub delivery_delivered: u64,
    pub delivery_consumed: u64,
    pub delivery_replayed: u64,
    pub orphan_child_count: u64,
    pub child_reuse_ratio_bps: Option<u64>,
    pub observe_to_action_ratio_bps: Option<u64>,
    pub spawn_to_delivery_ratio_bps: Option<u64>,
    pub orphan_child_ratio_bps: Option<u64>,
    pub avg_delivery_latency_ms: Option<u64>,
    pub max_delivery_latency_ms: Option<u64>,
}

/// 运行时可观测性快照，包含各类操作的指标。
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
    /// 子会话与缓存切换相关的结构化观测指标
    pub execution_diagnostics: ExecutionDiagnosticsSnapshot,
    /// agent-tool 协作效果评估读模型
    pub agent_collaboration: AgentCollaborationScorecardSnapshot,
}
