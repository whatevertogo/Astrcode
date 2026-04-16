/// ! 这是 App 的用例实现，不是 ports
use crate::{App, ApplicationError};

impl App {
    // ── Agent 控制用例（通过 kernel 稳定控制合同） ──────────

    /// 查询子运行状态。
    pub async fn get_subrun_status(
        &self,
        agent_id: &str,
    ) -> Result<Option<astrcode_kernel::SubRunStatusView>, ApplicationError> {
        self.validate_non_empty("agentId", agent_id)?;
        Ok(self.kernel.query_subrun_status(agent_id).await)
    }

    /// 查询指定 session 的根 agent 状态。
    pub async fn get_root_agent_status(
        &self,
        session_id: &str,
    ) -> Result<Option<astrcode_kernel::SubRunStatusView>, ApplicationError> {
        self.validate_non_empty("sessionId", session_id)?;
        Ok(self.kernel.query_root_status(session_id).await)
    }

    /// 列出所有 agent 状态。
    pub async fn list_agent_statuses(&self) -> Vec<astrcode_kernel::SubRunStatusView> {
        self.kernel.list_statuses().await
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
            // 显式校验归属，避免仅凭 agent_id 跨 session 关闭不相关子树。
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
