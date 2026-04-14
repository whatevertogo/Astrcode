//! # 服务器运行时组合根
//!
//! 由 server 显式组装 adapter、kernel、session-runtime、application。
//! 所有 provider 和 gateway 在此唯一位置接线，handler 只依赖 `App`。

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use astrcode_adapter_storage::core_port::FsEventStore;
use astrcode_adapter_tools::builtin_tools::tool_search::ToolSearchIndex;
use astrcode_application::{
    AgentOrchestrationService, App, AppGovernance, RuntimeObservabilityCollector, WatchService,
    lifecycle::TaskRegistry,
};
use astrcode_core::{
    CapabilityInvoker, Config, EventStore, LlmProvider, PromptProvider, ResolvedRuntimeConfig,
    ResourceProvider, Result, RuntimeCoordinator, resolve_runtime_config,
};
use astrcode_kernel::{AgentControlLimits, CapabilityRouter, Kernel, KernelBuilder};
use astrcode_session_runtime::SessionRuntime;

use super::{
    capabilities::{
        CapabilitySurfaceSync, build_agent_tool_invokers, build_core_tool_invokers,
        build_server_capability_router, build_skill_catalog, build_stable_local_invokers,
        sync_external_tool_search_index,
    },
    governance::{GovernanceBuildInput, build_app_governance},
    mcp::{bootstrap_mcp_manager, build_mcp_service, warmup_mcp_manager},
    plugins::{PluginBootstrapResult, bootstrap_plugins_with_skill_root},
    prompt_facts::build_prompt_facts_provider,
    providers::{
        build_config_service, build_llm_provider, build_profile_resolution_service,
        build_prompt_provider, build_resource_provider,
    },
    watch::{bootstrap_profile_watch_runtime, build_watch_service},
};

/// 服务器运行时：组合根输出。
pub struct ServerRuntime {
    pub app: Arc<App>,
    pub governance: Arc<AppGovernance>,
    pub handles: Arc<ServerRuntimeHandles>,
}

