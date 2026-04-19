//! `App` 依赖的 kernel 稳定端口。
//!
//! 定义 `AppKernelPort` trait，将应用层与 kernel 具体实现解耦。
//! `App` 只需要一组稳定的 agent 控制与 capability 查询契约，
//! 不直接绑定 `Kernel` 的内部结构。
//!
//! 同时提供 `Kernel` 对 `AppKernelPort` 的 blanket impl，
//! 组合根在 `bootstrap_server_runtime()` 中只需一次 `Arc<Kernel>` 即可满足两个端口的约束。

use astrcode_core::SubRunHandle;
use astrcode_kernel::{
    AgentControlError, CloseSubtreeResult, Kernel, KernelGateway, SubRunStatusView,
};
use async_trait::async_trait;

/// `App` 依赖的 kernel 稳定端口。
///
/// Why: `App` 是应用层用例入口，不应直接绑定 `Kernel` 具体实现；
/// 它只需要一组稳定的 agent 控制与 capability 查询契约。
#[async_trait]
pub trait AppKernelPort: Send + Sync {
    fn gateway(&self) -> KernelGateway;

    async fn query_subrun_status(&self, agent_id: &str) -> Option<SubRunStatusView>;
    async fn query_root_status(&self, session_id: &str) -> Option<SubRunStatusView>;
    async fn list_statuses(&self) -> Vec<SubRunStatusView>;
    async fn get_handle(&self, agent_id: &str) -> Option<SubRunHandle>;
    async fn find_root_handle_for_session(&self, session_id: &str) -> Option<SubRunHandle>;
    async fn register_root_agent(
        &self,
        agent_id: String,
        session_id: String,
        profile_id: String,
    ) -> Result<SubRunHandle, AgentControlError>;
    async fn set_resolved_limits(
        &self,
        sub_run_or_agent_id: &str,
        resolved_limits: astrcode_core::ResolvedExecutionLimitsSnapshot,
    ) -> Option<()>;
    async fn close_subtree(&self, agent_id: &str) -> Result<CloseSubtreeResult, AgentControlError>;
}

#[async_trait]
impl AppKernelPort for Kernel {
    fn gateway(&self) -> KernelGateway {
        self.gateway().clone()
    }

    async fn query_subrun_status(&self, agent_id: &str) -> Option<SubRunStatusView> {
        self.agent().query_subrun_status(agent_id).await
    }

    async fn query_root_status(&self, session_id: &str) -> Option<SubRunStatusView> {
        self.agent().query_root_status(session_id).await
    }

    async fn list_statuses(&self) -> Vec<SubRunStatusView> {
        self.agent().list_statuses().await
    }

    async fn get_handle(&self, agent_id: &str) -> Option<SubRunHandle> {
        self.agent().get_handle(agent_id).await
    }

    async fn find_root_handle_for_session(&self, session_id: &str) -> Option<SubRunHandle> {
        self.agent().find_root_handle_for_session(session_id).await
    }

    async fn register_root_agent(
        &self,
        agent_id: String,
        session_id: String,
        profile_id: String,
    ) -> Result<SubRunHandle, AgentControlError> {
        self.agent()
            .register_root_agent(agent_id, session_id, profile_id)
            .await
    }

    async fn set_resolved_limits(
        &self,
        sub_run_or_agent_id: &str,
        resolved_limits: astrcode_core::ResolvedExecutionLimitsSnapshot,
    ) -> Option<()> {
        self.agent()
            .set_resolved_limits(sub_run_or_agent_id, resolved_limits)
            .await
    }

    async fn close_subtree(&self, agent_id: &str) -> Result<CloseSubtreeResult, AgentControlError> {
        self.agent().close_subtree(agent_id).await
    }
}
