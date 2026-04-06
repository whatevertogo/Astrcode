//! Agent / tool 执行 façade 的内部实现按用例拆分：
//! - `surface`：读取当前 runtime surface 并构造 scoped execution 输入
//! - `status`：sub-run 状态查询
//! - `subagent`：作为工具执行子 agent
//! - `root`：根执行入口

mod root;
mod status;
mod subagent;
mod surface;

use std::sync::{Arc, RwLock as StdRwLock, Weak};

use astrcode_core::{AgentProfile, AstrError, Result, SubRunResult, ToolContext};
use astrcode_runtime_agent_tool::{AgentProfileCatalog, SpawnAgentParams, SubAgentExecutor};
use async_trait::async_trait;
pub use root::{
    AgentExecutionServiceHandle, AgentProfileSummary, ToolExecutionServiceHandle, ToolSummary,
};

use crate::service::{RuntimeService, ServiceError};

/// bootstrap 阶段使用的延迟执行器桥。
///
/// builtin router 在 `RuntimeService` 创建前就要注册 `spawnAgent`，因此这里先占位，
/// 等 service 创建完成后再绑定真实 runtime。
#[derive(Default)]
pub(crate) struct DeferredSubAgentExecutor {
    runtime: StdRwLock<Option<Weak<RuntimeService>>>,
}

impl DeferredSubAgentExecutor {
    pub(crate) fn bind(&self, runtime: &Arc<RuntimeService>) {
        let mut guard = self
            .runtime
            .write()
            .expect("sub-agent executor binding lock should not be poisoned");
        *guard = Some(Arc::downgrade(runtime));
    }

    fn runtime(&self) -> Result<Arc<RuntimeService>> {
        let guard = self
            .runtime
            .read()
            .expect("sub-agent executor binding lock should not be poisoned");
        let Some(runtime) = guard.as_ref().and_then(Weak::upgrade) else {
            return Err(AstrError::Internal(
                "spawnAgent executor is not bound to runtime service yet".to_string(),
            ));
        };
        Ok(runtime)
    }
}

#[async_trait]
impl SubAgentExecutor for DeferredSubAgentExecutor {
    async fn launch(&self, params: SpawnAgentParams, ctx: &ToolContext) -> Result<SubRunResult> {
        let runtime = self.runtime()?;
        runtime
            .agent_execution_service()
            .launch_subagent(params, ctx)
            .await
            .map_err(service_error_to_astr)
    }
}

impl RuntimeService {
    pub fn agent_execution_service(self: &Arc<Self>) -> AgentExecutionServiceHandle {
        AgentExecutionServiceHandle {
            runtime: Arc::clone(self),
        }
    }

    pub fn tool_execution_service(self: &Arc<Self>) -> ToolExecutionServiceHandle {
        ToolExecutionServiceHandle {
            runtime: Arc::clone(self),
        }
    }
}

#[async_trait]
impl SubAgentExecutor for root::AgentExecutionServiceHandle {
    async fn launch(&self, params: SpawnAgentParams, ctx: &ToolContext) -> Result<SubRunResult> {
        self.launch_subagent(params, ctx)
            .await
            .map_err(service_error_to_astr)
    }
}

impl AgentProfileCatalog for root::AgentExecutionServiceHandle {
    fn list_subagent_profiles(&self) -> Vec<AgentProfile> {
        self.runtime
            .agent_profiles()
            .list_subagent_profiles()
            .into_iter()
            .cloned()
            .collect()
    }
}

fn service_error_to_astr(error: ServiceError) -> AstrError {
    match error {
        ServiceError::NotFound(message)
        | ServiceError::Conflict(message)
        | ServiceError::InvalidInput(message) => AstrError::Validation(message),
        ServiceError::Internal(error) => error,
    }
}

#[cfg(test)]
mod tests;
