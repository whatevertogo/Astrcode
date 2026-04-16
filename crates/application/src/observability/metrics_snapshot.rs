//! # 可观测性指标快照类型
//!
//! owner 已下沉到 `astrcode_core::observability`。
//! application 这里只保留 re-export，避免继续维护第二套共享语义定义。

pub use astrcode_core::{
    AgentCollaborationScorecardSnapshot, ExecutionDiagnosticsSnapshot, OperationMetricsSnapshot,
    ReplayMetricsSnapshot, ReplayPath, RuntimeObservabilitySnapshot,
    SubRunExecutionMetricsSnapshot,
};
