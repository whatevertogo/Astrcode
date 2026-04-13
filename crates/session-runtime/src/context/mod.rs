//! 子 Agent 上下文快照。
//!
//! 从 `runtime-execution/context.rs` 迁入。
//! 描述子 Agent 执行时可继承的父上下文。
//!
//! 边界约束：
//! - 这里只表达“有哪些上下文来源、继承了哪些结果”
//! - 不负责 token 裁剪
//! - 不负责最终 request / prompt 组装

use serde::{Deserialize, Serialize};

/// 子 Agent 执行前的结构化上下文快照。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedContextSnapshot {
    /// 任务主体（上层已经解析好的 task payload）。
    #[serde(default)]
    pub task_payload: String,
    /// 从父会话继承的 compact summary。
    #[serde(default)]
    pub inherited_compact_summary: Option<String>,
    /// 从父会话继承的最近 N 轮对话 tail。
    #[serde(default)]
    pub inherited_recent_tail: Vec<String>,
}
