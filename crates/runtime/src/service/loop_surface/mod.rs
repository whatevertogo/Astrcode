use std::sync::Arc;

use astrcode_core::{CapabilityInvoker, HookHandler};
use astrcode_runtime_agent_loop::AgentLoop;
use astrcode_runtime_prompt::PromptDeclaration;
use astrcode_runtime_registry::CapabilityRouter;
use astrcode_runtime_skill_loader::{SkillCatalog, SkillSpec};

use crate::service::{RuntimeService, ServiceResult};

mod factory;
mod service;

pub(in crate::service) use factory::{
    LoopRuntimeDeps, LoopSurfaceInputs, build_agent_loop, build_scoped_agent_loop,
};
pub(crate) use service::LoopSurfaceService;

#[derive(Clone)]
pub struct LoopSurfaceSnapshot {
    pub capability_invokers: Vec<Arc<dyn CapabilityInvoker>>,
    pub prompt_declarations: Vec<PromptDeclaration>,
    pub base_skills: Vec<SkillSpec>,
    pub hook_handlers: Vec<Arc<dyn HookHandler>>,
}

/// `runtime-loop-surface` 的唯一 surface handle。
#[derive(Clone)]
pub struct LoopSurfaceServiceHandle {
    runtime: Arc<RuntimeService>,
}

impl LoopSurfaceServiceHandle {
    pub(super) fn new(runtime: Arc<RuntimeService>) -> Self {
        Self { runtime }
    }

    fn service(&self) -> LoopSurfaceService<'_> {
        LoopSurfaceService::new(self.runtime.as_ref())
    }

    pub async fn current_loop(&self) -> Arc<AgentLoop> {
        self.service().current_loop().await
    }

    pub async fn current_surface_snapshot(&self) -> LoopSurfaceSnapshot {
        self.service().current_surface_snapshot().await
    }

    pub async fn replace_surface(
        &self,
        capabilities: CapabilityRouter,
        prompt_declarations: Vec<PromptDeclaration>,
        skill_catalog: Arc<SkillCatalog>,
        hook_handlers: Vec<Arc<dyn HookHandler>>,
    ) -> ServiceResult<()> {
        self.service()
            .replace_surface(
                capabilities,
                prompt_declarations,
                skill_catalog,
                hook_handlers,
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::test_support::{TestEnvGuard, empty_capabilities};

    #[tokio::test]
    async fn rebuild_lock_keeps_inflight_readers_on_old_loop_snapshot() {
        let _guard = TestEnvGuard::new();
        let service =
            Arc::new(RuntimeService::from_capabilities(empty_capabilities()).expect("service"));
        let old_loop = service.loop_surface().current_loop().await;
        let (capabilities, prompt_declarations, skill_catalog, hook_handlers) = {
            let surface = service.surface.read().await;
            (
                surface.capabilities.clone(),
                surface.prompt_declarations.clone(),
                Arc::clone(&surface.skill_catalog),
                surface.hook_handlers.clone(),
            )
        };

        let rebuild_guard = service.rebuild_lock.lock().await;
        let service_for_replace = Arc::clone(&service);
        let replace_task = tokio::spawn(async move {
            service_for_replace
                .loop_surface()
                .replace_surface(
                    capabilities,
                    prompt_declarations,
                    skill_catalog,
                    hook_handlers,
                )
                .await
                .expect("surface replace should succeed");
        });

        tokio::task::yield_now().await;
        let loop_during_rebuild = service.loop_surface().current_loop().await;
        assert!(
            Arc::ptr_eq(&old_loop, &loop_during_rebuild),
            "rebuild lock held时读取方只能看到旧 loop 快照"
        );

        drop(rebuild_guard);
        replace_task.await.expect("replace task should finish");

        let loop_after_rebuild = service.loop_surface().current_loop().await;
        assert!(
            !Arc::ptr_eq(&old_loop, &loop_after_rebuild),
            "surface 替换完成后，新 turn 应看到新的 loop 快照"
        );
    }
}
