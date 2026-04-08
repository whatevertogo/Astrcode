//! execution owner handle 与内部执行辅助逻辑。
//!
//! 模块职责划分：
//! - `root`：根执行入口（execute_root_agent）与 handle 类型定义
//! - `subagent`：作为工具执行子 agent
//! - `surface`：读取当前 runtime surface 并构造 scoped execution 输入
//! - `status`：sub-run 状态查询
//! - `cancel`：sub-run 取消控制
//! - `context`：bootstrap 阶段的延迟执行器桥与错误转换工具

mod cancel;
mod context;
pub(super) mod root;
mod status;
mod subagent;
mod surface;

use std::sync::Arc;

use astrcode_core::{
    AgentProfile, AgentProfileCatalog, AstrError, ExecutionOrchestrationBoundary,
    LiveSubRunControlBoundary, Result, SpawnAgentParams, SubRunHandle, SubRunResult, ToolContext,
};
use astrcode_runtime_agent_tool::SubAgentExecutor;
use async_trait::async_trait;
pub(crate) use context::{DeferredSubAgentExecutor, service_error_to_astr};
pub use root::{
    AgentExecutionServiceHandle, AgentProfileSummary, ToolExecutionServiceHandle, ToolSummary,
};

use crate::service::{RuntimeService, ServiceError, ServiceResult};

impl RuntimeService {
    /// 获取 Agent 执行服务句柄。
    pub fn execution(self: &Arc<Self>) -> AgentExecutionServiceHandle {
        AgentExecutionServiceHandle {
            runtime: Arc::clone(self),
        }
    }

    /// 获取 Tool 执行服务句柄。
    pub fn tools(self: &Arc<Self>) -> ToolExecutionServiceHandle {
        ToolExecutionServiceHandle {
            runtime: Arc::clone(self),
        }
    }
}

#[async_trait]
impl SubAgentExecutor for AgentExecutionServiceHandle {
    async fn launch(&self, params: SpawnAgentParams, ctx: &ToolContext) -> Result<SubRunResult> {
        self.launch_subagent(params, ctx)
            .await
            .map_err(service_error_to_astr)
    }
}

impl AgentProfileCatalog for AgentExecutionServiceHandle {
    fn list_subagent_profiles(&self) -> Vec<AgentProfile> {
        self.runtime
            .agent_profiles()
            .list_subagent_profiles()
            .into_iter()
            .cloned()
            .collect()
    }
}

/// 加载指定工作目录的 agent profile 注册表。
impl AgentExecutionServiceHandle {
    pub(super) async fn load_profiles_for_working_dir(
        &self,
        working_dir: &std::path::Path,
    ) -> ServiceResult<Arc<astrcode_runtime_agent_loader::AgentProfileRegistry>> {
        if let Some(cached) = self.runtime.scoped_agent_profiles.get(working_dir) {
            return Ok(Arc::clone(cached.value()));
        }

        let loader = self.runtime.agent_loader();
        let working_dir = working_dir.to_path_buf();
        let load_working_dir = working_dir.clone();
        let registry = crate::service::blocking_bridge::spawn_blocking_service(
            "load scoped agent profiles",
            move || {
                loader
                    .load_for_working_dir(Some(&load_working_dir))
                    .map_err(|error| {
                        ServiceError::Internal(astrcode_core::AstrError::Validation(
                            error.to_string(),
                        ))
                    })
            },
        )
        .await?;
        let registry = Arc::new(registry);

        if let Some(cached) = self.runtime.scoped_agent_profiles.get(&working_dir) {
            return Ok(Arc::clone(cached.value()));
        }

        self.runtime
            .scoped_agent_profiles
            .insert(working_dir, Arc::clone(&registry));
        Ok(registry)
    }
}

impl AgentExecutionServiceHandle {
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

