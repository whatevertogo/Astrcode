use astrcode_core::{
    AgentInboxEnvelope, AgentLifecycleStatus, AgentTurnOutcome, ChildSessionNotification,
    DelegationMetadata, ResolvedExecutionLimitsSnapshot, ResolvedRuntimeConfig, StoredEvent,
    TurnId,
};
use astrcode_governance_contract::ModeId;
use astrcode_host_session::SubRunHandle;
use astrcode_runtime_contract::ExecutionSubmissionOutcome;
use async_trait::async_trait;

use crate::{
    agent_control_bridge::{ServerCloseAgentSummary, ServerLiveSubRunStatus},
    application_error_bridge::ServerRouteError,
    ports::{
        AppAgentPromptSubmission, RecoverableParentDelivery, ServerKernelControlError,
        SessionObserveSnapshot,
    },
};

#[path = "session_runtime_port_adapter.rs"]
pub(crate) mod adapter;

#[allow(dead_code)]
#[async_trait]
pub(crate) trait SessionRuntimePort: Send + Sync {
    async fn submit_prompt_for_agent(
        &self,
        session_id: &str,
        text: String,
        runtime: ResolvedRuntimeConfig,
        submission: AppAgentPromptSubmission,
    ) -> astrcode_core::Result<ExecutionSubmissionOutcome>;
    async fn interrupt_session(&self, session_id: &str) -> astrcode_core::Result<()>;
    async fn compact_session(
        &self,
        session_id: &str,
        runtime: ResolvedRuntimeConfig,
        instructions: Option<String>,
    ) -> astrcode_core::Result<bool>;
    async fn switch_mode(
        &self,
        session_id: &str,
        from: ModeId,
        to: ModeId,
    ) -> astrcode_core::Result<StoredEvent>;
    async fn submit_prompt_for_agent_with_submission(
        &self,
        session_id: &str,
        text: String,
        runtime: ResolvedRuntimeConfig,
        submission: AppAgentPromptSubmission,
    ) -> astrcode_core::Result<ExecutionSubmissionOutcome>;
    async fn try_submit_prompt_for_agent_with_turn_id(
        &self,
        session_id: &str,
        turn_id: TurnId,
        text: String,
        runtime: ResolvedRuntimeConfig,
        submission: AppAgentPromptSubmission,
    ) -> astrcode_core::Result<Option<ExecutionSubmissionOutcome>>;
    async fn submit_queued_inputs_for_agent_with_turn_id(
        &self,
        session_id: &str,
        turn_id: TurnId,
        queued_inputs: Vec<String>,
        runtime: ResolvedRuntimeConfig,
        submission: AppAgentPromptSubmission,
    ) -> astrcode_core::Result<Option<ExecutionSubmissionOutcome>>;
    async fn observe_agent_session(
        &self,
        open_session_id: &str,
        target_agent_id: &str,
        lifecycle_status: AgentLifecycleStatus,
    ) -> astrcode_core::Result<SessionObserveSnapshot>;
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
        resolved_limits: ResolvedExecutionLimitsSnapshot,
    ) -> Option<()>;
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
    async fn query_subrun_status(&self, agent_id: &str) -> Option<ServerLiveSubRunStatus>;
    async fn query_root_status(&self, session_id: &str) -> Option<ServerLiveSubRunStatus>;
    async fn close_subtree(
        &self,
        agent_id: &str,
    ) -> Result<ServerCloseAgentSummary, ServerRouteError>;
}
