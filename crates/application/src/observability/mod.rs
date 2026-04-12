//! # 可观测性
//!
//! 提供运行时指标快照类型和治理快照能力。
//! 实际的指标收集逻辑留在旧 runtime，Phase 10 组合根接线时桥接。

mod metrics_snapshot;

use std::path::PathBuf;

use astrcode_core::{CapabilitySpec, plugin::PluginEntry};
pub use metrics_snapshot::{
    ExecutionDiagnosticsSnapshot, OperationMetricsSnapshot, ReplayMetricsSnapshot, ReplayPath,
    RuntimeObservabilitySnapshot, SubRunExecutionMetricsSnapshot,
};

/// 运行时治理快照，替代旧 `RuntimeGovernanceSnapshot`。
///
/// 不依赖 `RuntimeService`，数据来源于运行时治理端口、`SessionRuntime`
/// 和可观测性指标提供者。
#[derive(Debug, Clone)]
pub struct GovernanceSnapshot {
    pub runtime_name: String,
    pub runtime_kind: String,
    pub loaded_session_count: usize,
    pub running_session_ids: Vec<String>,
    pub plugin_search_paths: Vec<PathBuf>,
    pub metrics: RuntimeObservabilitySnapshot,
    pub capabilities: Vec<CapabilitySpec>,
    pub plugins: Vec<PluginEntry>,
}

/// 运行时重载操作的结果。
#[derive(Debug, Clone)]
pub struct ReloadResult {
    /// 重载后的运行时快照
    pub snapshot: GovernanceSnapshot,
    /// 重载完成的时间
    pub reloaded_at: chrono::DateTime<chrono::Utc>,
}
