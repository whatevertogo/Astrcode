//! Turn 用例与执行核心。
//!
//! `session-runtime::turn` 只承接“单次 turn 如何开始、如何中断、如何回放、如何分支、如何执行”。
//! `runner` 负责 step 循环，`submit/replay/interrupt/branch` 负责对外 façade。

mod branch;
mod compaction_cycle;
mod continuation_cycle;
mod events;
mod fork;
mod interrupt;
mod journal;
pub(crate) mod llm_cycle;
mod loop_control;
pub(crate) mod manual_compact;
mod post_llm_policy;
mod replay;
mod request;
mod runner;
mod submit;
#[cfg(test)]
pub(crate) mod test_support;
// pub mod subagent;
mod summary;
pub(crate) mod tool_cycle;
mod tool_result_budget;

pub use fork::{ForkPoint, ForkResult};
pub use loop_control::{TurnLoopTransition, TurnStopCause};
pub use submit::AgentPromptSubmission;
pub use summary::{TurnCollaborationSummary, TurnFinishReason, TurnSummary};

/// Turn 结束原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TurnOutcome {
    /// LLM 返回纯文本（无 tool_calls），自然结束。
    Completed,
    /// 用户取消或 CancelToken 触发。
    Cancelled,
    /// 不可恢复错误。
    Error { message: String },
}

impl TurnOutcome {
    pub(crate) fn terminal_kind(
        &self,
        stop_cause: TurnStopCause,
    ) -> astrcode_core::TurnTerminalKind {
        match self {
            Self::Completed => stop_cause.terminal_kind(None),
            Self::Cancelled => astrcode_core::TurnTerminalKind::Cancelled,
            Self::Error { message } => stop_cause.terminal_kind(Some(message)),
        }
    }
}

pub(crate) use runner::{TurnRunRequest as RunnerRequest, TurnRunResult, run_turn};
