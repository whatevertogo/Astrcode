//! Turn 执行核心类型与实现。
//!
//! Turn 是单个 Agent 交互循环的完整生命周期（用户消息 -> LLM 响应 -> 工具调用 -> ... -> Turn
//! 结束）。

mod compaction_cycle;
pub mod llm_cycle;
mod runner;
// pub mod subagent;
pub mod token_budget;
pub mod tool_cycle;

use astrcode_core::{SessionId, TurnId};

/// Turn 运行请求参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnRunRequest {
    pub session_id: SessionId,
    pub turn_id: TurnId,
}

/// Turn 结束原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnOutcome {
    /// LLM 返回纯文本（无 tool_calls），自然结束。
    Completed,
    /// 用户取消或 CancelToken 触发。
    Cancelled,
    /// 不可恢复错误。
    Error { message: String },
}

pub use runner::{TurnRunRequest as RunnerRequest, TurnRunResult, run_turn};
