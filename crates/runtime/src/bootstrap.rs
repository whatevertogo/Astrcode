//! # 运行时引导 (Runtime Bootstrap)
//!
//! 负责初始化整个运行时系统，包括：
//! 1. 发现并加载插件清单
//! 2. 组装运行时能力面（内置工具 + 插件）
//! 3. 创建 `RuntimeService`（核心门面）
//! 4. 创建 `RuntimeCoordinator`（协调器）
//! 5. 创建 `RuntimeGovernance`（治理层，支持热重载）
//!
//! ## 引导时序
//!
//! ```text
//! 内置能力初始化 → RuntimeService → 后台插件加载
//! ```
//!
//! 插件加载在后台异步进行，不阻塞应用启动。
//! 内置能力立即可用，插件能力在加载完成后动态注册。

use std::{
    collections::HashSet,
    sync::{Arc, RwLock as StdRwLock},
};

use astrcode_core::{AstrError, PluginManifest, PluginRegistry, RuntimeCoordinator, RuntimeHandle};
use astrcode_runtime_agent_loader::{AgentLoaderError, AgentProfileLoader, AgentProfileRegistry};
use astrcode_runtime_registry::CapabilityRouter;
use astrcode_runtime_skill_loader::{SkillCatalog, merge_skill_layers};
use tokio::sync::RwLock;

use crate::{
    RuntimeService, ServiceError,
    plugin_discovery::{configured_plugin_paths, discover_plugin_manifests_in},
    runtime_governance::RuntimeGovernance,
    runtime_surface_assembler::{
        PluginInitializer, SupervisorPluginInitializer, assemble_plugins_only,
    },
    service::DeferredSubAgentExecutor,
};

/// 运行时引导完成后的结果容器。
///
/// 包含三个核心组件：
/// - `service`: `RuntimeService` 门面，处理所有会话和 Turn 操作
/// - `coordinator`: `RuntimeCoordinator`，管理插件生命周期和托管组件
/// - `governance`: `RuntimeGovernance`，提供治理和可观测性能力（如热重载）
/// - `plugin_load_handle`: 后台插件加载任务句柄，可用于等待插件加载完成
pub struct RuntimeBootstrap {
    /// 运行时服务门面，所有会话/工具/Turn 操作的入口
    pub service: Arc<RuntimeService>,
    /// Agent 定义加载器。
    pub agent_loader: Arc<AgentProfileLoader>,
    /// Agent Profile 注册表快照。
    ///
    /// bootstrap 在启动时完成 profile 合并，避免后续调用方各自读取文件导致语义漂移。
    pub agent_profiles: Arc<StdRwLock<Arc<AgentProfileRegistry>>>,
    /// 运行时协调器，管理插件注册和托管组件
    pub coordinator: Arc<RuntimeCoordinator>,
    /// 运行时治理层，支持快照、热重载等治理能力
    pub governance: Arc<RuntimeGovernance>,
    /// 后台插件加载任务句柄
    pub plugin_load_handle: PluginLoadHandle,
}

/// 插件加载状态。
#[derive(Debug, Clone, Default)]
pub struct PluginLoadState {
    /// 已加载的插件数量
    pub loaded_count: usize,
    /// 加载失败的插件数量
    pub failed_count: usize,
    /// 是否已完成所有插件加载
    pub completed: bool,
}

/// 后台插件加载任务句柄。
///
/// 用于查询插件加载状态或等待加载完成。
#[derive(Clone)]
pub struct PluginLoadHandle {
    state: Arc<RwLock<PluginLoadState>>,
    /// 用于通知等待者加载已完成
    completed_notify: Arc<tokio::sync::Notify>,
}

impl PluginLoadHandle {
    /// 创建新的句柄。
    fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(PluginLoadState::default())),
            completed_notify: Arc::new(tokio::sync::Notify::new()),
        }
    }

    /// 获取当前加载状态。
    pub async fn state(&self) -> PluginLoadState {
        self.state.read().await.clone()
    }

    /// 等待插件加载完成。
    ///
    /// 使用条件变量而非轮询，避免 CPU 空转。
    pub async fn wait_completed(&self) {
        loop {
            {
                let state = self.state.read().await;
                if state.completed {
                    return;
                }
            }
            // 等待完成通知，避免轮询
            self.completed_notify.notified().await;
        }
    }

    /// 标记加载完成并通知等待者。
    fn mark_completed(&self) {
        self.completed_notify.notify_waiters();
    }
}

/// 引导运行时系统。
///
/// 按以下顺序初始化：
/// 1. 创建内置能力路由器
/// 2. 创建 `RuntimeService`（立即可用）
/// 3. 在后台异步加载插件
///
/// 插件加载失败不会导致引导失败，只会记录警告日志。
pub async fn bootstrap_runtime() -> std::result::Result<RuntimeBootstrap, AstrError> {
    let search_paths = configured_plugin_paths();
    let manifests = discover_plugin_manifests_in(&search_paths)?;
    let initializer = SupervisorPluginInitializer::new(search_paths);
    bootstrap_runtime_from_manifests(manifests, &initializer).await
}

