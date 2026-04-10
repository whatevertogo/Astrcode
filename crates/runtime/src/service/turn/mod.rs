//! Turn façade 的内部实现按用例拆分：
//! - `submit`：提交 prompt 与启动异步 turn
//! - `interrupt`：中断正在执行的 turn
//! - `replay`：历史回放（从缓存或磁盘加载事件）
//! - `branch`：忙会话分支与稳定历史选择
//! - `TODO`: 修复branch暂不生效的功能
//! - `orchestration`：turn 执行链与 auto-continue

mod branch;
mod interrupt;
mod orchestration;
mod replay;
mod submit;

#[derive(Debug, Clone, Copy)]
pub(super) struct BudgetSettings {
    pub continuation_min_delta_tokens: usize,
    pub max_continuations: u8,
}

pub(super) use orchestration::{RuntimeTurnInput, complete_session_execution, run_session_turn};

#[cfg(test)]
mod tests;
