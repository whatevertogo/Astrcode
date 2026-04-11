use std::sync::Arc;

use astrcode_core::SpawnAgentParams;
use astrcode_runtime_agent_loop::AgentLoop;
use astrcode_runtime_execution::{
    AgentExecutionRequest, PreparedAgentExecution, ScopedExecutionSurface,
    prepare_scoped_agent_execution,
};

use super::root::AgentExecutionServiceHandle;
use crate::service::{
    ServiceResult,
    loop_surface::{LoopRuntimeDeps, LoopSurfaceInputs, build_scoped_agent_loop},
};

impl AgentExecutionServiceHandle {
    pub(super) async fn snapshot_execution_surface(
        &self,
    ) -> ScopedExecutionSurface<Arc<astrcode_runtime_skill_loader::SkillCatalog>> {
        let surface = self.runtime.surface.read().await;
        let config = self.runtime.config.lock().await.clone();
        ScopedExecutionSurface {
            capabilities: surface.capabilities.clone(),
            prompt_declarations: surface.prompt_declarations.clone(),
            skill_catalog: Arc::clone(&surface.skill_catalog),
            hook_handlers: surface.hook_handlers.clone(),
            prompt_builder: surface.prompt_builder.clone(),
            active_profile: config.active_profile,
            runtime_config: config.runtime,
        }
    }

    pub(super) async fn prepare_scoped_execution(
        &self,
        invocation_kind: astrcode_core::InvocationKind,
        profile: &astrcode_core::AgentProfile,
        params: &SpawnAgentParams,
        surface: ScopedExecutionSurface<Arc<astrcode_runtime_skill_loader::SkillCatalog>>,
        parent_state: Option<&astrcode_core::AgentState>,
    ) -> ServiceResult<PreparedAgentExecution<Arc<AgentLoop>>> {
        let request = AgentExecutionRequest::from_spawn_agent_params(params, None);
        // 从 surface 读取 factory 用于子代理 loop 组装
        let factory = self.runtime.surface.read().await.factory.clone();
        self.prepare_scoped_execution_request(
            invocation_kind,
            profile,
            request,
            surface,
            parent_state,
            factory,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare_scoped_execution_request(
        &self,
        invocation_kind: astrcode_core::InvocationKind,
        profile: &astrcode_core::AgentProfile,
        request: AgentExecutionRequest,
        surface: ScopedExecutionSurface<Arc<astrcode_runtime_skill_loader::SkillCatalog>>,
        parent_state: Option<&astrcode_core::AgentState>,
        factory: astrcode_runtime_agent_loop::DynProviderFactory,
    ) -> ServiceResult<PreparedAgentExecution<Arc<AgentLoop>>> {
        prepare_scoped_agent_execution(
            invocation_kind,
            profile,
            &request,
            surface,
            parent_state,
            |capabilities,
             prompt_declarations,
             skill_catalog,
             hook_handlers,
             prompt_builder,
             active_profile,
             runtime_config| {
                build_scoped_agent_loop(
                    LoopSurfaceInputs {
                        capabilities,
                        prompt_declarations,
                        skill_catalog,
                        hook_handlers,
                        prompt_builder,
                        factory: factory.clone(),
                    },
                    active_profile,
                    runtime_config,
                    LoopRuntimeDeps::new(
                        Arc::clone(&self.runtime.policy),
                        Arc::clone(&self.runtime.approval),
                        Some(self.runtime.agent_profile_catalog()),
                    ),
                )
            },
        )
        .map_err(crate::service::ServiceError::from)
    }
}
