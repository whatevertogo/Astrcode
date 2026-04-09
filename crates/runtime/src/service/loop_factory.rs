//! # AgentLoop 组装器
//!
//! RuntimeService 只持有当前 loop 与 surface 快照；真正的 loop 构造与 scoped
//! policy 包装统一收敛到这里，避免 `mod.rs` 同时承担 façade 与装配职责。

use std::sync::Arc;

use astrcode_core::{AgentProfileCatalog, HookHandler, PolicyEngine};
use astrcode_runtime_agent_loop::{AgentLoop, ApprovalBroker, SubAgentPolicyEngine};
use astrcode_runtime_prompt::PromptDeclaration;
use astrcode_runtime_registry::CapabilityRouter;
use astrcode_runtime_skill_loader::SkillCatalog;

use super::RuntimeSurfaceState;
use crate::{
    config::{
        resolve_auto_compact_enabled, resolve_compact_keep_recent_turns,
        resolve_compact_threshold_percent, resolve_max_tool_concurrency,
        resolve_tool_result_max_bytes,
    },
    provider_factory::ConfigFileProviderFactory,
};

#[derive(Clone)]
pub(super) struct LoopSurfaceInputs {
    pub(super) capabilities: CapabilityRouter,
    pub(super) prompt_declarations: Vec<PromptDeclaration>,
    pub(super) skill_catalog: Arc<SkillCatalog>,
    pub(super) hook_handlers: Vec<Arc<dyn HookHandler>>,
    pub(super) prompt_builder: astrcode_runtime_prompt::LayeredPromptBuilder,
}

impl LoopSurfaceInputs {
    fn from_runtime_surface(surface: &RuntimeSurfaceState) -> Self {
        Self {
            capabilities: surface.capabilities.clone(),
            prompt_declarations: surface.prompt_declarations.clone(),
            skill_catalog: Arc::clone(&surface.skill_catalog),
            hook_handlers: surface.hook_handlers.clone(),
            prompt_builder: surface.prompt_builder.clone(),
        }
    }
}

#[derive(Clone)]
pub(super) struct LoopRuntimeDeps {
    policy: Arc<dyn PolicyEngine>,
    approval: Arc<dyn ApprovalBroker>,
    agent_profile_catalog: Option<Arc<dyn AgentProfileCatalog>>,
}

impl LoopRuntimeDeps {
    pub(super) fn new(
        policy: Arc<dyn PolicyEngine>,
        approval: Arc<dyn ApprovalBroker>,
        agent_profile_catalog: Option<Arc<dyn AgentProfileCatalog>>,
    ) -> Self {
        Self {
            policy,
            approval,
            agent_profile_catalog,
        }
    }
}

pub(super) fn build_agent_loop(
    surface: &RuntimeSurfaceState,
    active_profile: &str,
    runtime_config: &crate::config::RuntimeConfig,
    deps: LoopRuntimeDeps,
) -> Arc<AgentLoop> {
    build_agent_loop_from_parts(
        LoopSurfaceInputs::from_runtime_surface(surface),
        active_profile,
        runtime_config,
        deps,
    )
}

pub(super) fn build_agent_loop_from_parts(
    surface: LoopSurfaceInputs,
    active_profile: &str,
    runtime_config: &crate::config::RuntimeConfig,
    deps: LoopRuntimeDeps,
) -> Arc<AgentLoop> {
    let LoopRuntimeDeps {
        policy,
        approval,
        agent_profile_catalog,
    } = deps;
    let max_tool_concurrency = resolve_max_tool_concurrency(runtime_config);
    let LoopSurfaceInputs {
        capabilities,
        prompt_declarations,
        skill_catalog,
        hook_handlers,
        prompt_builder,
    } = surface;
    Arc::new(
        AgentLoop::from_capabilities_with_prompt_inputs(
            Arc::new(ConfigFileProviderFactory),
            capabilities,
            prompt_declarations,
            skill_catalog,
            agent_profile_catalog,
            prompt_builder,
        )
        .with_policy_profile(active_profile)
        .with_hook_handlers(hook_handlers)
        .with_max_tool_concurrency(max_tool_concurrency)
        .with_auto_compact_enabled(resolve_auto_compact_enabled(runtime_config))
        .with_compact_threshold_percent(resolve_compact_threshold_percent(runtime_config))
        .with_tool_result_max_bytes(resolve_tool_result_max_bytes(runtime_config))
        .with_compact_keep_recent_turns(resolve_compact_keep_recent_turns(runtime_config) as usize)
        .with_policy_engine(policy)
        .with_approval_broker(approval),
    )
}

pub(super) fn build_scoped_agent_loop(
    surface: LoopSurfaceInputs,
    active_profile: &str,
    runtime_config: &crate::config::RuntimeConfig,
    deps: LoopRuntimeDeps,
) -> Arc<AgentLoop> {
    let LoopRuntimeDeps {
        policy,
        approval,
        agent_profile_catalog,
    } = deps;
    let scoped_policy = Arc::new(SubAgentPolicyEngine::new(
        policy,
        surface.capabilities.tool_names().into_iter().collect(),
    ));
    build_agent_loop_from_parts(
        surface,
        active_profile,
        runtime_config,
        LoopRuntimeDeps::new(scoped_policy, approval, agent_profile_catalog),
    )
}
