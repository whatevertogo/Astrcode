//! Agent 编排子域依赖的 kernel 稳定端口。
//!
//! `AgentKernelPort` 继承 `AppKernelPort`，扩展了 agent 编排所需的全部 kernel 操作：
//! lifecycle 管理、子 agent spawn/resume/terminate、inbox 投递、parent delivery 队列。
//!
//! 为什么单独抽 trait：`AgentOrchestrationService` 需要的控制面明显大于 `App`，
//! 避免 `AppKernelPort` 被动膨胀成新的大而全 façade。
//!
//! server-owned bridge 是正式实现入口，不把底层 session runtime 暴露成 kernel owner。

use astrcode_core::{
    AgentInboxEnvelope, AgentLifecycleStatus, AgentTurnOutcome, ChildSessionNotification,
    DelegationMetadata,
};
use astrcode_host_session::SubRunHandle;
use async_trait::async_trait;

use super::{AppKernelPort, RecoverableParentDelivery, ServerKernelControlError};

/// Agent 编排子域依赖的 kernel 稳定端口。
///
/// Why: `AgentOrchestrationService` 需要的控制面明显大于 `App`，
/// 单独抽 trait 能避免 `AppKernelPort` 被动膨胀成新的大而全 façade。
#[async_trait]
pub trait AgentKernelPort: AppKernelPort {
    async fn get_lifecycle(&self, sub_run_or_agent_id: &str) -> Option<AgentLifecycleStatus>;
    async fn get_turn_outcome(&self, sub_run_or_agent_id: &str) -> Option<AgentTurnOutcome>;
    async fn resume(&self, sub_run_or_agent_id: &str, parent_turn_id: &str)
    -> Option<SubRunHandle>;
    async fn spawn_independent_child(
        &self,
        profile: &astrcode_core::AgentProfile,
        session_id: String,
        child_session_id: String,
        parent_turn_id: String,
        parent_agent_id: String,
    ) -> Result<SubRunHandle, ServerKernelControlError>;
    async fn set_lifecycle(
        &self,
        sub_run_or_agent_id: &str,
        new_status: AgentLifecycleStatus,
    ) -> Option<()>;
    async fn complete_turn(
        &self,
        sub_run_or_agent_id: &str,
        outcome: AgentTurnOutcome,
    ) -> Option<AgentLifecycleStatus>;
    async fn set_delegation(
        &self,
        sub_run_or_agent_id: &str,
        delegation: Option<DelegationMetadata>,
    ) -> Option<()>;
    async fn count_children_spawned_for_turn(
        &self,
        parent_agent_id: &str,
        parent_turn_id: &str,
    ) -> usize;
    async fn collect_subtree_handles(&self, sub_run_or_agent_id: &str) -> Vec<SubRunHandle>;
    async fn terminate_subtree(&self, sub_run_or_agent_id: &str) -> Option<SubRunHandle>;
    async fn deliver(&self, agent_id: &str, envelope: AgentInboxEnvelope) -> Option<()>;
    async fn drain_inbox(&self, agent_id: &str) -> Option<Vec<AgentInboxEnvelope>>;
    async fn enqueue_child_delivery(
        &self,
        parent_session_id: String,
        parent_turn_id: String,
        notification: ChildSessionNotification,
    ) -> bool;
    async fn checkout_parent_delivery_batch(
        &self,
        parent_session_id: &str,
    ) -> Option<Vec<RecoverableParentDelivery>>;
    async fn pending_parent_delivery_count(&self, parent_session_id: &str) -> usize;
    async fn requeue_parent_delivery_batch(&self, parent_session_id: &str, delivery_ids: &[String]);
    async fn consume_parent_delivery_batch(
        &self,
        parent_session_id: &str,
        delivery_ids: &[String],
    ) -> bool;
}
