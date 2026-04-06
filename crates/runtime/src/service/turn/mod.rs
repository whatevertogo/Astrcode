//! Turn façade 的内部实现按用例拆分：
//! - `submit`：提交 prompt 与启动异步 turn
//! - `branch`：忙会话分支与稳定历史选择
//! - `compact`：手动 compact 与 durable tail 选择

mod branch;
mod compact;
mod submit;

pub(super) type BudgetSettings = astrcode_runtime_session::BudgetSettings;

#[cfg(test)]
mod tests;
