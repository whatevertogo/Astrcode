//! # 服务器运行时组合根
//!
//! 由 server 显式组装 adapter、kernel、session-runtime、application。
//! 所有 provider 和 gateway 在此唯一位置接线，handler 只依赖 `App`。

use std::sync::Arc;

use astrcode_adapter_storage::core_port::FsEventStore;
use astrcode_adapter_tools::builtin_tools::tool_search::ToolSearchIndex;
use astrcode_application::{AgentOrchestrationService, App, AppGovernance};
use astrcode_core::{
    CapabilityInvoker, EventStore, LlmProvider, PluginRegistry, PromptProvider, ResourceProvider,
    Result,
};
use astrcode_kernel::{CapabilityRouter, Kernel, KernelBuilder};
use astrcode_session_runtime::SessionRuntime;

use super::{
    capabilities::{
        CapabilitySurfaceSync, build_agent_invokers, build_builtin_capability_invokers,
        build_server_capability_router, build_skill_catalog, sync_external_tool_search_index,
    },
    governance::build_app_governance,
    mcp::{bootstrap_mcp_manager, build_mcp_service},
    plugins::{PluginBootstrapResult, bootstrap_plugins},
    prompt_facts::build_prompt_facts_provider,
    providers::{
        build_config_service, build_llm_provider, build_prompt_provider, build_resource_provider,
    },
};

/// 服务器运行时：组合根输出。
#[allow(dead_code)] // plugin 字段将在后续 change 中被治理路由使用
pub struct ServerRuntime {
    pub app: Arc<App>,
    pub governance: Arc<AppGovernance>,
    /// 插件注册表（治理视图使用）。
    pub plugin_registry: Arc<PluginRegistry>,
    /// 插件搜索路径。
    pub plugin_search_paths: Vec<std::path::PathBuf>,
}

/// 构建服务器运行时组合根。
///
/// 能力来源三路合并：builtin + MCP + plugin。
pub async fn bootstrap_server_runtime() -> Result<ServerRuntime> {
    let config_service = build_config_service()?;
    let working_dir = std::env::current_dir().map_err(|error| {
        astrcode_core::AstrError::io("failed to resolve server working directory", error)
    })?;
    let mcp_manager = bootstrap_mcp_manager(config_service.clone(), working_dir.as_path()).await?;

    // plugin + MCP 是外部事实源，需要先完成装配，随后再把它们注入
    // tool_search / skill catalog，避免启动态与 reload 态出现两套事实。
    let tool_search_index = Arc::new(ToolSearchIndex::new());
    let plugin_dirs = resolve_plugin_search_paths();
    let PluginBootstrapResult {
        invokers: plugin_invokers,
        skills: plugin_skills,
        registry: plugin_registry,
        supervisors: plugin_supervisors,
        search_paths: plugin_search_paths,
    } = bootstrap_plugins(plugin_dirs).await;
    let mcp_invokers = mcp_manager.current_surface().await.capability_invokers;

    let mut external_invokers: Vec<Arc<dyn CapabilityInvoker>> = mcp_invokers.clone();
    external_invokers.extend(plugin_invokers.clone());
    sync_external_tool_search_index(&tool_search_index, &external_invokers);

    // builtin 能力：工具发现索引 + skill 目录
    let skill_catalog = build_skill_catalog(plugin_skills);
    let builtin_invokers =
        build_builtin_capability_invokers(Arc::clone(&tool_search_index), skill_catalog.clone())?;

    // 三路合并：builtin + MCP + plugin
    let mut all_invokers = builtin_invokers.clone();
    all_invokers.extend(mcp_invokers);
    all_invokers.extend(plugin_invokers);
    let capabilities = build_server_capability_router(all_invokers)?;

    let kernel = Arc::new(build_kernel(
        capabilities,
        build_llm_provider(config_service.clone(), working_dir.clone()),
        build_prompt_provider(),
        build_resource_provider(mcp_manager.clone()),
    )?);
    let capability_sync =
        CapabilitySurfaceSync::new(kernel.clone(), builtin_invokers, tool_search_index);

    let event_store: Arc<dyn EventStore> = Arc::new(FsEventStore::new());
    let prompt_facts_provider =
        build_prompt_facts_provider(config_service.clone(), skill_catalog, mcp_manager.clone())?;
    let session_runtime = Arc::new(SessionRuntime::new(
        kernel.clone(),
        prompt_facts_provider,
        event_store,
    ));
    let mcp_service = build_mcp_service(
        config_service.clone(),
        working_dir,
        mcp_manager,
        capability_sync,
    );

    let agent_service = Arc::new(AgentOrchestrationService::new(
        kernel.clone(),
        session_runtime.clone(),
        Some(astrcode_application::resolve_default_token_budget(
            &config_service.get_config().await.runtime,
        )),
    ));

    // agent 四工具依赖 agent_service，必须在 kernel/session_runtime 之后单独注册
    let agent_invokers = build_agent_invokers(agent_service.clone())?;
    kernel
        .gateway()
        .capabilities()
        .register_invokers(agent_invokers)?;

    let app = Arc::new(App::new(
        kernel.clone(),
        session_runtime.clone(),
        config_service,
        mcp_service,
        agent_service,
    ));
    let governance = build_app_governance(
        session_runtime,
        kernel.clone(),
        plugin_registry.clone(),
        plugin_supervisors,
    );

    Ok(ServerRuntime {
        app,
        governance,
        plugin_registry,
        plugin_search_paths,
    })
}

/// 解析插件搜索路径。
///
/// 从环境变量 `ASTRCODE_PLUGIN_DIRS` 读取，未设置时默认为
/// `~/.astrcode/plugins`。
fn resolve_plugin_search_paths() -> Vec<std::path::PathBuf> {
    let separators: &[char] = if cfg!(windows) { &[';'] } else { &[':'] };
    match std::env::var(astrcode_core::env::ASTRCODE_PLUGIN_DIRS_ENV) {
        Ok(value) if !value.trim().is_empty() => value
            .split(separators)
            .filter(|s| !s.trim().is_empty())
            .map(|s| std::path::PathBuf::from(s.trim()))
            .collect(),
        _ => {
            let mut paths = Vec::new();
            if let Ok(home) = astrcode_core::home::resolve_home_dir() {
                paths.push(home.join(".astrcode").join("plugins"));
            }
            paths
        },
    }
}

fn build_kernel(
    capabilities: CapabilityRouter,
    llm_provider: Arc<dyn LlmProvider>,
    prompt_provider: Arc<dyn PromptProvider>,
    resource_provider: Arc<dyn ResourceProvider>,
) -> Result<Kernel> {
    KernelBuilder::default()
        .with_capabilities(capabilities)
        .with_llm_provider(llm_provider)
        .with_prompt_provider(prompt_provider)
        .with_resource_provider(resource_provider)
        .build()
        .map_err(|error| astrcode_core::AstrError::Internal(error.to_string()))
}
