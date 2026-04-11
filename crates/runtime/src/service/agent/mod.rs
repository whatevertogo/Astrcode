//! agent 编排边界。
//!
//! 这里承接“谁给谁发消息、谁可以关闭谁、谁可以观察谁”的协作语义，
//! 保持 execution 只负责“执行一轮 turn”。

mod context;
mod mailbox;
mod observe;
mod routing;
mod wake;

use std::sync::Arc;

use astrcode_core::{
    AgentProfile, AgentProfileCatalog, Result, SpawnAgentParams, SubRunHandle, SubRunResult,
    ToolContext,
};
use astrcode_runtime_agent_tool::SubAgentExecutor;
use async_trait::async_trait;
pub(crate) use context::{DeferredCollaborationExecutor, service_error_to_astr};

use crate::service::{RuntimeService, ServiceResult};

/// Agent 编排服务句柄。
#[derive(Clone)]
pub struct AgentServiceHandle {
    pub(crate) runtime: Arc<RuntimeService>,
}

#[async_trait]
impl SubAgentExecutor for AgentServiceHandle {
    async fn launch(&self, params: SpawnAgentParams, ctx: &ToolContext) -> Result<SubRunResult> {
        self.runtime
            .execution()
            .launch_subagent(params, ctx)
            .await
            .map_err(service_error_to_astr)
    }
}

impl AgentProfileCatalog for AgentServiceHandle {
    fn list_subagent_profiles(&self) -> Vec<AgentProfile> {
        self.runtime
            .agent_profiles()
            .list_subagent_profiles()
            .into_iter()
            .cloned()
            .collect()
    }
}

impl AgentServiceHandle {
    pub fn control(&self) -> astrcode_runtime_agent_control::AgentControl {
        self.runtime.agent_control.clone()
    }

    pub(crate) async fn current_loop(&self) -> Arc<astrcode_runtime_agent_loop::AgentLoop> {
        self.runtime.loop_surface().current_loop().await
    }

    pub(crate) fn collaboration_executor(&self) -> Arc<DeferredCollaborationExecutor> {
        Arc::clone(&self.runtime.collaboration_executor)
    }

    /// 查询指定 sub-run 的 live handle。
    pub async fn get_subrun_handle(
        &self,
        session_id: &str,
        sub_run_id: &str,
    ) -> ServiceResult<Option<SubRunHandle>> {
        let normalized_session_id = astrcode_runtime_session::normalize_session_id(session_id);
        Ok(self
            .runtime
            .agent_control
            .get(sub_run_id)
            .await
            .filter(|handle| {
                let handle_session_id =
                    astrcode_runtime_session::normalize_session_id(&handle.session_id);
                let child_session_id = handle
                    .child_session_id
                    .as_deref()
                    .map(astrcode_runtime_session::normalize_session_id);
                handle_session_id == normalized_session_id
                    || child_session_id.as_deref() == Some(normalized_session_id.as_str())
            }))
    }
}
