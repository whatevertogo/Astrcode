use std::sync::Arc;

use astrcode_runtime_agent_tool::RunAgentParams;
use astrcode_runtime_execution::{
    PreparedAgentExecution, ScopedExecutionSurface, prepare_scoped_agent_execution,
};

use super::root::AgentExecutionServiceHandle;
use crate::service::{ServiceResult, loop_factory::build_scoped_agent_loop};

impl AgentExecutionServiceHandle {
    pub(super) async fn snapshot_execution_surface(&self) -> ScopedExecutionSurface {
        let surface = self.runtime.surface.read().await;
        let runtime_config = self.runtime.config.lock().await.runtime.clone();
        ScopedExecutionSurface {
            capabilities: surface.capabilities.clone(),
            prompt_declarations: surface.prompt_declarations.clone(),
            skill_catalog: Arc::clone(&surface.skill_catalog),
            hook_handlers: surface.hook_handlers.clone(),
            runtime_config,
        }
    }

    pub(super) fn prepare_scoped_execution(
        &self,
        invocation_kind: astrcode_core::InvocationKind,
        profile: &astrcode_core::AgentProfile,
        params: &RunAgentParams,
        surface: ScopedExecutionSurface,
        parent_state: Option<&astrcode_core::AgentState>,
    ) -> ServiceResult<PreparedAgentExecution> {
        prepare_scoped_agent_execution(
            invocation_kind,
            profile,
            params,
            surface,
            parent_state,
            |capabilities, prompt_declarations, skill_catalog, hook_handlers, runtime_config| {
                build_scoped_agent_loop(
                    capabilities,
                    prompt_declarations,
                    skill_catalog,
                    hook_handlers,
                    runtime_config,
                    Arc::clone(&self.runtime.policy),
                    Arc::clone(&self.runtime.approval),
                )
            },
        )
        .map_err(crate::service::ServiceError::from)
    }
}
