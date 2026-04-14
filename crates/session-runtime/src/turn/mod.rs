//! Turn 用例与执行核心。
//!
//! `session-runtime::turn` 只承接“单次 turn 如何开始、如何中断、如何回放、如何分支、如何执行”。
//! `runner` 负责 step 循环，`submit/replay/interrupt/branch` 负责对外 façade。

mod branch;
mod compaction_cycle;
mod events;
mod interrupt;
pub mod llm_cycle;
pub(crate) mod manual_compact;
mod replay;
mod request;
mod runner;
mod submit;
#[cfg(test)]
pub(crate) mod test_support;
// pub mod subagent;
pub mod summary;
pub mod tool_cycle;

use astrcode_core::{SessionId, TurnId};
pub use summary::{TurnCollaborationSummary, TurnFinishReason, TurnSummary};

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
