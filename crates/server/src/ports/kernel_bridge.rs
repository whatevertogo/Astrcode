use std::sync::Arc;

use astrcode_core::{
    AgentInboxEnvelope, AgentLifecycleStatus, AgentTurnOutcome, ChildSessionNotification,
    DelegationMetadata, ResolvedExecutionLimitsSnapshot,
};
use astrcode_host_session::SubRunHandle;
use async_trait::async_trait;

use super::{AgentKernelPort, AppKernelPort, RecoverableParentDelivery, ServerKernelControlError};
use crate::{
    agent_control_bridge::{
        ServerAgentControlPort, ServerAgentHandleSummary, ServerCloseAgentSummary,
        ServerLiveSubRunStatus,
    },
    application_error_bridge::ServerRouteError,
    session_runtime_port::SessionRuntimePort,
};

pub(crate) fn build_server_kernel_bridge(
    session_runtime: Arc<dyn SessionRuntimePort>,
) -> Arc<ServerKernelBridge> {
    Arc::new(ServerKernelBridge { session_runtime })
}

pub(crate) struct ServerKernelBridge {
    session_runtime: Arc<dyn SessionRuntimePort>,
}

#[async_trait]
impl AppKernelPort for ServerKernelBridge {
    async fn get_handle(&self, agent_id: &str) -> Option<SubRunHandle> {
        self.session_runtime.get_handle(agent_id).await
    }

    async fn find_root_handle_for_session(&self, session_id: &str) -> Option<SubRunHandle> {
        self.session_runtime
            .find_root_handle_for_session(session_id)
            .await
    }

    async fn register_root_agent(
        &self,
        agent_id: String,
        session_id: String,
        profile_id: String,
    ) -> Result<SubRunHandle, ServerKernelControlError> {
        self.session_runtime
            .register_root_agent(agent_id, session_id, profile_id)
            .await
    }

    async fn set_resolved_limits(
        &self,
        sub_run_or_agent_id: &str,
        resolved_limits: ResolvedExecutionLimitsSnapshot,
    ) -> Option<()> {
        self.session_runtime
            .set_resolved_limits(sub_run_or_agent_id, resolved_limits)
            .await
    }
}

#[async_trait]
impl AgentKernelPort for ServerKernelBridge {
    async fn get_lifecycle(&self, sub_run_or_agent_id: &str) -> Option<AgentLifecycleStatus> {
        self.session_runtime
            .get_lifecycle(sub_run_or_agent_id)
            .await
    }

    async fn get_turn_outcome(&self, sub_run_or_agent_id: &str) -> Option<AgentTurnOutcome> {
        self.session_runtime
            .get_turn_outcome(sub_run_or_agent_id)
            .await
    }

    async fn resume(
        &self,
        sub_run_or_agent_id: &str,
        parent_turn_id: &str,
    ) -> Option<SubRunHandle> {
        self.session_runtime
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
    ) -> Result<SubRunHandle, ServerKernelControlError> {
        self.session_runtime
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
        self.session_runtime
            .set_lifecycle(sub_run_or_agent_id, new_status)
            .await
    }

    async fn complete_turn(
        &self,
        sub_run_or_agent_id: &str,
        outcome: AgentTurnOutcome,
    ) -> Option<AgentLifecycleStatus> {
        self.session_runtime
            .complete_turn(sub_run_or_agent_id, outcome)
            .await
    }

    async fn set_delegation(
        &self,
        sub_run_or_agent_id: &str,
        delegation: Option<DelegationMetadata>,
    ) -> Option<()> {
        self.session_runtime
            .set_delegation(sub_run_or_agent_id, delegation)
            .await
    }

    async fn count_children_spawned_for_turn(
        &self,
        parent_agent_id: &str,
        parent_turn_id: &str,
    ) -> usize {
        self.session_runtime
            .count_children_spawned_for_turn(parent_agent_id, parent_turn_id)
            .await
    }

    async fn collect_subtree_handles(&self, sub_run_or_agent_id: &str) -> Vec<SubRunHandle> {
        self.session_runtime
            .collect_subtree_handles(sub_run_or_agent_id)
            .await
    }

