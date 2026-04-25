//! # 服务器运行时组合根
//!
//! 由 server 装配 adapter 与运行时 owner。
//! plugin/provider/resource 生效事实统一来自 plugin-host active snapshot，
//! 组合根只负责把 server-owned bridge、host-session 与 agent-runtime 接起来。

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use astrcode_adapter_storage::session::FileSystemSessionRepository;
use astrcode_adapter_tools::builtin_tools::tool_search::ToolSearchIndex;
use astrcode_core::SkillCatalog;
use astrcode_governance_contract::{
    ActionPolicies, ActionPolicyEffect, ApprovalDefault, ApprovalRequest, GovernanceModeSpec,
};
use astrcode_host_session::{EventStore, SessionCatalog, SubAgentExecutor};
use astrcode_plugin_host::{
    BuiltinHookRegistry, PluginActiveSnapshot, PluginDescriptor, PluginRegistry,
    ProviderContributionCatalog, ResourceCatalog, builtin_collaboration_tools_descriptor,
    builtin_modes_descriptor, builtin_openai_provider_descriptor, builtin_tools_descriptor,
    resources_discover,
};
use astrcode_runtime_contract::hooks::{HookEffect, HookEventPayload};
use astrcode_support::hostpaths::resolve_home_dir;

use super::{
    super::{
        agent_api::ServerAgentApi,
        agent_runtime_bridge::{ServerAgentRuntimeBuildInput, build_server_agent_runtime_bundle},
    },
    builtin_plugins::{
        BUILTIN_GOVERNANCE_MODES_PLUGIN_ID, BUILTIN_PLANNING_PLUGIN_ID,
        EXTERNAL_PLUGIN_MODES_PLUGIN_ID, PERMISSION_PROVIDER_REQUEST_HOOK_ID,
        PERMISSION_TOOL_CALL_HOOK_ID, PLANNING_INPUT_HOOK_ID, builtin_composer_plugin_descriptor,
        builtin_permission_descriptor, builtin_planning_descriptor, descriptor_modes,
        descriptor_modes_for_plugin_ids, split_builtin_planning_modes,
    },
    capabilities::{
        CapabilitySurfaceSync, build_agent_tool_invokers, build_core_tool_invokers,
        build_planning_tool_invokers, build_skill_catalog, build_stable_local_invokers,
        sync_external_tool_search_index,
    },
    deps::core::{
        self, AstrError, CapabilityInvoker, Config, ResolvedRuntimeConfig, Result,
        resolve_runtime_config,
    },
    governance::{GovernanceBuildInput, build_server_governance_service},
    mcp::{bootstrap_mcp_manager, build_mcp_service, warmup_mcp_manager},
    plugins::{PluginBootstrapResult, bootstrap_plugins_with_skill_root},
    providers::{build_config_service, build_llm_provider, build_profile_resolution_service},
    runtime_coordinator::RuntimeCoordinator,
    watch::{bootstrap_profile_watch_runtime, build_watch_service},
};
use crate::{
    agent_control_bridge::ServerAgentControlPort,
    config_service_bridge::ServerConfigService,
    governance_service::ServerGovernanceService,
    hook_adapter::PluginHostHookDispatcher,
    mcp_service::ServerMcpService,
    mode_catalog_service::ServerModeCatalog,
    profile_service::ServerProfileService,
    runtime_owner_bridge::{
        ServerRuntimeObservability, ServerTaskRegistry, builtin_server_mode_specs,
    },
    session_runtime_owner_bridge::{
        ServerAgentControlLimits, ServerSessionRuntimeBootstrapInput, bootstrap_session_runtime,
    },
    watch_service::WatchService,
};

/// 服务器运行时：组合根输出。
pub struct ServerRuntime {
    pub agent_api: Arc<ServerAgentApi>,
    #[allow(dead_code)]
    pub agent_control: Arc<dyn ServerAgentControlPort>,
    pub config: Arc<ServerConfigService>,
    pub session_catalog: Arc<SessionCatalog>,
    #[allow(dead_code)]
    pub profiles: Arc<ServerProfileService>,
    #[allow(dead_code)]
    pub subagent_executor: Arc<dyn SubAgentExecutor>,
    pub mcp_service: Arc<ServerMcpService>,
    pub skill_catalog: Arc<dyn SkillCatalog>,
    pub resource_catalog: Arc<std::sync::RwLock<ResourceCatalog>>,
    pub mode_catalog: Arc<ServerModeCatalog>,
    pub governance: Arc<ServerGovernanceService>,
    pub handles: Arc<ServerRuntimeHandles>,
}