pub(crate) async fn bootstrap_runtime_from_manifests<I>(
    manifests: Vec<PluginManifest>,
    initializer: &I,
) -> std::result::Result<RuntimeBootstrap, AstrError>
where
    I: PluginInitializer + Clone + Send + Sync + 'static,
{
    let plugin_registry = Arc::new(PluginRegistry::default());
    let builtin_skills = astrcode_runtime_skill_loader::load_builtin_skills();
    let agent_loader = Arc::new(AgentProfileLoader::new()?);
    let agent_profiles = Arc::new(StdRwLock::new(Arc::new(
        agent_loader.load().map_err(agent_loader_error_to_astr)?,
    )));

    // 为当前 surface 创建独立的 skill 目录和 router。
    // 后台加载完成后会整体替换为新 surface，避免把半更新状态暴露给新 turn。
    let builtin_skill_catalog = Arc::new(SkillCatalog::new(builtin_skills.clone()));
    let subagent_executor = Arc::new(DeferredSubAgentExecutor::default());
    let router = create_builtin_router(
        Arc::clone(&builtin_skill_catalog),
        subagent_executor.clone(),
    )?;

    // 获取内置能力名称集合（用于冲突检测）
    let builtin_names: HashSet<String> = router
        .descriptors()
        .iter()
        .map(|d| d.name.clone())
        .collect();

    // 创建 RuntimeService（立即可用）
    let service = Arc::new(
        RuntimeService::from_capabilities_with_prompt_inputs_and_agents(
            router.clone(),
            Vec::new(), // prompt declarations 将在插件加载后动态添加
            Arc::clone(&builtin_skill_catalog),
            Arc::clone(&agent_loader),
            Arc::clone(&agent_profiles),
        )
        .map_err(service_error_to_astr)?,
    );
    subagent_executor.bind(&service);
    // 配置热重载需要尽早挂载 watcher，确保用户在应用启动后直接编辑 config.json
    // 时，后续新 turn 就能看到最新的 runtime 参数和默认模型选择。
    service.start_config_auto_reload();
    // Agent 定义属于独立的文件系统输入面，需要单独 watch，避免用户编辑 agents
    // 后必须重启 runtime 才能生效。
    service.start_agent_auto_reload();

    // 创建后台加载句柄
    let plugin_load_handle = PluginLoadHandle::new();

    // 获取内置能力描述符
    let builtin_capabilities = router.descriptors();

    // 先创建 coordinator 和 governance，以便后台任务可以更新它们
    let runtime: Arc<dyn RuntimeHandle> = service.clone();
    let coordinator = Arc::new(RuntimeCoordinator::new(
        runtime,
        Arc::clone(&plugin_registry),
        builtin_capabilities.clone(),
    ));
    let governance = Arc::new(RuntimeGovernance::with_active_plugins(
        Arc::clone(&service),
        Arc::clone(&coordinator),
        Vec::new(),
    ));

    // 如果没有插件，直接返回
    if manifests.is_empty() {
        // 标记加载完成
        {
            let mut state = plugin_load_handle.state.write().await;
            state.completed = true;
        }
        plugin_load_handle.mark_completed();

        return Ok(RuntimeBootstrap {
            service,
            agent_loader,
            agent_profiles,
            coordinator,
            governance,
            plugin_load_handle,
        });
    }

    // 准备后台任务所需的变量
    let plugin_registry_for_bg = Arc::clone(&plugin_registry);
    let initializer_for_bg = initializer.clone();
    let state_for_bg = Arc::clone(&plugin_load_handle.state);
    let completed_notify_for_bg = Arc::clone(&plugin_load_handle.completed_notify);
    let builtin_names_for_bg = builtin_names;
    let service_for_bg = Arc::clone(&service);
    let governance_for_bg = Arc::clone(&governance);
    let coordinator_for_bg = Arc::clone(&coordinator);
    let builtin_skills_for_bg = builtin_skills;
    let subagent_executor_for_bg = subagent_executor;
    let total_plugin_count = manifests.len();

    tokio::spawn(async move {
        let result = assemble_plugins_only(
            manifests,
            &initializer_for_bg,
            Arc::clone(&plugin_registry_for_bg),
            builtin_names_for_bg,
        )
        .await;

        match result {
            Ok(assembled) => {
                let updated_skill_catalog = Arc::new(SkillCatalog::new(merge_skill_layers(
                    builtin_skills_for_bg,
                    assembled.skills,
                )));
                let updated_router = match build_runtime_router(
                    Arc::clone(&updated_skill_catalog),
                    assembled.invokers,
                    subagent_executor_for_bg.clone(),
                ) {
                    Ok(router) => router,
                    Err(error) => {
                        log::error!("failed to build background runtime router: {}", error);
                        let mut state = state_for_bg.write().await;
                        state.failed_count =
                            assembled.stats.failed_count + assembled.stats.loaded_count;
                        state.completed = true;
                        drop(state); // 先释放锁再通知
                        completed_notify_for_bg.notify_waiters();
                        return;
                    },
                };
                let updated_capabilities = updated_router.descriptors();

                // 先切换 service，让新 turn 能看到完整的新 surface；若失败则不推进后续状态。
                if let Err(error) = service_for_bg
                    .replace_capabilities_with_prompt_inputs_and_hooks(
                        updated_router,
                        assembled.prompt_declarations,
                        updated_skill_catalog,
                        assembled.hook_handlers,
                    )
                    .await
                {
                    log::error!("failed to replace runtime capabilities: {}", error);
                    let mut state = state_for_bg.write().await;
                    state.failed_count =
                        assembled.stats.failed_count + assembled.stats.loaded_count;
                    state.completed = true;
                    drop(state); // 先释放锁再通知
                    completed_notify_for_bg.notify_waiters();
                    return;
                }

                // 更新 coordinator 的能力列表和托管组件
                let old_components = coordinator_for_bg.replace_runtime_surface(
                    assembled.plugin_entries,
                    updated_capabilities,
                    assembled.managed_components,
                );

                // 关闭旧的托管组件（这里应该没有，因为是第一次更新）
                for component in old_components {
                    if let Err(error) = component.shutdown_component().await {
                        log::warn!("failed to shut down old component: {}", error);
                    }
                }

                // 更新 governance 的活跃插件列表（用于健康检查）
                governance_for_bg
                    .update_active_plugins(assembled.active_plugins)
                    .await;

                // 更新状态
                let mut state = state_for_bg.write().await;
                state.loaded_count = assembled.stats.loaded_count;
                state.failed_count = assembled.stats.failed_count;
                state.completed = true;
                drop(state); // 先释放锁再通知
                completed_notify_for_bg.notify_waiters();

                log::info!(
                    "background plugin loading completed: {} loaded, {} failed",
                    assembled.stats.loaded_count,
                    assembled.stats.failed_count
                );
            },
            Err(error) => {
                log::error!("failed to assemble plugins: {}", error);
                let mut state = state_for_bg.write().await;
                // 整个装配过程失败，所有插件都视为加载失败
                state.failed_count = total_plugin_count;
                state.completed = true;
                drop(state); // 先释放锁再通知
                completed_notify_for_bg.notify_waiters();
            },
        }
    });

    Ok(RuntimeBootstrap {
        service,
        agent_loader,
        agent_profiles,
        coordinator,
        governance,
        plugin_load_handle,
    })
}

