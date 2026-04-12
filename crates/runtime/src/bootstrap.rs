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
    path::PathBuf,
    sync::{Arc, RwLock as StdRwLock},
};

use astrcode_core::{AstrError, PluginManifest, PluginRegistry, RuntimeCoordinator, RuntimeHandle};
use astrcode_runtime_agent_loader::{AgentLoaderError, AgentProfileLoader, AgentProfileRegistry};
use astrcode_runtime_mcp::{
    config::{
        McpApprovalData, McpConfigManager, McpConfigScope, McpServerConfig, McpSettingsStore,
    },
    manager::{McpConnectionManager, McpSurfaceSnapshot, hot_reload::McpHotReload},
};
use astrcode_runtime_registry::CapabilityRouter;
use astrcode_runtime_skill_loader::{SkillCatalog, SkillSource, merge_skill_layers};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::{
    RuntimeService, ServiceError,
    external_tool_catalog::ExternalToolCatalog,
    plugin_discovery::{configured_plugin_paths, discover_plugin_manifests_in},
    runtime_governance::RuntimeGovernance,
    runtime_surface_assembler::{
        PluginInitializer, SupervisorPluginInitializer, assemble_plugins_only,
    },
    service::{DeferredCollaborationExecutor, DeferredSubAgentExecutor},
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
    /// 后台插件初始化 spawn 的 JoinHandle，shutdown 时 abort。
    pub plugin_init_handle: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// MCP 连接管理器（可选，MCP 不可用时为 None）。
    pub mcp_manager: Option<Arc<astrcode_runtime_mcp::manager::McpConnectionManager>>,
    /// MCP 热加载 watcher 的 JoinHandle（shutdown 时 abort）。
    pub mcp_hot_reload_handle: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
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

struct McpBootstrapState {
    manager: Arc<McpConnectionManager>,
    surface: McpSurfaceSnapshot,
    watch_paths: Vec<PathBuf>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredMcpApprovals {
    #[serde(default)]
    projects: std::collections::HashMap<String, Vec<McpApprovalData>>,
}

struct FileMcpSettingsStore {
    path: PathBuf,
}

impl FileMcpSettingsStore {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn load_document(&self) -> std::result::Result<StoredMcpApprovals, String> {
        if !self.path.exists() {
            return Ok(StoredMcpApprovals::default());
        }
        let content = std::fs::read_to_string(&self.path)
            .map_err(|error| format!("failed to read {}: {}", self.path.display(), error))?;
        serde_json::from_str::<StoredMcpApprovals>(&content)
            .map_err(|error| format!("failed to parse {}: {}", self.path.display(), error))
    }

    fn save_document(&self, document: &StoredMcpApprovals) -> std::result::Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create {}: {}", parent.display(), error))?;
        }
        let content = serde_json::to_vec_pretty(document)
            .map_err(|error| format!("failed to serialize MCP approvals: {}", error))?;
        std::fs::write(&self.path, content)
            .map_err(|error| format!("failed to write {}: {}", self.path.display(), error))
    }
}

impl McpSettingsStore for FileMcpSettingsStore {
    fn load_approvals(
        &self,
        project_path: &str,
    ) -> std::result::Result<Vec<McpApprovalData>, String> {
        let document = self.load_document()?;
        Ok(document
            .projects
            .get(project_path)
            .cloned()
            .unwrap_or_default())
    }