pub struct ServerRuntimeHandles {
    // Why: server 集成测试需要直接操纵底层 session-runtime，避免把原始状态访问重新暴露给
    // application 端口；生产路径只把它当作资源守卫持有。
    pub(crate) _session_runtime_guard: Arc<dyn std::any::Any + Send + Sync>,
    pub(crate) _session_runtime_test_support:
        Arc<dyn crate::session_runtime_owner_bridge::ServerRuntimeTestSupport>,
    _profile_watch_runtime: Option<super::watch::ProfileWatchRuntime>,
    _mcp_warmup_runtime: McpWarmupRuntime,
}

/// 组合根的可覆盖选项。
///
/// 生产环境使用默认值；测试环境通过显式 sandbox 注入目录，避免再依赖
/// 进程级环境变量和全局锁做隔离。
#[derive(Debug, Clone)]
pub struct ServerBootstrapOptions {
    pub home_dir: Option<PathBuf>,
    pub working_dir: Option<PathBuf>,
    pub plugin_search_paths: Option<Vec<PathBuf>>,
    pub enable_profile_watch: bool,
    pub watch_service_override: Option<Arc<WatchService>>,
}

impl Default for ServerBootstrapOptions {
    fn default() -> Self {
        Self {
            home_dir: None,
            working_dir: None,
            plugin_search_paths: None,
            enable_profile_watch: true,
            watch_service_override: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ServerBootstrapPaths {
    pub home_dir: PathBuf,
    pub config_path: PathBuf,
    pub mcp_approvals_path: PathBuf,
    pub plugin_skill_root: PathBuf,
    pub projects_root: PathBuf,
    pub plugin_search_paths: Vec<PathBuf>,
}

struct McpWarmupRuntime {
    task: tokio::task::JoinHandle<()>,
}

impl Drop for McpWarmupRuntime {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl ServerBootstrapPaths {
    fn from_options(options: &ServerBootstrapOptions) -> Result<Self> {
        let home_dir = match &options.home_dir {
            Some(home_dir) => home_dir.clone(),
            None => resolve_home_dir()?,
        };
        let astrcode_dir = home_dir.join(".astrcode");
        Ok(Self {
            config_path: astrcode_dir.join("config.json"),
            mcp_approvals_path: astrcode_dir.join("mcp-approvals.json"),
            plugin_skill_root: astrcode_dir.join("runtime").join("plugin-skills"),
            projects_root: astrcode_dir.join("projects"),
            plugin_search_paths: resolve_plugin_search_paths(
                home_dir.as_path(),
                options.plugin_search_paths.clone(),
            ),
            home_dir,
        })
    }
}

/// 构建服务器运行时组合根。
///
/// 能力来源拆为两层：
/// - 稳定本地能力：core builtin tools + agent tools
/// - 动态外部能力：MCP + plugin
pub async fn bootstrap_server_runtime() -> Result<ServerRuntime> {
    bootstrap_server_runtime_with_options(ServerBootstrapOptions::default()).await
}

pub async fn bootstrap_server_runtime_with_options(
    options: ServerBootstrapOptions,
) -> Result<ServerRuntime> {
    let paths = ServerBootstrapPaths::from_options(&options)?;
    let config_service = build_config_service(paths.config_path.clone())?;
    let working_dir = match options.working_dir {
        Some(working_dir) => working_dir,
        None => std::env::current_dir()
            .map_err(|error| AstrError::io("failed to resolve server working directory", error))?,
    };
    let resolved_config = config_service
        .load_overlayed_config(Some(working_dir.as_path()))
        .map_err(|error| AstrError::Internal(error.to_string()))?;
    let agent_loader =
        astrcode_adapter_agents::AgentProfileLoader::new_with_home_dir(paths.home_dir.as_path());
    let mcp_manager =
        bootstrap_mcp_manager(working_dir.as_path(), paths.mcp_approvals_path.clone()).await?;

    // plugin + MCP 是动态外部事实源，需要先完成装配，随后再把它们注入
    // tool_search / skill catalog，避免启动态与 reload 态出现两套事实。
    let tool_search_index = Arc::new(ToolSearchIndex::new());
    let PluginBootstrapResult {
        invokers: plugin_invokers,
        skills: plugin_skills,
        modes: plugin_modes,
        registry: plugin_registry,
        managed_components: managed_plugin_components,
        search_paths: plugin_search_paths,
        resource_catalog: plugin_resource_catalog,
        descriptors: plugin_descriptors,
    } = bootstrap_plugins_with_skill_root(
        paths.plugin_search_paths.clone(),
        paths.plugin_skill_root.clone(),
    )
    .await;
    let mcp_invokers = mcp_manager.current_surface().await.capability_invokers;

    let mut external_dynamic_invokers: Vec<Arc<dyn CapabilityInvoker>> = mcp_invokers.clone();
    external_dynamic_invokers.extend(plugin_invokers.clone());
    sync_external_tool_search_index(&tool_search_index, &external_dynamic_invokers);

    // core builtin tools：工具定义本身是 builtin + stable；
    // 其中 `Skill` 工具消费的 catalog 可以包含 builtin / mcp / plugin 三类 skill。
    let skill_catalog = build_skill_catalog(
        paths.home_dir.as_path(),
        plugin_skills,
        &plugin_resource_catalog,
    );
    let skill_catalog_bridge: Arc<dyn SkillCatalog> = skill_catalog.clone();
    let builtin_mode_specs = builtin_server_mode_specs()?;
    let builtin_mode_specs_for_reload = builtin_mode_specs.clone();
    let (governance_mode_specs, planning_mode_specs) =
        split_builtin_planning_modes(builtin_mode_specs.clone());
    let core_tool_invokers =
        build_core_tool_invokers(Arc::clone(&tool_search_index), skill_catalog.clone())?;
    let planning_tool_invokers = build_planning_tool_invokers()?;
    let core_tool_specs = core_tool_invokers
        .iter()
        .map(|invoker| invoker.capability_spec())
        .collect::<Vec<_>>();
    let planning_tool_specs = planning_tool_invokers
        .iter()
        .map(|invoker| invoker.capability_spec())
        .collect::<Vec<_>>();
    let active_plugin_descriptors = build_server_plugin_contribution_descriptors(
        &core_tool_invokers,
        &planning_tool_invokers,
        &mcp_invokers,
        governance_mode_specs,
        planning_mode_specs,
        plugin_modes,
        plugin_descriptors.clone(),
    );
    let mut builtin_hook_registry = BuiltinHookRegistry::new();
    register_builtin_planning_hooks(&mut builtin_hook_registry);
    register_builtin_permission_hooks(&mut builtin_hook_registry, builtin_mode_specs.clone());
    let builtin_hook_registry = Arc::new(builtin_hook_registry);
    let plugin_host_reload = reload_server_plugin_host_snapshot(
        plugin_registry.as_ref(),
        active_plugin_descriptors,
        builtin_hook_registry.as_ref(),
    )?;
    log::info!(
        "plugin-host bridge activated snapshot {} with {} plugins",
        plugin_host_reload.snapshot.snapshot_id,
        plugin_host_reload.snapshot.plugin_ids.len()
    );
    let plugin_resource_catalog_state =
        Arc::new(std::sync::RwLock::new(plugin_host_reload.resources.clone()));
    let provider_catalog = Arc::new(std::sync::RwLock::new(
        plugin_host_reload.provider_catalog.clone(),
    ));

    let observability = ServerRuntimeObservability::new();
    let task_registry = ServerTaskRegistry::new();
    let mode_catalog = ServerModeCatalog::from_mode_specs(
        plugin_host_reload.builtin_modes.clone(),
        plugin_host_reload.plugin_modes.clone(),
    )?;
    let plugin_hook_dispatcher = Arc::new(PluginHostHookDispatcher::new(
        plugin_host_reload.snapshot.snapshot_id.clone(),
        Arc::new(plugin_host_reload.snapshot.hook_bindings.clone()),
        Arc::clone(&builtin_hook_registry),
    ));
    let runtime_hook_dispatcher: Arc<dyn astrcode_agent_runtime::HookDispatcher> =
        plugin_hook_dispatcher.clone();
    let owner_hook_dispatcher: Arc<dyn astrcode_host_session::HookDispatch> =
        plugin_hook_dispatcher.clone();
    let hook_snapshot_id = {
        let plugin_hook_dispatcher = Arc::clone(&plugin_hook_dispatcher);
        Arc::new(move || plugin_hook_dispatcher.active_snapshot_id())
            as Arc<dyn Fn() -> String + Send + Sync>
    };

    let event_store: Arc<dyn EventStore> = Arc::new(
        FileSystemSessionRepository::new_with_projects_root(paths.projects_root.clone()),
    );
    let session_catalog = Arc::new(SessionCatalog::new(Arc::clone(&event_store)));
    // 初始 capability surface 先用“当前可立即装配的能力面”启动：
    // core builtin tools + 当前 external 动态能力。
    // agent tools 要等 agent_service 准备好后再并入稳定本地层。
    let mut initial_router_invokers = core_tool_invokers.clone();
    initial_router_invokers.extend(planning_tool_invokers.clone());
    initial_router_invokers.extend(external_dynamic_invokers.clone());
    let session_runtime = bootstrap_session_runtime(ServerSessionRuntimeBootstrapInput {
        capability_invokers: initial_router_invokers,
        llm_provider: build_llm_provider(
            config_service.clone(),
            working_dir.clone(),
            Arc::clone(&provider_catalog),
            Some(owner_hook_dispatcher.clone()),
        ),
        session_catalog: Arc::clone(&session_catalog),
        mode_catalog: Arc::clone(&mode_catalog),
        agent_limits: resolve_agent_control_limits(&resolved_config),
        hook_dispatcher: Some(runtime_hook_dispatcher),
        owner_hook_dispatcher: Some(owner_hook_dispatcher),
        hook_snapshot_id,
    })?;
    let profiles = build_profile_resolution_service(agent_loader.clone())?;
    let watch_service = match options.watch_service_override.clone() {
        Some(service) => service,
        None => build_watch_service(agent_loader)
            .map_err(|error| AstrError::Internal(error.to_string()))?,
    };
    let agent_runtime = build_server_agent_runtime_bundle(ServerAgentRuntimeBuildInput {
        agent_kernel: session_runtime.agent_kernel.clone(),
        agent_sessions: session_runtime.agent_sessions.clone(),
        app_sessions: session_runtime.app_sessions.clone(),
        agent_control: session_runtime.agent_control.clone(),
        config_service: config_service.clone(),
        profiles: profiles.clone(),
        mode_catalog: mode_catalog.clone(),
        task_registry: task_registry.clone(),
        observability: observability.clone(),
    });

    // agent 四工具依赖 agent_service，必须在 kernel/session_runtime 之后单独注册。
    // 组装完成后，稳定本地层 = core builtin tools + agent tools。
    let agent_tool_invokers = build_agent_tool_invokers(
        Arc::clone(&agent_runtime.subagent_executor),
        Arc::clone(&agent_runtime.collaboration_executor),
    )?;
    let collaboration_tool_specs = agent_tool_invokers
        .iter()
        .map(|invoker| invoker.capability_spec())
        .collect::<Vec<_>>();
    let mut core_and_planning_tool_invokers = core_tool_invokers;
    core_and_planning_tool_invokers.extend(planning_tool_invokers);
    let stable_local_invokers =
        build_stable_local_invokers(core_and_planning_tool_invokers, agent_tool_invokers);
    let capability_sync = CapabilitySurfaceSync::new(
        session_runtime.capability_surface.clone(),
        stable_local_invokers,
        Arc::clone(&tool_search_index),
    );
    capability_sync.apply_external_invokers(external_dynamic_invokers.clone())?;
    let coordinator = Arc::new(RuntimeCoordinator::new(
        Arc::new(super::governance::AppRuntimeHandle),
        plugin_registry.clone(),
        capability_sync.current_capabilities(),
    ));
    let mcp_service = build_mcp_service(
        config_service.clone(),
        working_dir.clone(),
        mcp_manager.clone(),
        capability_sync.clone(),
    );
    let governance = build_server_governance_service(GovernanceBuildInput {
        sessions: session_runtime.sessions.clone(),
        config_service: config_service.clone(),
        coordinator,
        task_registry,
        observability,
        mcp_manager: Arc::clone(&mcp_manager),
        capability_sync: capability_sync.clone(),
        skill_catalog,
        resource_catalog: Arc::clone(&plugin_resource_catalog_state),
        provider_catalog,
        plugin_search_paths: plugin_search_paths.clone(),
        plugin_skill_root: paths.plugin_skill_root.clone(),
        managed_plugin_components,
        working_dir: working_dir.clone(),
        mode_catalog: Some(Arc::clone(&mode_catalog)),
        core_tool_specs,
        planning_tool_specs,
        collaboration_tool_specs,
        builtin_mode_specs: builtin_mode_specs_for_reload,
        builtin_hook_registry: Arc::clone(&builtin_hook_registry),
        hook_dispatcher: Arc::clone(&plugin_hook_dispatcher),
    });
    let profile_watch_runtime = if options.enable_profile_watch {
        Some(
            bootstrap_profile_watch_runtime(
                Arc::clone(&session_catalog),
                Arc::clone(&profiles),
                Arc::clone(&watch_service),
            )
            .await
            .map_err(|error| AstrError::Internal(error.to_string()))?,
        )
    } else {
        None
    };
    let mcp_warmup_runtime = McpWarmupRuntime {
        task: tokio::spawn(warmup_mcp_manager(
            Arc::clone(&mcp_manager),
            Arc::clone(&config_service),
            working_dir,
            capability_sync,
            plugin_invokers,
        )),
    };

    Ok(ServerRuntime {
        agent_api: agent_runtime.agent_api,
        agent_control: agent_runtime.agent_control,
        config: config_service,
        session_catalog,
        profiles,
        subagent_executor: agent_runtime.subagent_executor,
        mcp_service,
        skill_catalog: skill_catalog_bridge,
        resource_catalog: Arc::clone(&plugin_resource_catalog_state),
        mode_catalog,
        governance,
        handles: Arc::new(ServerRuntimeHandles {
            _session_runtime_guard: session_runtime.keepalive,
            _session_runtime_test_support: session_runtime.test_support,
            _profile_watch_runtime: profile_watch_runtime,
            _mcp_warmup_runtime: mcp_warmup_runtime,
        }),
    })
}

fn build_server_plugin_contribution_descriptors(
    core_tool_invokers: &[Arc<dyn CapabilityInvoker>],
    planning_tool_invokers: &[Arc<dyn CapabilityInvoker>],
    mcp_invokers: &[Arc<dyn CapabilityInvoker>],
    governance_modes: Vec<GovernanceModeSpec>,
    planning_modes: Vec<GovernanceModeSpec>,
    plugin_modes: Vec<GovernanceModeSpec>,
    mut external_descriptors: Vec<PluginDescriptor>,
) -> Vec<PluginDescriptor> {
    let mut descriptors = vec![
        builtin_openai_provider_descriptor(),
        builtin_modes_descriptor(
            BUILTIN_GOVERNANCE_MODES_PLUGIN_ID,
            "Builtin Governance Modes",
            governance_modes,
        ),
        builtin_permission_descriptor(),
        builtin_planning_descriptor(
            planning_tool_invokers
                .iter()
                .map(|invoker| invoker.capability_spec())
                .collect(),
            planning_modes,
        ),
        builtin_modes_descriptor(
            EXTERNAL_PLUGIN_MODES_PLUGIN_ID,
            "External Plugin Modes",
            plugin_modes,
        ),
        builtin_composer_plugin_descriptor(),
        builtin_tools_descriptor(
            "builtin-core-tools",
            "Builtin Core Tools",
            core_tool_invokers
                .iter()
                .map(|invoker| invoker.capability_spec())
                .collect(),
        ),
        builtin_tools_descriptor(
            "external-mcp-tools",
            "External MCP Tools",
            mcp_invokers
                .iter()
                .map(|invoker| invoker.capability_spec())
                .collect(),
        ),
        builtin_collaboration_tools_descriptor(),
    ];
    descriptors.append(&mut external_descriptors);
    descriptors
}

fn register_builtin_planning_hooks(registry: &mut BuiltinHookRegistry) {
    registry.on_input(PLANNING_INPUT_HOOK_ID, |_context, payload| async move {
        Ok(planning_input_effects(payload))
    });
}

fn register_builtin_permission_hooks(
    registry: &mut BuiltinHookRegistry,
    modes: Vec<GovernanceModeSpec>,
) {
    let policy_by_mode = Arc::new(
        modes
            .into_iter()
            .map(|mode| (mode.id.as_str().to_string(), mode.action_policies))
            .collect::<BTreeMap<_, _>>(),
    );

    let tool_policy_by_mode = Arc::clone(&policy_by_mode);
    registry.on_tool_call(PERMISSION_TOOL_CALL_HOOK_ID, move |_context, payload| {
        let policy_by_mode = Arc::clone(&tool_policy_by_mode);
        async move { Ok(permission_tool_call_effects(payload, &policy_by_mode)) }
    });

    registry.on_before_provider_request(
        PERMISSION_PROVIDER_REQUEST_HOOK_ID,
        move |_context, payload| {
            let policy_by_mode = Arc::clone(&policy_by_mode);
            async move {
                Ok(permission_provider_request_effects(
                    payload,
                    &policy_by_mode,
                ))
            }
        },
    );
}

fn permission_tool_call_effects(
    payload: HookEventPayload,
    policy_by_mode: &BTreeMap<String, ActionPolicies>,
) -> Vec<HookEffect> {
    let HookEventPayload::ToolCall {
        session_id,
        turn_id,
        tool_call_id,
        tool_name,
        args,
        capability_spec,
        current_mode,
        ..
    } = payload
    else {
        return vec![HookEffect::Continue];
    };
    let Some(policy_effect) = current_mode
        .as_deref()
        .map(|mode| policy_effect_for_mode(mode, policy_by_mode))
    else {
        return vec![HookEffect::Continue];
    };

    match policy_effect {
        ActionPolicyEffect::Allow => vec![HookEffect::Continue],
        ActionPolicyEffect::Deny if should_gate_tool(&capability_spec) => {
            vec![HookEffect::BlockToolResult {
                tool_call_id,
                reason: format!("tool '{tool_name}' is denied by the active mode policy"),
            }]
        },
        ActionPolicyEffect::Deny => vec![HookEffect::Continue],
        ActionPolicyEffect::Ask if should_gate_tool(&capability_spec) => {
            let approval = ApprovalRequest {
                request_id: tool_call_id.clone(),
                session_id,
                turn_id,
                capability: (*capability_spec).clone(),
                payload: args,
                prompt: format!("Allow tool '{tool_name}'?"),
                default: ApprovalDefault::Deny,
                metadata: serde_json::Value::Null,
            };
            vec![HookEffect::RequireApproval {
                request_id: approval.request_id,
                reason: approval.prompt,
            }]
        },
        ActionPolicyEffect::Ask => vec![HookEffect::Continue],
    }
}

fn permission_provider_request_effects(
    payload: HookEventPayload,
    policy_by_mode: &BTreeMap<String, ActionPolicies>,
) -> Vec<HookEffect> {
    let HookEventPayload::BeforeProviderRequest {
        current_mode,
        model_ref,
        ..
    } = payload
    else {
        return vec![HookEffect::Continue];
    };
    match current_mode
        .as_deref()
        .map(|mode| provider_policy_effect_for_mode(mode, policy_by_mode))
    {
        Some(ActionPolicyEffect::Deny) => vec![HookEffect::DenyProviderRequest {
            reason: format!("model request for '{model_ref}' is denied by the active mode policy"),
        }],
        Some(ActionPolicyEffect::Ask) => vec![HookEffect::RequireApproval {
            request_id: format!("provider:{model_ref}"),
            reason: format!("model request for '{model_ref}' requires approval"),
        }],
        Some(ActionPolicyEffect::Allow) | None => vec![HookEffect::Continue],
    }
}

fn provider_policy_effect_for_mode(
    mode_id: &str,
    policy_by_mode: &BTreeMap<String, ActionPolicies>,
) -> ActionPolicyEffect {
    policy_by_mode
        .get(mode_id)
        .map(|policy| policy.default_effect)
        .unwrap_or(ActionPolicyEffect::Allow)
}

fn policy_effect_for_mode(
    mode_id: &str,
    policy_by_mode: &BTreeMap<String, ActionPolicies>,
) -> ActionPolicyEffect {
    let Some(policy) = policy_by_mode.get(mode_id) else {
        return ActionPolicyEffect::Allow;
    };
    if policy
        .rules
        .iter()
        .any(|rule| matches!(rule.effect, ActionPolicyEffect::Deny))
    {
        return ActionPolicyEffect::Deny;
    }
    if policy
        .rules
        .iter()
        .any(|rule| matches!(rule.effect, ActionPolicyEffect::Ask))
    {
        return ActionPolicyEffect::Ask;
    }
    policy.default_effect
}

fn should_gate_tool(capability: &core::CapabilitySpec) -> bool {
    capability.side_effect != core::SideEffect::None
        && !capability.tags.iter().any(|tag| tag == "plan")
}

fn planning_input_effects(payload: HookEventPayload) -> Vec<HookEffect> {
    let HookEventPayload::Input { text, .. } = payload else {
        return vec![HookEffect::Continue];
    };
    let trimmed = text.trim();

    if let Some(rest) =
        strip_prefixed_input(trimmed, "/plan").or_else(|| strip_prefixed_input(trimmed, "plan:"))
    {
        return planning_mode_prefix_effects("plan", rest, "已切换到 plan mode。");
    }

    if let Some(rest) =
        strip_prefixed_input(trimmed, "/code").or_else(|| strip_prefixed_input(trimmed, "code:"))
    {
        return planning_mode_prefix_effects("code", rest, "已切换到 code mode。");
    }

    vec![HookEffect::Continue]
}

fn strip_prefixed_input<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let rest = text.strip_prefix(prefix)?;
    if prefix.ends_with(':')
        || rest.is_empty()
        || rest
            .chars()
            .next()
            .is_some_and(|character| character.is_whitespace())
    {
        Some(rest.trim())
    } else {
        None
    }
}

fn planning_mode_prefix_effects(
    mode_id: &str,
    rest: &str,
    handled_message: &str,
) -> Vec<HookEffect> {
    let mut effects = vec![HookEffect::SwitchMode {
        mode_id: mode_id.to_string(),
    }];
    if rest.is_empty() {
        effects.push(HookEffect::HandledInput {
            response: handled_message.to_string(),
        });
    } else {
        effects.push(HookEffect::TransformInput {
            text: rest.to_string(),
        });
    }
    effects
}

#[derive(Debug, Clone)]
struct ServerPluginHostReload {
    snapshot: PluginActiveSnapshot,
    resources: ResourceCatalog,
    provider_catalog: ProviderContributionCatalog,
    builtin_modes: Vec<GovernanceModeSpec>,
    plugin_modes: Vec<GovernanceModeSpec>,
}

fn reload_server_plugin_host_snapshot(
    registry: &PluginRegistry,
    descriptors: Vec<PluginDescriptor>,
    hook_registry: &BuiltinHookRegistry,
) -> Result<ServerPluginHostReload> {
    let resources = resources_discover(&descriptors)?.catalog;
    let provider_catalog = ProviderContributionCatalog::from_descriptors(&descriptors)?;
    let builtin_modes = descriptor_modes_for_plugin_ids(
        &descriptors,
        &[
            BUILTIN_GOVERNANCE_MODES_PLUGIN_ID,
            BUILTIN_PLANNING_PLUGIN_ID,
        ],
    );
    let plugin_modes = descriptor_modes(&descriptors, EXTERNAL_PLUGIN_MODES_PLUGIN_ID);
    registry.stage_candidate_with_hook_registry(descriptors, hook_registry)?;
    let snapshot = registry.commit_candidate().ok_or_else(|| {
        AstrError::Internal("plugin-host active snapshot commit unexpectedly failed".to_string())
    })?;

    Ok(ServerPluginHostReload {
        snapshot,
        resources,
        provider_catalog,
        builtin_modes,
        plugin_modes,
    })
}

/// 解析插件搜索路径。
///
/// 从环境变量 `ASTRCODE_PLUGIN_DIRS` 读取，未设置时默认为
/// `~/.astrcode/plugins`。
fn resolve_plugin_search_paths(
    home_dir: &Path,
    explicit_paths: Option<Vec<PathBuf>>,
) -> Vec<std::path::PathBuf> {
    if let Some(paths) = explicit_paths {
        return paths;
    }
    let separators: &[char] = if cfg!(windows) { &[';'] } else { &[':'] };
    match std::env::var(core::env::ASTRCODE_PLUGIN_DIRS_ENV) {
        Ok(value) if !value.trim().is_empty() => value
            .split(separators)
            .filter(|s| !s.trim().is_empty())
            .map(|s| std::path::PathBuf::from(s.trim()))
            .collect(),
        _ => vec![home_dir.join(".astrcode").join("plugins")],
    }
}

fn resolve_agent_control_limits(config: &Config) -> ServerAgentControlLimits {
    let runtime = resolve_runtime_config(&config.runtime);
    resolve_agent_control_limits_from_runtime(&runtime)
}

fn resolve_agent_control_limits_from_runtime(
    runtime: &ResolvedRuntimeConfig,
) -> ServerAgentControlLimits {
    ServerAgentControlLimits {
        max_depth: runtime.agent.max_subrun_depth,
        max_concurrent: runtime.agent.max_concurrent,
        finalized_retain_limit: runtime.agent.finalized_retain_limit,
        inbox_capacity: runtime.agent.inbox_capacity,
        parent_delivery_capacity: runtime.agent.parent_delivery_capacity,
    }
}

#[cfg(test)]
mod tests {
    use astrcode_plugin_host::{
        BuiltinHookRegistry, CommandDescriptor, PluginDescriptor, PluginRegistry,
        ProviderDescriptor,
    };

    use super::{
        build_server_plugin_contribution_descriptors, builtin_server_mode_specs,
        register_builtin_permission_hooks, register_builtin_planning_hooks,
        reload_server_plugin_host_snapshot, resolve_agent_control_limits,
        split_builtin_planning_modes,
    };
    use crate::bootstrap::deps::core::{AgentConfig, Config, RuntimeConfig, config};

    #[test]
    fn resolve_agent_control_limits_uses_runtime_agent_config() {
        let config = Config {
            runtime: RuntimeConfig {
                agent: Some(AgentConfig {
                    max_subrun_depth: Some(5),
                    max_spawn_per_turn: Some(2),
                    max_concurrent: Some(4),
                    finalized_retain_limit: Some(123),
                    inbox_capacity: Some(456),
                    parent_delivery_capacity: Some(789),
                }),
                ..RuntimeConfig::default()
            },
            ..Config::default()
        };

        let limits = resolve_agent_control_limits(&config);

        assert_eq!(limits.max_depth, 5);
        assert_eq!(limits.max_concurrent, 4);
        assert_eq!(limits.finalized_retain_limit, 123);
        assert_eq!(limits.inbox_capacity, 456);
        assert_eq!(limits.parent_delivery_capacity, 789);
    }

    #[test]
    fn resolve_agent_control_limits_uses_config_defaults() {
        let limits = resolve_agent_control_limits(&Config::default());

        assert_eq!(limits.max_depth, config::DEFAULT_MAX_SUBRUN_DEPTH);
    }

    #[test]
    fn server_plugin_descriptors_collect_builtin_and_external_contributions() {
        let external = PluginDescriptor::builtin("external-plugin", "External Plugin");
        let builtin_modes = builtin_server_mode_specs().expect("builtin mode specs should build");
        let (governance_modes, planning_modes) = split_builtin_planning_modes(builtin_modes);

        let descriptors = build_server_plugin_contribution_descriptors(
            &[],
            &[],
            &[],
            governance_modes,
            planning_modes,
            Vec::new(),
            vec![external],
        );
        let plugin_ids = descriptors
            .iter()
            .map(|descriptor| descriptor.plugin_id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            plugin_ids,
            vec![
                "builtin-provider-openai",
                "builtin-governance-modes",
                "builtin-permission-policy",
                "builtin-planning",
                "external-plugin-modes",
                "builtin-composer-resources",
                "builtin-core-tools",
                "external-mcp-tools",
                "builtin-collaboration-tools",
                "external-plugin",
            ]
        );
        assert_eq!(descriptors[1].modes.len(), 1);
        assert_eq!(descriptors[2].hooks.len(), 2);
        assert_eq!(descriptors[3].modes.len(), 1);
        assert_eq!(descriptors[3].hooks.len(), 1);
    }

    #[test]
    fn server_plugin_reload_bridge_commits_snapshot_resources_and_providers() {
        let registry = PluginRegistry::default();
        let mut descriptor = PluginDescriptor::builtin("project-runtime", "Project Runtime");
        descriptor.commands.push(CommandDescriptor {
            command_id: "project.apply".to_string(),
            entry_ref: ".codex/commands/apply.md".to_string(),
        });
        descriptor.providers.push(ProviderDescriptor {
            provider_id: "project-openai".to_string(),
            api_kind: "openai".to_string(),
        });
        let builtin_modes = builtin_server_mode_specs().expect("builtin mode specs should build");
        let builtin_modes_for_hooks = builtin_modes.clone();
        let (governance_modes, planning_modes) = split_builtin_planning_modes(builtin_modes);
        let descriptors = build_server_plugin_contribution_descriptors(
            &[],
            &[],
            &[],
            governance_modes,
            planning_modes,
            Vec::new(),
            vec![descriptor],
        );

        let mut hook_registry = BuiltinHookRegistry::new();
        register_builtin_planning_hooks(&mut hook_registry);
        register_builtin_permission_hooks(&mut hook_registry, builtin_modes_for_hooks);
        let reload = reload_server_plugin_host_snapshot(&registry, descriptors, &hook_registry)
            .expect("bridge reload should commit");

        assert_eq!(
            reload.snapshot.plugin_ids,
            vec![
                "builtin-provider-openai".to_string(),
                "builtin-governance-modes".to_string(),
                "builtin-permission-policy".to_string(),
                "builtin-planning".to_string(),
                "external-plugin-modes".to_string(),
                "builtin-composer-resources".to_string(),
                "builtin-core-tools".to_string(),
                "external-mcp-tools".to_string(),
                "builtin-collaboration-tools".to_string(),
                "project-runtime".to_string(),
            ]
        );
        assert_eq!(reload.builtin_modes.len(), 2);
        assert_eq!(reload.snapshot.modes.len(), 2);
        assert_eq!(reload.resources.commands.len(), 2);
        assert_eq!(
            reload
                .provider_catalog
                .provider("project-openai")
                .expect("provider should be registered")
                .api_kind,
            "openai"
        );
        assert_eq!(
            registry
                .active_snapshot()
                .expect("active snapshot should be committed")
                .snapshot_id,
            reload.snapshot.snapshot_id
        );
    }
}
