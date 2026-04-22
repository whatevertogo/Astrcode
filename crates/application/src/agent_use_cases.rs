//! Agent 控制用例（`App` 的 agent 相关方法）。
//!
//! 通过 kernel 的稳定控制合同实现 agent 状态查询、子运行生命周期管理等用例。

use astrcode_core::{AgentLifecycleStatus, ResolvedExecutionLimitsSnapshot, SubRunStorageMode};
use astrcode_kernel::SubRunStatusView;

use crate::{
    AgentExecuteSummary, App, ApplicationError, RootExecutionRequest, SubRunStatusSourceSummary,
    SubRunStatusSummary,
};

impl App {
    // ── Agent 控制用例（通过 kernel 稳定控制合同） ──────────

    /// 查询子运行状态。
    pub async fn get_subrun_status(
        &self,
        agent_id: &str,
    ) -> Result<Option<SubRunStatusView>, ApplicationError> {
        self.validate_non_empty("agentId", agent_id)?;
        Ok(self.kernel.query_subrun_status(agent_id).await)
    }

    /// 查询指定 session 的根 agent 状态。
    pub async fn get_root_agent_status(
        &self,
        session_id: &str,
    ) -> Result<Option<SubRunStatusView>, ApplicationError> {
        self.validate_non_empty("sessionId", session_id)?;
        Ok(self.kernel.query_root_status(session_id).await)
    }

    /// 列出所有 agent 状态。
    pub async fn list_agent_statuses(&self) -> Vec<SubRunStatusView> {
        self.kernel.list_statuses().await
    }

    /// 执行 root agent 并返回共享摘要输入。
    pub async fn execute_root_agent_summary(
        &self,
        request: RootExecutionRequest,
    ) -> Result<AgentExecuteSummary, ApplicationError> {
        let accepted = self.execute_root_agent(request).await?;
        let session_id = accepted.session_id.to_string();
        Ok(AgentExecuteSummary {
            accepted: true,
            message: format!(
                "agent '{}' execution accepted; subscribe to \
                 /api/v1/conversation/sessions/{}/stream for progress",
                accepted.agent_id.as_deref().unwrap_or("unknown-agent"),
                session_id
            ),
            session_id: Some(session_id),
            turn_id: Some(accepted.turn_id.to_string()),
            agent_id: accepted.agent_id.map(|value| value.to_string()),
        })
    }

