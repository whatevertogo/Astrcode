use std::sync::Arc;

use astrcode_core::HookHandler;
use astrcode_runtime_agent_loop::AgentLoop;
use astrcode_runtime_prompt::PromptDeclaration;
use astrcode_runtime_registry::CapabilityRouter;
use astrcode_runtime_skill_loader::SkillCatalog;

use super::{
    RuntimeService, RuntimeSurfaceState, ServiceResult,
    loop_factory::{LoopRuntimeDeps, build_agent_loop},
};

/// 能力面管理器：集中处理 capability surface 与 loop 热替换。
///
/// 之所以抽成独立组件，是为了把 RuntimeService 从“状态容器 + 业务细节”
/// 收敛为“门面 + 编排”，降低后续扩展热重载策略时的修改面。
pub(super) struct CapabilityManager<'a> {
    runtime: &'a RuntimeService,
}

impl<'a> CapabilityManager<'a> {
    pub(super) fn new(runtime: &'a RuntimeService) -> Self {
        Self { runtime }
    }

    pub(super) async fn current_loop(&self) -> Arc<AgentLoop> {
        self.runtime.loop_.read().await.clone()
    }

    pub(super) async fn replace_surface(
        &self,
        capabilities: CapabilityRouter,
        prompt_declarations: Vec<PromptDeclaration>,
        skill_catalog: Arc<SkillCatalog>,
        hook_handlers: Vec<Arc<dyn HookHandler>>,
    ) -> ServiceResult<()> {
        let _guard = self.runtime.rebuild_lock.lock().await;
        let runtime_config = {
            let config = self.runtime.config.lock().await;
            config.runtime.clone()
        };
        let next_surface = RuntimeSurfaceState {
            capabilities,
            prompt_declarations,
            skill_catalog,
            hook_handlers,
        };
        let next_loop = build_agent_loop(
            &next_surface,
            &runtime_config,
            LoopRuntimeDeps::new(
                Arc::clone(&self.runtime.policy),
                Arc::clone(&self.runtime.approval),
                Some(self.runtime.agent_profile_catalog()),
            ),
        );
        *self.runtime.loop_.write().await = next_loop;
        *self.runtime.surface.write().await = next_surface;
        Ok(())
    }
}
