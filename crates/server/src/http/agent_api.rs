//! server-owned agent route bridge。
//!
//! 负责把 agent 路由需要的 root execute / status / close 能力，
//! 直接接到 kernel + session-runtime + profile/governance 装配面，
//! 所有 agent 路由统一经由 server-owned agent API。

use std::sync::Arc;

use astrcode_core::{
    AgentLifecycleStatus, AgentProfile, ResolvedExecutionLimitsSnapshot,
    ResolvedSubagentContextOverrides, SubRunResult, SubRunStorageMode,
};

use crate::{
    agent_control_bridge::{
        ServerAgentControlPort, ServerCloseAgentSummary, ServerLiveSubRunStatus,
    },
    application_error_bridge::ServerRouteError,
    ports::{AppSessionPort, DurableSubRunStatusSummary},
    profile_service::ServerProfileService,
    root_execute_service::{
        ServerAgentExecuteSummary, ServerRootExecuteService, ServerRootExecutionRequest,
        ServerSessionPromptRequest,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ServerSubRunStatusSource {
    Live,
    Durable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServerSubRunStatusSummary {
    pub sub_run_id: String,
    pub tool_call_id: Option<String>,
    pub source: ServerSubRunStatusSource,
    pub agent_id: String,
    pub agent_profile: String,
    pub session_id: String,
    pub child_session_id: Option<String>,
    pub depth: usize,
    pub parent_agent_id: Option<String>,
    pub parent_sub_run_id: Option<String>,
    pub storage_mode: SubRunStorageMode,
    pub lifecycle: AgentLifecycleStatus,
    pub last_turn_outcome: Option<astrcode_core::AgentTurnOutcome>,
    pub result: Option<SubRunResult>,
    pub step_count: Option<u32>,
    pub estimated_tokens: Option<u64>,
    pub resolved_overrides: Option<ResolvedSubagentContextOverrides>,
    pub resolved_limits: Option<ResolvedExecutionLimitsSnapshot>,
}

#[derive(Clone)]
pub(crate) struct ServerAgentApi {
    agent_control: Arc<dyn ServerAgentControlPort>,
    sessions: Arc<dyn AppSessionPort>,
    profiles: Arc<ServerProfileService>,
    root_executor: Arc<ServerRootExecuteService>,
}

impl ServerAgentApi {
    pub(crate) fn new(
        agent_control: Arc<dyn ServerAgentControlPort>,
        sessions: Arc<dyn AppSessionPort>,
        profiles: Arc<ServerProfileService>,
        root_executor: Arc<ServerRootExecuteService>,
    ) -> Self {
        Self {
            agent_control,
            sessions,
            profiles,
            root_executor,
        }
    }

    pub(crate) fn list_global_agent_profiles(&self) -> Result<Vec<AgentProfile>, ServerRouteError> {
        Ok(self.profiles.resolve_global()?.as_ref().clone())
    }

    pub(crate) async fn execute_root_agent_summary(
        &self,
        request: ServerRootExecutionRequest,
    ) -> Result<ServerAgentExecuteSummary, ServerRouteError> {
        self.root_executor.execute_summary(request).await
    }

    pub(crate) async fn submit_existing_session_prompt(
        &self,
        request: ServerSessionPromptRequest,
    ) -> Result<ServerAgentExecuteSummary, ServerRouteError> {
        self.root_executor
            .submit_existing_session_prompt(request)
            .await
    }

    pub(crate) async fn get_subrun_status_summary(
        &self,
        session_id: &str,
        requested_subrun_id: &str,
    ) -> Result<ServerSubRunStatusSummary, ServerRouteError> {
        validate_non_empty("sessionId", session_id)?;
        validate_non_empty("subRunId", requested_subrun_id)?;

        if let Some(view) = self.get_subrun_status(requested_subrun_id).await? {
            if view.session_id == session_id {
                return Ok(summarize_live_subrun_status(view));
            }
        }

        if let Some(view) = self.get_root_agent_status(session_id).await? {
            if view.sub_run_id == requested_subrun_id {
                return Ok(summarize_live_subrun_status(view));
            }
            return Err(ServerRouteError::not_found(format!(
                "subrun '{}' not found in session '{}'",
                requested_subrun_id, session_id
            )));
        }

        if let Some(summary) = self
            .durable_subrun_status_summary(session_id, requested_subrun_id)
            .await?
        {
            return Ok(summary);
        }

        Ok(default_subrun_status_summary(
            session_id.to_string(),
            requested_subrun_id.to_string(),
        ))
    }

    pub(crate) async fn close_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<ServerCloseAgentSummary, ServerRouteError> {
        validate_non_empty("sessionId", session_id)?;
        validate_non_empty("agentId", agent_id)?;
        let Some(handle) = self.agent_control.get_handle(agent_id).await else {
            return Err(ServerRouteError::not_found(format!(
                "agent '{}' not found",
                agent_id
            )));
        };
        if handle.session_id.as_str() != session_id {
            return Err(ServerRouteError::not_found(format!(
                "agent '{}' not found in session '{}'",
                agent_id, session_id
            )));
        }
        self.agent_control.close_subtree(agent_id).await
    }

    pub(crate) async fn get_subrun_status(
        &self,
        agent_id: &str,
    ) -> Result<Option<ServerLiveSubRunStatus>, ServerRouteError> {
        validate_non_empty("agentId", agent_id)?;
        Ok(self.agent_control.query_subrun_status(agent_id).await)
    }

    async fn get_root_agent_status(
        &self,
        session_id: &str,
    ) -> Result<Option<ServerLiveSubRunStatus>, ServerRouteError> {
        validate_non_empty("sessionId", session_id)?;
        Ok(self.agent_control.query_root_status(session_id).await)
    }

    async fn durable_subrun_status_summary(
        &self,
        parent_session_id: &str,
        requested_subrun_id: &str,
    ) -> Result<Option<ServerSubRunStatusSummary>, ServerRouteError> {
        Ok(self
            .sessions
            .durable_subrun_status_snapshot(parent_session_id, requested_subrun_id)
            .await
            .map_err(|error| ServerRouteError::internal(error.to_string()))?
            .map(summarize_durable_subrun_status))
    }
}

fn validate_non_empty(field: &str, value: &str) -> Result<(), ServerRouteError> {
    if value.trim().is_empty() {
        return Err(ServerRouteError::invalid_argument(format!(
            "field '{field}' must not be empty"
        )));
    }
    Ok(())
}

fn summarize_live_subrun_status(view: ServerLiveSubRunStatus) -> ServerSubRunStatusSummary {
    ServerSubRunStatusSummary {
        sub_run_id: view.sub_run_id,
        tool_call_id: None,
        source: ServerSubRunStatusSource::Live,
        agent_id: view.agent_id,
        agent_profile: view.agent_profile,
        session_id: view.session_id,
        child_session_id: view.child_session_id,
        depth: view.depth,
        parent_agent_id: view.parent_agent_id,
        parent_sub_run_id: None,
        storage_mode: SubRunStorageMode::IndependentSession,
        lifecycle: view.lifecycle,
        last_turn_outcome: view.last_turn_outcome,
        result: None,
        step_count: None,
        estimated_tokens: None,
        resolved_overrides: None,
        resolved_limits: Some(view.resolved_limits),
    }
}

fn default_subrun_status_summary(
    session_id: String,
    sub_run_id: String,
) -> ServerSubRunStatusSummary {
    ServerSubRunStatusSummary {
        sub_run_id,
        tool_call_id: None,
        source: ServerSubRunStatusSource::Live,
        agent_id: "root-agent".to_string(),
        agent_profile: "default".to_string(),
        session_id,
        child_session_id: None,
        depth: 0,
        parent_agent_id: None,
        parent_sub_run_id: None,
        storage_mode: SubRunStorageMode::IndependentSession,
        lifecycle: AgentLifecycleStatus::Idle,
        last_turn_outcome: None,
        result: None,
        step_count: None,
        estimated_tokens: None,
        resolved_overrides: None,
        resolved_limits: Some(ResolvedExecutionLimitsSnapshot),
    }
}

fn summarize_durable_subrun_status(
    snapshot: DurableSubRunStatusSummary,
) -> ServerSubRunStatusSummary {
    ServerSubRunStatusSummary {
        sub_run_id: snapshot.sub_run_id,
        tool_call_id: snapshot.tool_call_id,
        source: ServerSubRunStatusSource::Durable,
        agent_id: snapshot.agent_id,
        agent_profile: snapshot.agent_profile,
        session_id: snapshot.session_id,
        child_session_id: snapshot.child_session_id,
        depth: snapshot.depth,
        parent_agent_id: snapshot.parent_agent_id,
        parent_sub_run_id: snapshot.parent_sub_run_id,
        storage_mode: snapshot.storage_mode,
        lifecycle: snapshot.lifecycle,
        last_turn_outcome: snapshot.last_turn_outcome,
        result: snapshot.result,
        step_count: snapshot.step_count,
        estimated_tokens: snapshot.estimated_tokens,
        resolved_overrides: snapshot.resolved_overrides,
        resolved_limits: Some(snapshot.resolved_limits),
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{
        AgentLifecycleStatus, AgentTurnOutcome, ResolvedExecutionLimitsSnapshot,
        ResolvedSubagentContextOverrides, SubRunStorageMode,
    };

    use super::{ServerSubRunStatusSource, summarize_durable_subrun_status};
    use crate::ports::DurableSubRunStatusSummary;

    #[test]
    fn summarize_durable_subrun_status_reuses_runtime_projection() {
        let summary = summarize_durable_subrun_status(DurableSubRunStatusSummary {
            sub_run_id: "subrun-child".to_string(),
            tool_call_id: Some("tool-1".to_string()),
            agent_id: "agent-child".to_string(),
            agent_profile: "reviewer".to_string(),
            session_id: "session-parent".to_string(),
            child_session_id: Some("session-child".to_string()),
            depth: 1,
            parent_agent_id: None,
            parent_sub_run_id: Some("subrun-parent".to_string()),
            storage_mode: SubRunStorageMode::IndependentSession,
            lifecycle: AgentLifecycleStatus::Idle,
            last_turn_outcome: Some(AgentTurnOutcome::Completed),
            result: None,
            step_count: Some(3),
            estimated_tokens: Some(120),
            resolved_overrides: Some(ResolvedSubagentContextOverrides::default()),
            resolved_limits: ResolvedExecutionLimitsSnapshot,
        });

        assert_eq!(summary.source, ServerSubRunStatusSource::Durable);
        assert_eq!(summary.sub_run_id, "subrun-child");
        assert_eq!(summary.child_session_id.as_deref(), Some("session-child"));
        assert_eq!(summary.step_count, Some(3));
        assert!(summary.resolved_limits.is_some());
    }
}