    /// 查询指定 session/sub-run 的共享状态摘要。
    ///
    /// 查找策略（按优先级）：
    /// 1. Live 状态：从 kernel 获取 sub-run 或 root agent 的实时状态
    /// 2. Durable 状态：从 session-runtime 的只读投影读取 child session 终态
    /// 3. 都找不到：返回默认的 Idle 状态摘要
    pub async fn get_subrun_status_summary(
        &self,
        session_id: &str,
        requested_subrun_id: &str,
    ) -> Result<SubRunStatusSummary, ApplicationError> {
        self.validate_non_empty("sessionId", session_id)?;
        self.validate_non_empty("subRunId", requested_subrun_id)?;

        if let Some(view) = self.get_subrun_status(requested_subrun_id).await? {
            return Ok(summarize_live_subrun_status(view, session_id.to_string()));
        }

        if let Some(view) = self.get_root_agent_status(session_id).await? {
            if view.sub_run_id == requested_subrun_id {
                return Ok(summarize_live_subrun_status(view, session_id.to_string()));
            }
            return Err(ApplicationError::NotFound(format!(
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

    /// 从 session-runtime 的 durable query 读取子运行状态。
    async fn durable_subrun_status_summary(
        &self,
        parent_session_id: &str,
        requested_subrun_id: &str,
    ) -> Result<Option<SubRunStatusSummary>, ApplicationError> {
        Ok(self
            .session_runtime
            .durable_subrun_status_snapshot(parent_session_id, requested_subrun_id)
            .await?
            .map(summarize_durable_subrun_status))
    }

    /// 关闭 agent 及其子树。
    pub async fn close_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<astrcode_kernel::CloseSubtreeResult, ApplicationError> {
        self.validate_non_empty("sessionId", session_id)?;
        self.validate_non_empty("agentId", agent_id)?;
        let Some(handle) = self.kernel.get_handle(agent_id).await else {
            return Err(ApplicationError::NotFound(format!(
                "agent '{}' not found",
                agent_id
            )));
        };
        if handle.session_id.as_str() != session_id {
            return Err(ApplicationError::NotFound(format!(
                "agent '{}' not found in session '{}'",
                agent_id, session_id
            )));
        }
        self.kernel
            .close_subtree(agent_id)
            .await
            .map_err(|error| ApplicationError::Internal(error.to_string()))
    }
}

fn summarize_live_subrun_status(view: SubRunStatusView, session_id: String) -> SubRunStatusSummary {
    SubRunStatusSummary {
        sub_run_id: view.sub_run_id,
        tool_call_id: None,
        source: SubRunStatusSourceSummary::Live,
        agent_id: view.agent_id,
        agent_profile: view.agent_profile,
        session_id,
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

fn default_subrun_status_summary(session_id: String, sub_run_id: String) -> SubRunStatusSummary {
    SubRunStatusSummary {
        sub_run_id,
        tool_call_id: None,
        source: SubRunStatusSourceSummary::Live,
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
    snapshot: astrcode_session_runtime::SubRunStatusSnapshot,
) -> SubRunStatusSummary {
    let handle = snapshot.handle;
    SubRunStatusSummary {
        sub_run_id: handle.sub_run_id.to_string(),
        tool_call_id: snapshot.tool_call_id,
        source: SubRunStatusSourceSummary::Durable,
        agent_id: handle.agent_id.to_string(),
        agent_profile: handle.agent_profile,
        session_id: handle.session_id.to_string(),
        child_session_id: handle.child_session_id.map(|id| id.to_string()),
        depth: handle.depth,
        parent_agent_id: handle.parent_agent_id.map(|id| id.to_string()),
        parent_sub_run_id: handle.parent_sub_run_id.map(|id| id.to_string()),
        storage_mode: handle.storage_mode,
        lifecycle: handle.lifecycle,
        last_turn_outcome: handle.last_turn_outcome,
        result: snapshot.result,
        step_count: snapshot.step_count,
        estimated_tokens: snapshot.estimated_tokens,
        resolved_overrides: snapshot.resolved_overrides,
        resolved_limits: Some(handle.resolved_limits),
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{
        AgentLifecycleStatus, AgentTurnOutcome, ResolvedExecutionLimitsSnapshot,
        ResolvedSubagentContextOverrides, SubRunHandle, SubRunStorageMode,
    };
    use astrcode_session_runtime::{SubRunStatusSnapshot, SubRunStatusSource};

    use super::summarize_durable_subrun_status;
    use crate::SubRunStatusSourceSummary;

    #[test]
    fn summarize_durable_subrun_status_reuses_runtime_projection() {
        let summary = summarize_durable_subrun_status(SubRunStatusSnapshot {
            handle: SubRunHandle {
                sub_run_id: "subrun-child".into(),
                agent_id: "agent-child".into(),
                session_id: "session-parent".into(),
                child_session_id: Some("session-child".into()),
                depth: 1,
                parent_turn_id: "turn-parent".into(),
                parent_agent_id: None,
                parent_sub_run_id: Some("subrun-parent".into()),
                lineage_kind: astrcode_core::ChildSessionLineageKind::Spawn,
                agent_profile: "reviewer".to_string(),
                storage_mode: SubRunStorageMode::IndependentSession,
                lifecycle: AgentLifecycleStatus::Idle,
                last_turn_outcome: Some(AgentTurnOutcome::Completed),
                resolved_limits: ResolvedExecutionLimitsSnapshot,
                delegation: None,
            },
            tool_call_id: Some("call-1".to_string()),
            source: SubRunStatusSource::Durable,
            result: None,
            step_count: Some(5),
            estimated_tokens: Some(2048),
            resolved_overrides: Some(ResolvedSubagentContextOverrides::default()),
        });

        assert_eq!(summary.source, SubRunStatusSourceSummary::Durable);
        assert_eq!(summary.session_id, "session-parent");
        assert_eq!(summary.child_session_id.as_deref(), Some("session-child"));
        assert_eq!(summary.tool_call_id.as_deref(), Some("call-1"));
        assert_eq!(summary.last_turn_outcome, Some(AgentTurnOutcome::Completed));
        assert_eq!(summary.step_count, Some(5));
    }
}