/// 创建只包含内置能力的路由器。
fn create_builtin_router(
    skill_catalog: Arc<SkillCatalog>,
    subagent_executor: Arc<dyn astrcode_runtime_agent_tool::SubAgentExecutor>,
) -> std::result::Result<CapabilityRouter, AstrError> {
    let invokers = crate::builtin_capabilities::built_in_capability_invokers(
        skill_catalog,
        subagent_executor,
    )?;

    let mut builder = CapabilityRouter::builder();
    for invoker in invokers {
        builder = builder.register_invoker(invoker);
    }
    builder.build()
}

/// 构建完整 runtime router。
///
/// 先挂载共享同一份 skill 目录的内置能力，再批量注册插件能力，
/// 确保 `Skill` 工具与 prompt surface 看到的是同一份目录。
fn build_runtime_router(
    skill_catalog: Arc<SkillCatalog>,
    plugin_invokers: Vec<Arc<dyn astrcode_core::CapabilityInvoker>>,
    subagent_executor: Arc<dyn astrcode_runtime_agent_tool::SubAgentExecutor>,
) -> std::result::Result<CapabilityRouter, AstrError> {
    let router = create_builtin_router(skill_catalog, subagent_executor)?;
    router.register_invokers(plugin_invokers)?;
    Ok(router)
}

fn service_error_to_astr(error: ServiceError) -> AstrError {
    match error {
        ServiceError::NotFound(message)
        | ServiceError::Conflict(message)
        | ServiceError::InvalidInput(message) => AstrError::Validation(message),
        ServiceError::Internal(error) => AstrError::Internal(error.to_string()),
    }
}

fn agent_loader_error_to_astr(error: AgentLoaderError) -> AstrError {
    match error {
        AgentLoaderError::ResolvePath(source) => source,
        AgentLoaderError::ReadDir { path, source }
        | AgentLoaderError::ReadFile { path, source } => AstrError::io(
            format!("failed to load agent definitions from {}", path),
            source,
        ),
        AgentLoaderError::MissingFrontmatter { path } => {
            AstrError::Validation(format!("agent file '{}' is missing YAML frontmatter", path))
        },
        AgentLoaderError::ParseFrontmatter { path, source } => AstrError::Validation(format!(
            "failed to parse agent frontmatter for '{}': {}",
            path, source
        )),
        AgentLoaderError::InvalidFrontmatter { path, message } => AstrError::Validation(format!(
            "invalid agent frontmatter for '{}': {}",
            path, message
        )),
    }
}
