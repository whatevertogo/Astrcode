//! Agent 编排子域依赖的 kernel 稳定端口。
//!
//! `AgentKernelPort` 继承 `AppKernelPort`，扩展了 agent 编排所需的全部 kernel 操作：
//! lifecycle 管理、子 agent spawn/resume/terminate、inbox 投递、parent delivery 队列。
//!
//! 为什么单独抽 trait：`AgentOrchestrationService` 需要的控制面明显大于 `App`，
//! 避免 `AppKernelPort` 被动膨胀成新的大而全 façade。
//!
//! 同时提供 `Kernel` 对 `AgentKernelPort` 的 blanket impl。

use astrcode_core::{
    AgentInboxEnvelope, AgentLifecycleStatus, AgentTurnOutcome, ChildSessionNotification,
    DelegationMetadata, SubRunHandle,
};
use astrcode_kernel::{AgentControlError, Kernel, PendingParentDelivery};
use async_trait::async_trait;

use super::AppKernelPort;

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
    ) -> Result<SubRunHandle, AgentControlError>;
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
    ) -> Option<Vec<PendingParentDelivery>>;
    async fn pending_parent_delivery_count(&self, parent_session_id: &str) -> usize;
    async fn requeue_parent_delivery_batch(&self, parent_session_id: &str, delivery_ids: &[String]);
    async fn consume_parent_delivery_batch(
        &self,
        parent_session_id: &str,
        delivery_ids: &[String],
    ) -> bool;
}

#[async_trait]
impl AgentKernelPort for Kernel {
    async fn get_lifecycle(&self, sub_run_or_agent_id: &str) -> Option<AgentLifecycleStatus> {
        self.agent().get_lifecycle(sub_run_or_agent_id).await
    }

    async fn get_turn_outcome(&self, sub_run_or_agent_id: &str) -> Option<AgentTurnOutcome> {
        self.agent().get_turn_outcome(sub_run_or_agent_id).await
    }

    async fn resume(
        &self,
        sub_run_or_agent_id: &str,
        parent_turn_id: &str,
    ) -> Option<SubRunHandle> {
        self.agent()
            .resume(sub_run_or_agent_id, parent_turn_id)
            .await
    }

    async fn spawn_independent_child(
        &self,
        profile: &astrcode_core::AgentProfile,
        session_id: String,
        child_session_id: String,
        parent_turn_id: String,
        parent_agent_id: String,
    ) -> Result<SubRunHandle, AgentControlError> {
        self.agent()
            .spawn_independent_child(
                profile,
                session_id,
                child_session_id,
                parent_turn_id,
                parent_agent_id,
            )
            .await
    }

    async fn set_lifecycle(
        &self,
        sub_run_or_agent_id: &str,
        new_status: AgentLifecycleStatus,
    ) -> Option<()> {
        self.agent()
            .set_lifecycle(sub_run_or_agent_id, new_status)
            .await
    }

    async fn complete_turn(
        &self,
        sub_run_or_agent_id: &str,
        outcome: AgentTurnOutcome,
    ) -> Option<AgentLifecycleStatus> {
        self.agent_control()
            .complete_turn(sub_run_or_agent_id, outcome)
            .await
    }

    async fn set_delegation(
        &self,
        sub_run_or_agent_id: &str,
        delegation: Option<DelegationMetadata>,
    ) -> Option<()> {
        self.agent()
            .set_delegation(sub_run_or_agent_id, delegation)
            .await
    }

    async fn count_children_spawned_for_turn(
        &self,
        parent_agent_id: &str,
        parent_turn_id: &str,
    ) -> usize {
        self.agent()
            .count_children_spawned_for_turn(parent_agent_id, parent_turn_id)
            .await
    }

    async fn collect_subtree_handles(&self, sub_run_or_agent_id: &str) -> Vec<SubRunHandle> {
        self.agent()
            .collect_subtree_handles(sub_run_or_agent_id)
            .await
    }

    async fn terminate_subtree(&self, sub_run_or_agent_id: &str) -> Option<SubRunHandle> {
        self.agent().terminate_subtree(sub_run_or_agent_id).await
    }

    async fn deliver(&self, agent_id: &str, envelope: AgentInboxEnvelope) -> Option<()> {
        self.agent().deliver(agent_id, envelope).await
    }

    async fn drain_inbox(&self, agent_id: &str) -> Option<Vec<AgentInboxEnvelope>> {
        self.agent().drain_inbox(agent_id).await
    }

    async fn enqueue_child_delivery(
        &self,
        parent_session_id: String,
        parent_turn_id: String,
        notification: ChildSessionNotification,
    ) -> bool {
        self.agent()
            .enqueue_child_delivery(parent_session_id, parent_turn_id, notification)
            .await
    }

    async fn checkout_parent_delivery_batch(
        &self,
        parent_session_id: &str,
    ) -> Option<Vec<PendingParentDelivery>> {
        self.agent()
            .checkout_parent_delivery_batch(parent_session_id)
            .await
    }

    async fn pending_parent_delivery_count(&self, parent_session_id: &str) -> usize {
        self.agent_control()
            .pending_parent_delivery_count(parent_session_id)
            .await
    }

    async fn requeue_parent_delivery_batch(
        &self,
        parent_session_id: &str,
        delivery_ids: &[String],
    ) {
        self.agent()
            .requeue_parent_delivery_batch(parent_session_id, delivery_ids)
            .await
    }

    async fn consume_parent_delivery_batch(
        &self,
        parent_session_id: &str,
        delivery_ids: &[String],
    ) -> bool {
        self.agent()
            .consume_parent_delivery_batch(parent_session_id, delivery_ids)
            .await
    }
}