pub struct ServerRuntimeHandles {
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
            None => astrcode_core::home::resolve_home_dir()?,
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
        None => std::env::current_dir().map_err(|error| {
            astrcode_core::AstrError::io("failed to resolve server working directory", error)
        })?,
    };
    let resolved_config = config_service
        .load_overlayed_config(Some(working_dir.as_path()))
        .map_err(|error| astrcode_core::AstrError::Internal(error.to_string()))?;
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
        registry: plugin_registry,
        supervisors: plugin_supervisors,
        search_paths: plugin_search_paths,
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
    let skill_catalog = build_skill_catalog(plugin_skills);
    let core_tool_invokers =
        build_core_tool_invokers(Arc::clone(&tool_search_index), skill_catalog.clone())?;

    // 初始 router 先用“当前可立即装配的能力面”启动：
    // core builtin tools + 当前 external 动态能力。
    // agent tools 要等 agent_service 准备好后再并入稳定本地层。
    let mut initial_router_invokers = core_tool_invokers.clone();
    initial_router_invokers.extend(external_dynamic_invokers.clone());
    let capabilities = build_server_capability_router(initial_router_invokers)?;

    let kernel = Arc::new(build_kernel(
        capabilities,
        build_llm_provider(config_service.clone(), working_dir.clone()),
        build_prompt_provider(),
        build_resource_provider(mcp_manager.clone()),
        resolve_agent_control_limits(&resolved_config),
    )?);
    let observability = Arc::new(RuntimeObservabilityCollector::new());
    let task_registry = Arc::new(TaskRegistry::new());

    let event_store: Arc<dyn EventStore> = Arc::new(FsEventStore::new_with_projects_root(
        paths.projects_root.clone(),
    ));
    let prompt_facts_provider = build_prompt_facts_provider(
        config_service.clone(),
        skill_catalog.clone(),
        mcp_manager.clone(),
        agent_loader.clone(),
    )?;
    let session_runtime = Arc::new(SessionRuntime::new(
        kernel.clone(),
        prompt_facts_provider,
        event_store,
        observability.clone(),
    ));
    let profiles = build_profile_resolution_service(agent_loader.clone())?;
    let watch_service = match options.watch_service_override.clone() {
        Some(service) => service,
        None => build_watch_service(agent_loader)
            .map_err(|error| astrcode_core::AstrError::Internal(error.to_string()))?,
    };
    let agent_service = Arc::new(AgentOrchestrationService::new(
        kernel.clone(),
        session_runtime.clone(),
        config_service.clone(),
        profiles.clone(),
        task_registry.clone(),
        observability.clone(),
    ));

    // agent 四工具依赖 agent_service，必须在 kernel/session_runtime 之后单独注册。
    // 组装完成后，稳定本地层 = core builtin tools + agent tools。
    let agent_tool_invokers = build_agent_tool_invokers(agent_service.clone())?;
    let stable_local_invokers =
        build_stable_local_invokers(core_tool_invokers, agent_tool_invokers);
    let capability_sync = CapabilitySurfaceSync::new(
        kernel.clone(),
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

    let app = Arc::new(App::new(
        kernel.clone(),
        session_runtime.clone(),
        profiles,
        config_service.clone(),
        mcp_service,
        agent_service,
    ));
    let governance = build_app_governance(GovernanceBuildInput {
        session_runtime,
        config_service: config_service.clone(),
        coordinator,
        task_registry,
        observability,
        mcp_manager: Arc::clone(&mcp_manager),
        capability_sync: capability_sync.clone(),
        skill_catalog,
        plugin_search_paths: plugin_search_paths.clone(),
        plugin_skill_root: paths.plugin_skill_root.clone(),
        plugin_supervisors,
        working_dir: working_dir.clone(),
    });
    let profile_watch_runtime = if options.enable_profile_watch {
        Some(
            bootstrap_profile_watch_runtime(Arc::clone(&app), Arc::clone(&watch_service))
                .await
                .map_err(|error| astrcode_core::AstrError::Internal(error.to_string()))?,
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
        app,
        governance,
        handles: Arc::new(ServerRuntimeHandles {
            _profile_watch_runtime: profile_watch_runtime,
            _mcp_warmup_runtime: mcp_warmup_runtime,
        }),
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
    match std::env::var(astrcode_core::env::ASTRCODE_PLUGIN_DIRS_ENV) {
        Ok(value) if !value.trim().is_empty() => value
            .split(separators)
            .filter(|s| !s.trim().is_empty())
            .map(|s| std::path::PathBuf::from(s.trim()))
            .collect(),
        _ => vec![home_dir.join(".astrcode").join("plugins")],
    }
}

fn build_kernel(
    capabilities: CapabilityRouter,
    llm_provider: Arc<dyn LlmProvider>,
    prompt_provider: Arc<dyn PromptProvider>,
    resource_provider: Arc<dyn ResourceProvider>,
    agent_limits: AgentControlLimits,
) -> Result<Kernel> {
    KernelBuilder::default()
        .with_capabilities(capabilities)
        .with_llm_provider(llm_provider)
        .with_prompt_provider(prompt_provider)
        .with_resource_provider(resource_provider)
        .with_agent_limits(agent_limits)
        .build()
        .map_err(|error| astrcode_core::AstrError::Internal(error.to_string()))
}

fn resolve_agent_control_limits(config: &Config) -> AgentControlLimits {
    let runtime = resolve_runtime_config(&config.runtime);
    resolve_agent_control_limits_from_runtime(&runtime)
}

fn resolve_agent_control_limits_from_runtime(
    runtime: &ResolvedRuntimeConfig,
) -> AgentControlLimits {
    AgentControlLimits {
        max_depth: runtime.agent.max_subrun_depth,
        max_concurrent: runtime.agent.max_concurrent,
        finalized_retain_limit: runtime.agent.finalized_retain_limit,
        inbox_capacity: runtime.agent.inbox_capacity,
        parent_delivery_capacity: runtime.agent.parent_delivery_capacity,
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{AgentConfig, Config, RuntimeConfig};

    use super::resolve_agent_control_limits;

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

        assert_eq!(
            limits.max_depth,
            astrcode_core::config::DEFAULT_MAX_SUBRUN_DEPTH
        );
    }
}