    async fn terminate_subtree(&self, sub_run_or_agent_id: &str) -> Option<SubRunHandle> {
        self.session_runtime
            .terminate_subtree(sub_run_or_agent_id)
            .await
    }

    async fn deliver(&self, agent_id: &str, envelope: AgentInboxEnvelope) -> Option<()> {
        self.session_runtime.deliver(agent_id, envelope).await
    }

    async fn drain_inbox(&self, agent_id: &str) -> Option<Vec<AgentInboxEnvelope>> {
        self.session_runtime.drain_inbox(agent_id).await
    }

    async fn enqueue_child_delivery(
        &self,
        parent_session_id: String,
        parent_turn_id: String,
        notification: ChildSessionNotification,
    ) -> bool {
        self.session_runtime
            .enqueue_child_delivery(parent_session_id, parent_turn_id, notification)
            .await
    }

    async fn checkout_parent_delivery_batch(
        &self,
        parent_session_id: &str,
    ) -> Option<Vec<RecoverableParentDelivery>> {
        self.session_runtime
            .checkout_parent_delivery_batch(parent_session_id)
            .await
            .map(|deliveries| {
                deliveries
                    .into_iter()
                    .map(|value| RecoverableParentDelivery {
                        delivery_id: value.delivery_id,
                        parent_session_id: value.parent_session_id,
                        parent_turn_id: value.parent_turn_id,
                        queued_at_ms: value.queued_at_ms,
                        notification: value.notification,
                    })
                    .collect()
            })
    }

    async fn pending_parent_delivery_count(&self, parent_session_id: &str) -> usize {
        self.session_runtime
            .pending_parent_delivery_count(parent_session_id)
            .await
    }

    async fn requeue_parent_delivery_batch(
        &self,
        parent_session_id: &str,
        delivery_ids: &[String],
    ) {
        self.session_runtime
            .requeue_parent_delivery_batch(parent_session_id, delivery_ids)
            .await
    }

    async fn consume_parent_delivery_batch(
        &self,
        parent_session_id: &str,
        delivery_ids: &[String],
    ) -> bool {
        self.session_runtime
            .consume_parent_delivery_batch(parent_session_id, delivery_ids)
            .await
    }
}

#[async_trait]
impl ServerAgentControlPort for ServerKernelBridge {
    async fn query_subrun_status(&self, agent_id: &str) -> Option<ServerLiveSubRunStatus> {
        self.session_runtime.query_subrun_status(agent_id).await
    }

    async fn query_root_status(&self, session_id: &str) -> Option<ServerLiveSubRunStatus> {
        self.session_runtime.query_root_status(session_id).await
    }

    async fn get_handle(&self, agent_id: &str) -> Option<ServerAgentHandleSummary> {
        self.session_runtime
            .get_handle(agent_id)
            .await
            .map(|handle| ServerAgentHandleSummary {
                agent_id: handle.agent_id.to_string(),
                session_id: handle.session_id.to_string(),
            })
    }

    async fn register_root_agent(
        &self,
        agent_id: String,
        session_id: String,
        profile_id: String,
    ) -> Result<ServerAgentHandleSummary, ServerRouteError> {
        self.session_runtime
            .register_root_agent(agent_id, session_id, profile_id)
            .await
            .map(|handle| ServerAgentHandleSummary {
                agent_id: handle.agent_id.to_string(),
                session_id: handle.session_id.to_string(),
            })
            .map_err(|error| {
                ServerRouteError::internal(format!("failed to register root agent: {error}"))
            })
    }

    async fn set_resolved_limits(
        &self,
        sub_run_or_agent_id: &str,
        resolved_limits: ResolvedExecutionLimitsSnapshot,
    ) -> bool {
        self.session_runtime
            .set_resolved_limits(sub_run_or_agent_id, resolved_limits)
            .await
            .is_some()
    }

    async fn close_subtree(
        &self,
        agent_id: &str,
    ) -> Result<ServerCloseAgentSummary, ServerRouteError> {
        self.session_runtime
            .close_subtree(agent_id)
            .await
            .map(|result| ServerCloseAgentSummary {
                closed_agent_ids: result.closed_agent_ids,
            })
            .map_err(|error| ServerRouteError::internal(error.to_string()))
    }
}
