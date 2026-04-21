//! Turn 用例与执行核心。
//!
//! `session-runtime::turn` 只承接“单次 turn 如何开始、如何中断、如何分支、如何执行”。
//! `runtime` 拥有运行时控制状态，`watcher` 拥有等待终态的异步监听循环，
//! `runner` 负责 step 循环，`submit/interrupt/branch` 负责对外 façade。

mod branch;
mod compact_events;
mod compaction_cycle;
mod continuation_cycle;
mod events;
mod finalize;
mod fork;
mod interrupt;
mod journal;
pub(crate) mod llm_cycle;
mod loop_control;
pub(crate) mod manual_compact;
mod post_llm_policy;
pub(crate) mod projector;
mod request;
mod runner;
mod runtime;
mod submit;
mod subrun_events;
#[cfg(test)]
pub(crate) mod test_support;
// pub mod subagent;
mod summary;
pub(crate) mod tool_cycle;
mod tool_result_budget;
mod watcher;

pub use fork::{ForkPoint, ForkResult};
pub use loop_control::{TurnLoopTransition, TurnStopCause};
pub(crate) use runtime::{PendingManualCompactRequest, TurnRuntimeState};
pub use submit::AgentPromptSubmission;
pub use summary::{TurnCollaborationSummary, TurnFinishReason, TurnSummary};
pub(crate) use watcher::{wait_and_project_turn_outcome, wait_for_turn_terminal_snapshot};

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
