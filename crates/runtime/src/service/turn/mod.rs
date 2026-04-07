//! Turn façade 的内部实现按用例拆分：
//! - `submit`：提交 prompt 与启动异步 turn
//! - `branch`：忙会话分支与稳定历史选择
//! - `compact`：手动 compact 与 durable tail 选择

mod branch;
mod compact;
mod orchestration;

#[derive(Debug, Clone, Copy)]
pub(super) struct BudgetSettings {
    pub continuation_min_delta_tokens: usize,
    pub max_continuations: u8,
}

pub(super) use orchestration::{complete_session_execution, run_session_turn};

#[cfg(test)]
mod tests;
