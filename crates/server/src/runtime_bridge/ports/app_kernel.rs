//! `App` 依赖的 kernel 稳定端口。
//!
//! 定义 `AppKernelPort` trait，将应用层与 kernel 具体实现解耦。
//! `App` 只需要一组稳定的 agent 控制与 capability 查询契约。
//!
//! server-owned bridge 是正式实现入口，避免把底层 session runtime 当成 owner surface 暴露。

use std::fmt;

use astrcode_host_session::SubRunHandle;
use async_trait::async_trait;

/// server-owned 的最小 agent control 错误模型。
///
/// Why: owner bridge 只向执行面暴露 server 真正需要解释的约束语义。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerKernelControlError {
    MaxDepthExceeded { current: usize, max: usize },
    MaxConcurrentExceeded { current: usize, max: usize },
    ParentAgentNotFound { agent_id: String },
}

impl fmt::Display for ServerKernelControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MaxDepthExceeded { current, max } => {
                write!(f, "max depth exceeded ({current}/{max})")
            },
            Self::MaxConcurrentExceeded { current, max } => {
                write!(f, "max concurrent agents exceeded ({current}/{max})")
            },
            Self::ParentAgentNotFound { agent_id } => {
                write!(f, "parent agent '{agent_id}' not found")
            },
        }
    }
}

/// `App` 依赖的 kernel 稳定端口。
///
/// Why: `App` 是应用层用例入口，不应直接绑定 `Kernel` 具体实现；
/// 它只需要一组稳定的 agent 控制与 capability 查询契约。
#[async_trait]
pub trait AppKernelPort: Send + Sync {
    async fn get_handle(&self, agent_id: &str) -> Option<SubRunHandle>;
    async fn find_root_handle_for_session(&self, session_id: &str) -> Option<SubRunHandle>;
    async fn register_root_agent(
        &self,
        agent_id: String,
        session_id: String,
        profile_id: String,
    ) -> Result<SubRunHandle, ServerKernelControlError>;
    async fn set_resolved_limits(
        &self,
        sub_run_or_agent_id: &str,
        resolved_limits: astrcode_core::ResolvedExecutionLimitsSnapshot,
    ) -> Option<()>;
}
