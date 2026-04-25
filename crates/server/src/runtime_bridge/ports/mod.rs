//! # 应用层端口（Port）
//!
//! 定义 application 层与外部系统交互的 trait 契约，实现依赖反转：
//! - `AppKernelPort`：`App` 依赖的 kernel 控制面
//! - `AgentKernelPort`：Agent 编排子域扩展的 kernel 端口
//! - `AppSessionPort`：`App` 依赖的 session-runtime 稳定端口
//! - `AgentSessionPort`：Agent 编排子域扩展的 session 端口

mod agent_kernel;
mod agent_session;
mod app_kernel;
mod app_session;
pub(crate) mod kernel_bridge;
pub(crate) mod session_bridge;
mod session_contracts;
mod session_submission;

pub use agent_kernel::AgentKernelPort;
pub use agent_session::AgentSessionPort;
pub use app_kernel::{AppKernelPort, ServerKernelControlError};
pub use app_session::AppSessionPort;
#[cfg(test)]
pub(crate) use session_bridge::recoverable_parent_deliveries;
pub use session_contracts::{
    DurableSubRunStatusSummary, RecoverableParentDelivery, SessionObserveSnapshot,
    SessionTurnOutcomeSummary, SessionTurnTerminalState,
};
pub use session_submission::AppAgentPromptSubmission;
