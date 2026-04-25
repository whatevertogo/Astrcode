//! server-owned agent control bridge。
//!
//! 收敛 agent 路由和 root execute 需要的 live status / close / root register 能力，
//! 让协议层和 server 状态面只暴露 server-owned DTO。

use astrcode_core::{AgentLifecycleStatus, AgentTurnOutcome, ResolvedExecutionLimitsSnapshot};
use async_trait::async_trait;

use crate::application_error_bridge::ServerRouteError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServerAgentHandleSummary {
    pub agent_id: String,
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServerLiveSubRunStatus {
    pub sub_run_id: String,
    pub agent_id: String,
    pub agent_profile: String,
    pub session_id: String,
    pub child_session_id: Option<String>,
    pub depth: usize,
    pub parent_agent_id: Option<String>,
    pub lifecycle: AgentLifecycleStatus,
    pub last_turn_outcome: Option<AgentTurnOutcome>,
    pub resolved_limits: ResolvedExecutionLimitsSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServerCloseAgentSummary {
    pub closed_agent_ids: Vec<String>,
}

#[async_trait]
pub(crate) trait ServerAgentControlPort: Send + Sync {
    async fn query_subrun_status(&self, agent_id: &str) -> Option<ServerLiveSubRunStatus>;
    async fn query_root_status(&self, session_id: &str) -> Option<ServerLiveSubRunStatus>;
    async fn get_handle(&self, agent_id: &str) -> Option<ServerAgentHandleSummary>;
    async fn register_root_agent(
        &self,
        agent_id: String,
        session_id: String,
        profile_id: String,
    ) -> Result<ServerAgentHandleSummary, ServerRouteError>;
    async fn set_resolved_limits(
        &self,
        sub_run_or_agent_id: &str,
        resolved_limits: ResolvedExecutionLimitsSnapshot,
    ) -> bool;
    async fn close_subtree(
        &self,
        agent_id: &str,
    ) -> Result<ServerCloseAgentSummary, ServerRouteError>;
}