    fn save_approval(
        &self,
        project_path: &str,
        data: &McpApprovalData,
    ) -> std::result::Result<(), String> {
        let mut document = self.load_document()?;
        let approvals = document
            .projects
            .entry(project_path.to_string())
            .or_default();
        if let Some(existing) = approvals
            .iter_mut()
            .find(|approval| approval.server_signature == data.server_signature)
        {
            *existing = data.clone();
        } else {
            approvals.push(data.clone());
        }
        self.save_document(&document)
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
        agent_loader
            .load_for_working_dir(None)
            .map_err(agent_loader_error_to_astr)?,
    )));

    // 为当前 surface 创建独立的 skill 目录和 router。
    // 后台加载完成后会整体替换为新 surface，避免把半更新状态暴露给新 turn。
    let builtin_skill_catalog = Arc::new(SkillCatalog::new(builtin_skills.clone()));
    let external_tool_catalog = Arc::new(ExternalToolCatalog::default());
    let subagent_executor = Arc::new(DeferredSubAgentExecutor::default());
    let collaboration_executor = Arc::new(DeferredCollaborationExecutor::default());
    let builtin_router = create_builtin_router(
        Arc::clone(&builtin_skill_catalog),
        Arc::clone(&external_tool_catalog),
        subagent_executor.clone(),
        collaboration_executor.clone(),
    )?;

    // 获取内置能力名称集合（用于冲突检测）
    let builtin_names: HashSet<String> = builtin_router
        .descriptors()
        .iter()
        .map(|d| d.name.clone())
        .collect();

    // 尝试初始化 MCP 连接（MCP 不可用时 runtime 照常启动）
    let mcp_bootstrap = init_mcp_connections().await;
    let (router, initial_prompt_declarations) = if let Some(bootstrap) = &mcp_bootstrap {
        let router = build_router_with_mcp(
            &builtin_router,
            bootstrap.surface.capability_invokers.clone(),
        )?;
        (router, bootstrap.surface.prompt_declarations.clone())
    } else {
        (builtin_router.clone(), Vec::new())
    };
    let mcp_manager = mcp_bootstrap
        .as_ref()
        .map(|bootstrap| bootstrap.manager.clone());
    external_tool_catalog.replace_from_descriptors(&router.descriptors());

    // 创建 RuntimeService（立即可用）
    let service = Arc::new(
        RuntimeService::from_capabilities_with_prompt_inputs_and_agents(
            router.clone(),
            initial_prompt_declarations,
            Arc::clone(&builtin_skill_catalog),
            Arc::clone(&agent_loader),
            Arc::clone(&agent_profiles),
        )
        .map_err(service_error_to_astr)?,
    );
    if let Some(bootstrap) = &mcp_bootstrap {
        service.install_mcp_manager(bootstrap.manager.clone()).await;
    }
    subagent_executor.bind(&service);
    collaboration_executor.bind(&service);
    // 配置热重载需要尽早挂载 watcher，确保用户在应用启动后直接编辑 config.json
    // 时，后续新 turn 就能看到最新的 runtime 参数和默认模型选择。
    service.watch().start_config_auto_reload();
    // Agent 定义属于独立的文件系统输入面，需要单独 watch，避免用户编辑 agents
    // 后必须重启 runtime 才能生效。
    service.watch().start_agent_auto_reload();
    let mcp_hot_reload_handle = mcp_bootstrap.as_ref().map(|bootstrap| {
        start_mcp_surface_sync(
            Arc::clone(&service),
            bootstrap.manager.clone(),
            Arc::clone(&external_tool_catalog),
            bootstrap.watch_paths.clone(),
        )
    });

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
            plugin_init_handle: std::sync::Mutex::new(None),
            mcp_manager,
            mcp_hot_reload_handle: std::sync::Mutex::new(mcp_hot_reload_handle),
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
    let external_tool_catalog_for_bg = Arc::clone(&external_tool_catalog);
    let total_plugin_count = manifests.len();

    let plugin_init_task = tokio::spawn(async move {
        let result = assemble_plugins_only(
            manifests,
            &initializer_for_bg,
            Arc::clone(&plugin_registry_for_bg),
            builtin_names_for_bg,
        )
        .await;

        match result {
            Ok(assembled) => {
                let current_surface = service_for_bg
                    .loop_surface()
                    .current_surface_snapshot()
                    .await;
                let updated_skill_catalog = Arc::new(SkillCatalog::new(merge_skill_layers(
                    filter_base_skills_by_source(current_surface.base_skills, SkillSource::Plugin),
                    assembled.skills,
                )));
                let updated_router = match build_router_from_invokers(
                    current_surface
                        .capability_invokers
                        .into_iter()
                        .filter(|invoker| !has_source_tag(&invoker.descriptor(), "source:plugin"))
                        .chain(assembled.invokers)
                        .collect(),
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
                external_tool_catalog_for_bg.replace_from_descriptors(&updated_capabilities);

                // 先切换 service，让新 turn 能看到完整的新 surface；若失败则不推进后续状态。
                let merged_prompt_declarations = merge_prompt_declarations(
                    current_surface
                        .prompt_declarations
                        .into_iter()
                        .filter(|declaration| {
                            declaration.source
                                != astrcode_runtime_prompt::prompt_declaration::PromptDeclarationSource::Plugin
                        })
                        .collect(),
                    assembled.prompt_declarations,
                );
                if let Err(error) = service_for_bg
                    .loop_surface()
                    .replace_surface(
                        updated_router,
                        merged_prompt_declarations,
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
        plugin_init_handle: std::sync::Mutex::new(Some(plugin_init_task)),
        mcp_manager,
        mcp_hot_reload_handle: std::sync::Mutex::new(mcp_hot_reload_handle),
    })
}

/// 创建只包含内置能力的路由器。
fn create_builtin_router(
    skill_catalog: Arc<SkillCatalog>,
    external_tool_catalog: Arc<ExternalToolCatalog>,
    subagent_executor: Arc<dyn astrcode_runtime_agent_tool::SubAgentExecutor>,
    collaboration_executor: Arc<dyn astrcode_runtime_agent_tool::CollaborationExecutor>,
) -> std::result::Result<CapabilityRouter, AstrError> {
    let invokers = crate::builtin_capabilities::built_in_capability_invokers(
        skill_catalog,
        external_tool_catalog,
        subagent_executor,
        collaboration_executor,
    )?;

    build_router_from_invokers(invokers)
}

fn build_router_with_mcp(
    builtin_router: &CapabilityRouter,
    mcp_invokers: Vec<Arc<dyn astrcode_core::CapabilityInvoker>>,
) -> std::result::Result<CapabilityRouter, AstrError> {
    build_router_from_invokers(
        builtin_router
            .invokers()
            .into_iter()
            .chain(mcp_invokers)
            .collect(),
    )
}

fn build_router_from_invokers(
    invokers: Vec<Arc<dyn astrcode_core::CapabilityInvoker>>,
) -> std::result::Result<CapabilityRouter, AstrError> {
    let mut builder = CapabilityRouter::builder();
    for invoker in invokers {
        builder = builder.register_invoker(invoker);
    }
    builder.build()
}

fn has_source_tag(
    descriptor: &astrcode_protocol::capability::CapabilityDescriptor,
    tag: &str,
) -> bool {
    descriptor.tags.iter().any(|candidate| candidate == tag)
}

fn filter_base_skills_by_source(
    skills: Vec<astrcode_runtime_skill_loader::SkillSpec>,
    source: SkillSource,
) -> Vec<astrcode_runtime_skill_loader::SkillSpec> {
    skills
        .into_iter()
        .filter(|skill| skill.source != source)
        .collect()
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

/// 尝试初始化 MCP 连接并启动热加载。
async fn init_mcp_connections() -> Option<McpBootstrapState> {
    let (configs, project_key, approval_store_path, watch_paths) = match load_mcp_declared_configs()
    {
        Ok(inputs) => inputs,
        Err(error) => {
            log::warn!("MCP config load failed, skipping: {}", error);
            return None;
        },
    };

    if configs.is_empty() {
        return None;
    }

    let manager = Arc::new(McpConnectionManager::new().with_approval(
        astrcode_runtime_mcp::config::McpApprovalManager::new(Box::new(FileMcpSettingsStore::new(
            approval_store_path,
        ))),
        project_key,
    ));
    let results = manager.connect_all(configs).await;
    for (name, error) in &results.failed {
        log::warn!("MCP server '{}' failed: {}", name, error);
    }

    Some(McpBootstrapState {
        surface: manager.current_surface().await,
        manager,
        watch_paths,
    })
}

fn start_mcp_surface_sync(
    service: Arc<RuntimeService>,
    manager: Arc<McpConnectionManager>,
    external_tool_catalog: Arc<ExternalToolCatalog>,
    watch_paths: Vec<PathBuf>,
) -> tokio::task::JoinHandle<()> {
    let mut surface_events = manager.subscribe_surface_events();
    let mut hot_reload = McpHotReload::new_with_paths(watch_paths);

    tokio::spawn(async move {
        loop {
            tokio::select! {
                event = hot_reload.events().recv() => {
                    if event.is_none() {
                        break;
                    }
                    match load_mcp_declared_configs() {
                        Ok((configs, _, _, _)) => {
                            if let Err(error) = manager.reload_config(configs).await {
                                log::warn!("MCP config reload failed: {}", error);
                                continue;
                            }
                            if let Err(error) = apply_mcp_surface(&service, &manager, &external_tool_catalog).await {
                                log::warn!("MCP surface apply after config reload failed: {}", error);
                            }
                        },
                        Err(error) => {
                            log::warn!("MCP config reload load failed: {}", error);
                        },
                    }
                }
                changed = surface_events.recv() => {
                    if changed.is_err() {
                        break;
                    }
                    if let Err(error) = apply_mcp_surface(&service, &manager, &external_tool_catalog).await {
                        log::warn!("MCP surface apply failed: {}", error);
                    }
                }
            }
        }
        log::info!("MCP surface sync task ended");
    })
}

async fn apply_mcp_surface(
    service: &Arc<RuntimeService>,
    manager: &Arc<McpConnectionManager>,
    external_tool_catalog: &Arc<ExternalToolCatalog>,
) -> std::result::Result<(), AstrError> {
    let current_surface = service.loop_surface().current_surface_snapshot().await;
    let mcp_surface = manager.current_surface().await;
    let capabilities = build_router_from_invokers(
        current_surface
            .capability_invokers
            .into_iter()
            .filter(|invoker| !has_source_tag(&invoker.descriptor(), "source:mcp"))
            .chain(mcp_surface.capability_invokers)
            .collect(),
    )?;
    let prompt_declarations = merge_prompt_declarations(
        current_surface
            .prompt_declarations
            .into_iter()
            .filter(|declaration| {
                declaration.source
                    != astrcode_runtime_prompt::prompt_declaration::PromptDeclarationSource::Mcp
            })
            .collect(),
        mcp_surface.prompt_declarations,
    );
    let skill_catalog = Arc::new(SkillCatalog::new(filter_base_skills_by_source(
        current_surface.base_skills,
        SkillSource::Mcp,
    )));
    let descriptors = capabilities.descriptors();
    service
        .loop_surface()
        .replace_surface(
            capabilities,
            prompt_declarations,
            skill_catalog,
            current_surface.hook_handlers,
        )
        .await
        .map_err(service_error_to_astr)?;
    external_tool_catalog.replace_from_descriptors(&descriptors);
    Ok(())
}

fn load_mcp_declared_configs()
-> std::result::Result<(Vec<McpServerConfig>, String, PathBuf, Vec<PathBuf>), AstrError> {
    let working_dir = std::env::current_dir()
        .map_err(|error| AstrError::io("failed to resolve current directory", error))?;
    let user_config_path = crate::config::config_path()?;
    let user_config = crate::config::load_config_from_path(&user_config_path)?;
    let local_overlay_path = crate::config::project_overlay_path(&working_dir)?;
    let local_overlay = crate::config::load_config_overlay_from_path(&local_overlay_path)?;
    let project_mcp_path = working_dir.join(".mcp.json");
    let approval_store_path = approval_store_path()?;
    let project_key = std::fs::canonicalize(&working_dir)
        .unwrap_or_else(|_| working_dir.clone())
        .to_string_lossy()
        .to_string();

    let mut configs = Vec::new();
    if let Some(raw) = user_config.mcp.as_ref() {
        configs.extend(McpConfigManager::load_from_value(
            raw,
            McpConfigScope::User,
        )?);
    }
    if let Some(overlay) = local_overlay
        .as_ref()
        .and_then(|overlay| overlay.mcp.as_ref())
    {
        configs.extend(McpConfigManager::load_from_value(
            overlay,
            McpConfigScope::Local,
        )?);
    }
    if project_mcp_path.exists() {
        configs.extend(McpConfigManager::load_from_file(
            &project_mcp_path,
            McpConfigScope::Project,
        )?);
    }

    Ok((
        merge_mcp_scoped_configs(configs),
        project_key,
        approval_store_path.clone(),
        vec![
            user_config_path,
            local_overlay_path,
            project_mcp_path,
            approval_store_path,
        ],
    ))
}

fn merge_mcp_scoped_configs(configs: Vec<McpServerConfig>) -> Vec<McpServerConfig> {
    let mut by_signature = std::collections::HashMap::<String, McpServerConfig>::new();
    for config in configs {
        let signature = McpConfigManager::compute_signature(&config);
        match by_signature.get(&signature) {
            Some(existing) if existing.scope > config.scope => {},
            _ => {
                by_signature.insert(signature, config);
            },
        }
    }
    let mut merged = by_signature.into_values().collect::<Vec<_>>();
    merged.sort_by(|left, right| left.name.cmp(&right.name));
    merged
}

fn approval_store_path() -> std::result::Result<PathBuf, AstrError> {
    Ok(astrcode_core::project::astrcode_dir()?.join("mcp-approvals.json"))
}

/// 合并 MCP prompt declarations 和插件 prompt declarations，按 block_id 去重。
///
/// MCP declarations 在前（较低优先级），插件 declarations 在后（较高优先级）。
/// 相同 block_id 的声明只保留后出现的（插件覆盖 MCP）。
fn merge_prompt_declarations(
    mcp_declarations: Vec<astrcode_runtime_prompt::PromptDeclaration>,
    plugin_declarations: Vec<astrcode_runtime_prompt::PromptDeclaration>,
) -> Vec<astrcode_runtime_prompt::PromptDeclaration> {
    use std::collections::HashSet;

    let mut seen_block_ids = HashSet::new();
    let mut merged = Vec::with_capacity(mcp_declarations.len() + plugin_declarations.len());

    // 先加 MCP（较低优先级）
    for decl in mcp_declarations {
        seen_block_ids.insert(decl.block_id.clone());
        merged.push(decl);
    }

    // 再加插件（较高优先级），去重
    for decl in plugin_declarations {
        if seen_block_ids.insert(decl.block_id.clone()) {
            merged.push(decl);
        } else {
            // 替换已有的同 block_id 声明
            if let Some(existing) = merged.iter_mut().find(|d| d.block_id == decl.block_id) {
                *existing = decl;
            }
        }
    }

    merged
}
