//! 子 Agent 上下文快照。
//!
//! 从 `runtime-execution/context.rs` 迁入。
//! 描述子 Agent 执行时可继承的父上下文。

use serde::{Deserialize, Serialize};

/// 子 Agent 执行前的完整上下文快照。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedContextSnapshot {
    /// 任务主体（prompt + context 合并后的文本）。
    #[serde(default)]
    pub task_payload: String,
    /// 从父会话继承的 compact summary。
    #[serde(default)]
    pub inherited_compact_summary: Option<String>,
    /// 从父会话继承的最近 N 轮对话 tail。
    #[serde(default)]
    pub inherited_recent_tail: Vec<String>,
}
