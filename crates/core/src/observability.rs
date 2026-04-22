//! # 运行时可观测性
//!
//! 定义运行时指标快照和记录接口，用于监控运行时健康状况和性能。
//!
//! ## 快照类型
//!
//! - `OperationMetricsSnapshot`: 单一操作的计数/耗时/失败率
//! - `ReplayMetricsSnapshot`: SSE 回放操作的缓存命中/磁盘回退
//! - `SubRunExecutionMetricsSnapshot`: 子执行域的完成/取消/token 超限统计
//! - `ExecutionDiagnosticsSnapshot`: 子会话生命周期和缓存切换的结构化诊断
//! - `AgentCollaborationScorecardSnapshot`: agent-tool 协作效果的评估读模型
//! - `RuntimeObservabilitySnapshot`: 聚合所有指标的顶层快照
//!
//! `RuntimeMetricsRecorder` 是窄写入接口，业务层只通过它记录事实，
//! 不反向依赖具体快照实现。

use crate::{AgentCollaborationFact, AgentTurnOutcome, SubRunStorageMode};

/// 回放路径：优先缓存，不足时回退到磁盘。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReplayPath {
    /// 从内存缓存读取（快速路径）
    Cache,
    /// 从磁盘 JSONL 文件加载（慢速回退路径）
    DiskFallback,
}

/// 单一操作的指标快照。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
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
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
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
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubRunExecutionMetricsSnapshot {
    pub total: u64,
    pub failures: u64,
    pub completed: u64,
    pub cancelled: u64,
    pub independent_session_total: u64,
    pub total_duration_ms: u64,
    pub last_duration_ms: u64,
    pub total_steps: u64,
    pub last_step_count: u64,
    pub total_estimated_tokens: u64,
    pub last_estimated_tokens: u64,
}

/// 子会话与缓存切换相关的结构化观测指标快照。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
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
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
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
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
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

#[cfg(test)]
mod tests {
    use super::{
        AgentCollaborationScorecardSnapshot, ExecutionDiagnosticsSnapshot,
        OperationMetricsSnapshot, ReplayMetricsSnapshot, RuntimeObservabilitySnapshot,
        SubRunExecutionMetricsSnapshot,
    };

    #[test]
    fn operation_metrics_snapshot_uses_camel_case_wire_shape() {
        let value = serde_json::to_value(OperationMetricsSnapshot {
            total: 1,
            failures: 2,
            total_duration_ms: 3,
            last_duration_ms: 4,
            max_duration_ms: 5,
        })
        .expect("operation metrics should serialize");

        assert_eq!(
            value,
            serde_json::json!({
                "total": 1,
                "failures": 2,
                "totalDurationMs": 3,
                "lastDurationMs": 4,
                "maxDurationMs": 5,
            })
        );
    }

    #[test]
    fn runtime_observability_snapshot_round_trips_with_nested_metrics() {
        let snapshot = RuntimeObservabilitySnapshot {
            session_rehydrate: OperationMetricsSnapshot {
                total: 1,
                failures: 0,
                total_duration_ms: 2,
                last_duration_ms: 3,
                max_duration_ms: 4,
            },
            sse_catch_up: ReplayMetricsSnapshot {
                totals: OperationMetricsSnapshot {
                    total: 5,
                    failures: 1,
                    total_duration_ms: 6,
                    last_duration_ms: 7,
                    max_duration_ms: 8,
                },
                cache_hits: 9,
                disk_fallbacks: 10,
                recovered_events: 11,
            },
            turn_execution: OperationMetricsSnapshot {
                total: 12,
                failures: 2,
                total_duration_ms: 13,
                last_duration_ms: 14,
                max_duration_ms: 15,
            },
            subrun_execution: SubRunExecutionMetricsSnapshot {
                total: 16,
                failures: 3,
                completed: 17,
                cancelled: 18,
                independent_session_total: 20,
                total_duration_ms: 21,
                last_duration_ms: 22,
                total_steps: 23,
                last_step_count: 24,
                total_estimated_tokens: 25,
                last_estimated_tokens: 26,
            },
            execution_diagnostics: ExecutionDiagnosticsSnapshot {
                child_spawned: 27,
                child_started_persisted: 28,
                child_terminal_persisted: 29,
                parent_reactivation_requested: 30,
                parent_reactivation_succeeded: 31,
                parent_reactivation_failed: 32,
                lineage_mismatch_parent_agent: 33,
                lineage_mismatch_parent_session: 34,
                lineage_mismatch_child_session: 35,
                lineage_mismatch_descriptor_missing: 36,
                cache_reuse_hits: 37,
                cache_reuse_misses: 38,
                delivery_buffer_queued: 39,
                delivery_buffer_dequeued: 40,
                delivery_buffer_wake_requested: 41,
                delivery_buffer_wake_succeeded: 42,
                delivery_buffer_wake_failed: 43,
            },
            agent_collaboration: AgentCollaborationScorecardSnapshot {
                total_facts: 44,
                spawn_accepted: 45,
                spawn_rejected: 46,
                send_reused: 47,
                send_queued: 48,
                send_rejected: 49,
                observe_calls: 50,
                observe_rejected: 51,
                observe_followed_by_action: 52,
                close_calls: 53,
                close_rejected: 54,
                delivery_delivered: 55,
                delivery_consumed: 56,
                delivery_replayed: 57,
                orphan_child_count: 58,
                child_reuse_ratio_bps: Some(59),
                observe_to_action_ratio_bps: Some(60),
                spawn_to_delivery_ratio_bps: Some(61),
                orphan_child_ratio_bps: Some(62),
                avg_delivery_latency_ms: Some(63),
                max_delivery_latency_ms: Some(64),
            },
        };

        let json = serde_json::to_string(&snapshot).expect("snapshot should serialize");
        let decoded: RuntimeObservabilitySnapshot =
            serde_json::from_str(&json).expect("snapshot should deserialize");

        assert_eq!(decoded, snapshot);
    }
}