    pub(super) async fn reactivate_parent_agent_if_idle(
        &self,
        parent_session_id: &str,
        parent_turn_id: &str,
        notification: &astrcode_core::ChildSessionNotification,
    ) {
        let parent_session_id = astrcode_runtime_session::normalize_session_id(parent_session_id);

        let should_reactivate =
            self.runtime
                .sessions
                .get(&parent_session_id)
                .is_some_and(|state| {
                    let is_running = state.running.load(std::sync::atomic::Ordering::SeqCst);
                    let active_turn_id = state
                        .active_turn_id
                        .lock()
                        .ok()
                        .and_then(|guard| guard.clone());
                    !is_running && active_turn_id.as_deref() != Some(parent_turn_id)
                });

        if !should_reactivate {
            return;
        }

        let followup_prompt =
            astrcode_runtime_agent_loop::child_delivery_reactivation_prompt(notification);
        match self
            .submit_prompt(&parent_session_id, followup_prompt)
            .await
        {
            Ok(_) => {
                log::info!(
                    "reactivated parent agent from child delivery: parentSession='{}', \
                     childAgent='{}', subRunId='{}'",
                    parent_session_id,
                    notification.child_ref.agent_id,
                    notification.child_ref.sub_run_id
                );
            },
            Err(error) => {
                log::warn!(
                    "failed to reactivate parent agent from child delivery: parentSession='{}', \
                     childAgent='{}', subRunId='{}', error='{}'",
                    parent_session_id,
                    notification.child_ref.agent_id,
                    notification.child_ref.sub_run_id,
                    error
                );
            },
        }
    }
}

#[async_trait]
impl ExecutionOrchestrationBoundary for AgentExecutionServiceHandle {
    async fn submit_prompt(
        &self,
        session_id: &str,
        text: String,
    ) -> std::result::Result<astrcode_core::PromptAccepted, AstrError> {
        AgentExecutionServiceHandle::submit_prompt(self, session_id, text)
            .await
            .map(|accepted| astrcode_core::PromptAccepted {
                turn_id: accepted.turn_id,
                session_id: accepted.session_id,
                branched_from_session_id: accepted.branched_from_session_id,
            })
            .map_err(service_error_to_astr)
    }

    async fn interrupt_session(&self, session_id: &str) -> std::result::Result<(), AstrError> {
        AgentExecutionServiceHandle::interrupt_session(self, session_id)
            .await
            .map_err(service_error_to_astr)
    }

    async fn execute_root_agent(
        &self,
        agent_id: String,
        task: String,
        context: Option<String>,
        context_overrides: Option<astrcode_core::SubagentContextOverrides>,
        working_dir: std::path::PathBuf,
    ) -> std::result::Result<astrcode_core::RootExecutionAccepted, AstrError> {
        AgentExecutionServiceHandle::execute_root_agent(
            self,
            agent_id,
            task,
            context,
            context_overrides,
            working_dir,
        )
        .await
        .map(|accepted| astrcode_core::RootExecutionAccepted {
            session_id: accepted.session_id,
            turn_id: accepted.turn_id,
            agent_id: accepted.agent_id,
        })
        .map_err(service_error_to_astr)
    }

    async fn launch_subagent(
        &self,
        params: SpawnAgentParams,
        ctx: &ToolContext,
    ) -> std::result::Result<SubRunResult, AstrError> {
        AgentExecutionServiceHandle::launch_subagent(self, params, ctx)
            .await
            .map_err(service_error_to_astr)
    }
}

#[async_trait]
impl LiveSubRunControlBoundary for AgentExecutionServiceHandle {
    async fn get_subrun_handle(
        &self,
        session_id: &str,
        sub_run_id: &str,
    ) -> std::result::Result<Option<SubRunHandle>, AstrError> {
        AgentExecutionServiceHandle::get_subrun_handle(self, session_id, sub_run_id)
            .await
            .map_err(service_error_to_astr)
    }

    async fn cancel_subrun(
        &self,
        session_id: &str,
        sub_run_id: &str,
    ) -> std::result::Result<(), AstrError> {
        AgentExecutionServiceHandle::cancel_subrun(self, session_id, sub_run_id)
            .await
            .map_err(service_error_to_astr)
    }

    async fn list_profiles(&self) -> std::result::Result<Vec<AgentProfile>, AstrError> {
        Ok(self
            .runtime
            .agent_profiles()
            .list_subagent_profiles()
            .into_iter()
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests;
