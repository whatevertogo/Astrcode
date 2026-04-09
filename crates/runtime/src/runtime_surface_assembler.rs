//! # 运行时能力面组装 (Runtime Surface Assembler)
//!
//! 负责将内置工具和外部插件的能力统一组装到 `CapabilityRouter` 中。
//!
//! ## 组装流程
//!
//! 1. 收集内置工具（shell, readFile, writeFile 等）的 invoker
//! 2. 对插件清单按名称/版本排序（保证确定性冲突解决）
//! 3. 逐个初始化插件：通过 `PluginInitializer` 启动进程并握手
//! 4. 对每个插件分三种结果：
//!    - **成功**: 注册其能力到 router，记录为活跃插件
//!    - **能力冲突**: 如果能力名已被注册，跳过该插件，记录为健康冲突
//!    - **初始化失败**: 跳过该插件，记录为不健康
//! 5. 返回组装结果：router + 所有插件条目（含活跃/跳过/失败的） + 需要管理的组件
//!
//! ## 关键约束
//!
//! - 能力名必须全局唯一：先到先得，排序保证确定性
//! - 插件初始化失败不阻塞其他插件
//! - 返回的 `managed_components` 需要在 shutdown 时有序关闭

use std::{
    collections::{BTreeMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::Instant,
};

use astrcode_core::{
    AstrError, CapabilityExecutionResult, CapabilityInvoker, HookHandler, ManagedRuntimeComponent,
    PluginHealth, PluginManifest, PluginRegistry, format_local_rfc3339, plugin::PluginEntry,
};
use astrcode_plugin::{PluginLoader, Supervisor, SupervisorHealth};
use astrcode_protocol::{
    capability::CapabilityDescriptor,
    plugin::{PeerDescriptor, PeerRole, SkillDescriptor},
};
use astrcode_runtime_prompt::{PromptDeclaration, PromptDeclarationSource};
use astrcode_runtime_registry::CapabilityRouter;
use astrcode_runtime_skill_loader::{SkillCatalog, SkillSpec, merge_skill_layers};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;

use crate::{
    builtin_capabilities::built_in_capability_invokers,
    plugin_hook_adapter::build_plugin_hook_handlers,
    plugin_skill_materializer::materialize_plugin_skills,
};

/// 组装后的运行时能力面
///
/// 包含所有成功注册的能力路由、prompt 声明、插件条目、需要管理的组件和 base skills。
pub(crate) struct AssembledRuntimeSurface {
    pub(crate) router: CapabilityRouter,
    pub(crate) skill_catalog: Arc<SkillCatalog>,
    pub(crate) prompt_declarations: Vec<PromptDeclaration>,
    pub(crate) hook_handlers: Vec<Arc<dyn HookHandler>>,
    pub(crate) plugin_entries: Vec<PluginEntry>,
    pub(crate) managed_components: Vec<Arc<dyn ManagedRuntimeComponent>>,
    pub(crate) active_plugins: Vec<ActivePluginRuntime>,
}

/// 统一的 runtime surface 贡献模型。
///
/// capability、skill 和 prompt declaration 共享同一条装配管线，
/// 但运行时仍保留它们不同的语义边界，不把 skill 伪装成普通 capability。
///
/// TODO(runtime-surface): hook 目前已经进入统一装配模型，但 surface 仍由多个并列
/// Vec 字段拼接而成。后续如果 hook / prompt / tool 再继续增长，建议把它们收口成
/// 一个更明确的 `RuntimeSurface` 聚合对象，减少 bootstrap / reload 时的平行传参。
#[derive(Clone)]
pub(crate) struct RuntimeSurfaceContribution {
    pub(crate) capability_invokers: Vec<Arc<dyn CapabilityInvoker>>,
    pub(crate) prompt_declarations: Vec<PromptDeclaration>,
    pub(crate) skills: Vec<SkillSpec>,
    pub(crate) hook_handlers: Vec<Arc<dyn HookHandler>>,
}

/// 活跃插件运行时
///
/// 代表成功初始化并注册到路由器的插件实例。
#[derive(Clone)]
pub(crate) struct ActivePluginRuntime {
    pub(crate) name: String,
    pub(crate) component: Arc<dyn ManagedPluginComponent>,
}

/// 加载完成的插件
///
/// 包含插件组件、能力描述符、调用器、prompt 声明和 skill 声明。
#[derive(Clone)]
pub(crate) struct LoadedPlugin {
    pub(crate) component: Arc<dyn ManagedPluginComponent>,
    pub(crate) capabilities: Vec<CapabilityDescriptor>,
    pub(crate) declared_skills: Vec<SkillDescriptor>,
    pub(crate) contribution: RuntimeSurfaceContribution,
}

/// 插件健康状态报告
///
/// 包装 `PluginHealth` 并附加可选的消息说明。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManagedPluginHealth {
    pub(crate) health: PluginHealth,
    pub(crate) message: Option<String>,
}

/// 可管理插件组件 trait
///
/// 扩展 `ManagedRuntimeComponent`，提供健康状态报告能力。
#[async_trait]
pub(crate) trait ManagedPluginComponent: ManagedRuntimeComponent {
    async fn health_report(&self) -> std::result::Result<ManagedPluginHealth, AstrError>;
}

/// 为 `Supervisor` 实现 `ManagedPluginComponent`
///
/// 将 Supervisor 的健康状态映射为插件健康状态。
#[async_trait]
impl ManagedPluginComponent for Supervisor {
    async fn health_report(&self) -> std::result::Result<ManagedPluginHealth, AstrError> {
        let report = Supervisor::health_report(self).await?;
        Ok(match report.health {
            SupervisorHealth::Healthy => ManagedPluginHealth {
                health: PluginHealth::Healthy,
                message: None,
            },
            SupervisorHealth::Unavailable => ManagedPluginHealth {
                health: PluginHealth::Unavailable,
                message: report.message,
            },
        })
    }
}

/// 带治理的插件能力调用器
///
/// 包装原始插件调用器，在调用前后更新 `PluginRegistry` 中的运行时统计，
/// 并在插件不健康时短路返回失败。
struct GovernedPluginInvoker {
    plugin_name: String,
    inner: Arc<dyn CapabilityInvoker>,
    plugin_registry: Arc<PluginRegistry>,
}

#[async_trait]
impl CapabilityInvoker for GovernedPluginInvoker {
    fn descriptor(&self) -> CapabilityDescriptor {
        self.inner.descriptor()
    }

    async fn invoke(
        &self,
        payload: Value,
        ctx: &astrcode_core::CapabilityContext,
    ) -> astrcode_core::Result<CapabilityExecutionResult> {
        // 健康检查：插件不健康时短路返回失败，避免调用已失效的插件
        if let Some(entry) = self.plugin_registry.get(&self.plugin_name) {
            if matches!(entry.health, PluginHealth::Unavailable) {
                return Ok(CapabilityExecutionResult::failure(
                    self.inner.descriptor().name,
                    entry
                        .failure
                        .unwrap_or_else(|| format!("plugin '{}' is unavailable", self.plugin_name)),
                    Value::Null,
                ));
            }
        }

        // 执行调用并记录耗时和结果
        let started_at = Instant::now();
        let invocation = self.inner.invoke(payload, ctx).await;
        // 运行时面板和故障信息会直接消费这个字段，使用本地时区避免用户看到“时间错了”。
        let checked_at = format_local_rfc3339(Utc::now());
        match &invocation {
            Ok(result) if result.success => {
                self.plugin_registry
                    .record_runtime_success(&self.plugin_name, checked_at);
            },
            Ok(result) => {
                self.plugin_registry.record_runtime_failure(
                    &self.plugin_name,
                    result
                        .error
                        .clone()
                        .unwrap_or_else(|| "plugin invocation returned failure".to_string()),
                    checked_at,
                );
                log::warn!(
                    "plugin '{}' capability '{}' failed in {}ms",
                    self.plugin_name,
                    result.capability_name,
                    started_at.elapsed().as_millis()
                );
            },
            Err(error) => {
                self.plugin_registry.record_runtime_failure(
                    &self.plugin_name,
                    error.to_string(),
                    checked_at,
                );
                log::warn!(
                    "plugin '{}' invocation raised error after {}ms: {}",
                    self.plugin_name,
                    started_at.elapsed().as_millis(),
                    error
                );
            },
        }
        invocation
    }
}

/// 插件初始化器 trait
///
/// 抽象插件的启动和握手过程，便于测试和不同插件类型的扩展。
#[async_trait]
pub(crate) trait PluginInitializer: Send + Sync {
    async fn initialize(
        &self,
        manifest: &PluginManifest,
    ) -> std::result::Result<LoadedPlugin, AstrError>;
}

/// 基于 Supervisor 的插件初始化器
///
/// 使用 `PluginLoader` 启动插件进程并完成 MCP 握手。
#[derive(Clone)]
pub(crate) struct SupervisorPluginInitializer {
    loader: PluginLoader,
}

impl SupervisorPluginInitializer {
    /// 创建新的 Supervisor 插件初始化器。
    ///
    /// `search_paths` 指定插件可执行文件的搜索路径列表。
    pub(crate) fn new(search_paths: Vec<PathBuf>) -> Self {
        Self {
            loader: PluginLoader { search_paths },
        }
    }
}

#[async_trait]
impl PluginInitializer for SupervisorPluginInitializer {
    /// 初始化单个插件：启动进程、完成 MCP 握手、提取能力。
    async fn initialize(
        &self,
        manifest: &PluginManifest,
    ) -> std::result::Result<LoadedPlugin, AstrError> {
        let supervisor = Arc::new(
            self.loader
                .start(manifest, host_peer_descriptor(), None)
                .await?,
        );
        Ok(LoadedPlugin {
            component: supervisor.clone(),
            capabilities: supervisor.core_capabilities(),
            declared_skills: supervisor.declared_skills(),
            contribution: RuntimeSurfaceContribution {
                capability_invokers: supervisor.capability_invokers(),
                prompt_declarations: normalize_prompt_declarations(
                    &manifest.name,
                    &supervisor.remote_initialize().metadata,
                ),
                skills: Vec::new(),
                hook_handlers: build_plugin_hook_handlers(
                    &manifest.name,
                    &supervisor.remote_initialize().handlers,
                    supervisor.clone(),
                ),
            },
        })
    }
}

/// 组装完整的运行时能力面（capability surface）。
///
/// 将内置工具和外部插件的能力统一注册到 `CapabilityRouter` 中。
///
/// ## 流程
///
/// 1. 收集内置工具（shell, readFile, writeFile 等）的 invoker
/// 2. 对插件清单按名称/版本排序（保证确定性冲突解决）
/// 3. 逐个初始化插件：通过 `PluginInitializer` 启动进程并握手
/// 4. 对每个插件分三种结果：
///    - **成功**: 注册其能力到 router，记录为活跃插件
///    - **能力冲突**: 如果能力名已被注册，跳过该插件，记录为健康冲突
///    - **初始化失败**: 跳过该插件，记录为不健康
/// 5. 返回组装结果：router + 所有插件条目（含活跃/跳过/失败的） + 需要管理的组件
///
/// ## 关键约束
///
/// - 能力名必须全局唯一：先到先得，排序保证确定性
/// - 插件初始化失败不阻塞其他插件
/// - 返回的 `managed_components` 需要在 shutdown 时有序关闭
pub(crate) async fn assemble_runtime_surface<I>(
    manifests: Vec<PluginManifest>,
    initializer: &I,
    plugin_registry: Arc<PluginRegistry>,
    builtin_skills: Vec<SkillSpec>,
    subagent_executor: Arc<dyn astrcode_runtime_agent_tool::SubAgentExecutor>,
    collaboration_executor: Arc<dyn astrcode_runtime_agent_tool::CollaborationExecutor>,
) -> std::result::Result<AssembledRuntimeSurface, AstrError>
where
    I: PluginInitializer,
{
    let skill_catalog = Arc::new(SkillCatalog::new(builtin_skills.clone()));
    let built_in_invokers = built_in_capability_invokers(
        Arc::clone(&skill_catalog),
        subagent_executor,
        collaboration_executor,
    )?;
    let mut registered_capability_names: HashSet<String> = built_in_invokers
        .iter()
        .map(|invoker| invoker.descriptor().name)
        .collect();
    let mut builder = CapabilityRouter::builder();
    for invoker in built_in_invokers {
        builder = builder.register_invoker(invoker);
    }
    let mut base_skills = builtin_skills;
    let mut plugin_entries = BTreeMap::new();
    let mut prompt_declarations = Vec::new();
    let mut hook_handlers = Vec::new();
    let mut managed_components = Vec::new();
    let mut active_plugins = Vec::new();
    let surface_materialization_id = Utc::now().timestamp_millis().to_string();

    let mut manifests = manifests;
    manifests.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.version.cmp(&right.version))
            .then_with(|| left.executable.cmp(&right.executable))
    });

    for manifest in manifests {
        plugin_entries.insert(manifest.name.clone(), make_discovered_entry(&manifest));

        let loaded_plugin = match initializer.initialize(&manifest).await {
            Ok(loaded_plugin) => loaded_plugin,
            Err(error) => {
                log::error!("failed to initialize plugin '{}': {}", manifest.name, error);
                plugin_entries.insert(
                    manifest.name.clone(),
                    make_failed_entry(manifest, Vec::new(), error.to_string(), Vec::new()),
                );
                continue;
            },
        };

        if let Some(failure) = invalid_capability_reason(&loaded_plugin.capabilities) {
            log::error!("failed to register plugin '{}': {}", manifest.name, failure);
            plugin_entries.insert(
                manifest.name.clone(),
                make_failed_entry(
                    manifest.clone(),
                    loaded_plugin.capabilities.clone(),
                    failure,
                    Vec::new(),
                ),
            );
            if let Err(error) = loaded_plugin.component.shutdown_component().await {
                log::warn!(
                    "failed to shut down rejected plugin component '{}': {}",
                    loaded_plugin.component.component_name(),
                    error
                );
            }
            continue;
        }

        if let Some(conflict) =
            conflicting_capability_name(&registered_capability_names, &loaded_plugin.capabilities)
        {
            let failure = format!(
                "capability '{}' conflicts with an already registered capability",
                conflict
            );
            log::error!("failed to register plugin '{}': {}", manifest.name, failure);
            plugin_entries.insert(
                manifest.name.clone(),
                make_failed_entry(
                    manifest.clone(),
                    loaded_plugin.capabilities.clone(),
                    failure,
                    Vec::new(),
                ),
            );
            if let Err(error) = loaded_plugin.component.shutdown_component().await {
                log::warn!(
                    "failed to shut down rejected plugin component '{}': {}",
                    loaded_plugin.component.component_name(),
                    error
                );
            }
            continue;
        }

        let available_tool_names = registered_capability_names
            .iter()
            .cloned()
            .chain(
                loaded_plugin
                    .capabilities
                    .iter()
                    .map(|capability| capability.name.clone()),
            )
            .collect::<HashSet<_>>();
        let materialized_skills = materialize_plugin_skills(
            &manifest,
            &surface_materialization_id,
            &loaded_plugin.declared_skills,
            &available_tool_names,
        );
        let plugin_warnings = materialized_skills.warnings.clone();
        let mut loaded_plugin = loaded_plugin;
        loaded_plugin.contribution.skills = materialized_skills.skills.clone();
        base_skills = merge_skill_layers(base_skills, loaded_plugin.contribution.skills.clone());

        for capability in &loaded_plugin.capabilities {
            registered_capability_names.insert(capability.name.clone());
        }
        for invoker in loaded_plugin.contribution.capability_invokers {
            builder = builder.register_invoker(Arc::new(GovernedPluginInvoker {
                plugin_name: manifest.name.clone(),
                inner: invoker,
                plugin_registry: Arc::clone(&plugin_registry),
            }));
        }
        prompt_declarations.extend(loaded_plugin.contribution.prompt_declarations.clone());
        hook_handlers.extend(loaded_plugin.contribution.hook_handlers.clone());
        plugin_entries.insert(
            manifest.name.clone(),
            make_initialized_entry(
                &manifest,
                loaded_plugin.capabilities.clone(),
                plugin_warnings,
            ),
        );
        log::info!("loaded plugin '{}'", manifest.name);
        managed_components
            .push(loaded_plugin.component.clone() as Arc<dyn ManagedRuntimeComponent>);
        managed_components.extend(materialized_skills.managed_components);
        active_plugins.push(ActivePluginRuntime {
            name: manifest.name,
            component: loaded_plugin.component,
        });
    }

    skill_catalog.replace_base_skills(base_skills.clone());

    Ok(AssembledRuntimeSurface {
        router: builder.build()?,
        skill_catalog,
        prompt_declarations,
        hook_handlers,
        plugin_entries: plugin_entries.into_values().collect(),
        managed_components,
        active_plugins,
    })
}

/// 插件加载结果（不含路由器）。
///
/// 用于后台加载场景，插件能力会动态注册到已有的路由器中。
pub(crate) struct AssembledPlugins {
    /// 加载成功的插件能力调用器
    pub(crate) invokers: Vec<Arc<dyn CapabilityInvoker>>,
    /// Prompt 声明
    pub(crate) prompt_declarations: Vec<PromptDeclaration>,
    pub(crate) hook_handlers: Vec<Arc<dyn HookHandler>>,
    /// 物化后的 skills
    pub(crate) skills: Vec<SkillSpec>,
    /// 插件条目列表
    pub(crate) plugin_entries: Vec<PluginEntry>,
    /// 需要管理的组件
    pub(crate) managed_components: Vec<Arc<dyn ManagedRuntimeComponent>>,
    /// 活跃插件运行时
    pub(crate) active_plugins: Vec<ActivePluginRuntime>,
    /// 加载统计
    pub(crate) stats: PluginLoadStats,
}

/// 插件加载统计。
#[derive(Debug, Clone, Default)]
pub(crate) struct PluginLoadStats {
    pub(crate) loaded_count: usize,
    pub(crate) failed_count: usize,
}

/// 仅加载插件（不包含内置能力），用于后台加载场景。
///
/// 与 `assemble_runtime_surface` 不同，此函数：
/// - 不创建新的路由器
/// - 不注册内置能力
/// - 返回能力调用器列表，由调用方决定如何注册
///
/// 用于启动时先创建内置能力的服务，然后在后台加载插件。
pub(crate) async fn assemble_plugins_only<I>(
    manifests: Vec<PluginManifest>,
    initializer: &I,
    plugin_registry: Arc<PluginRegistry>,
    existing_capability_names: HashSet<String>,
) -> std::result::Result<AssembledPlugins, AstrError>
where
    I: PluginInitializer,
{
    let mut registered_capability_names = existing_capability_names;
    let mut plugin_entries = BTreeMap::new();
    let mut prompt_declarations = Vec::new();
    let mut managed_components = Vec::new();
    let mut active_plugins = Vec::new();
    let mut invokers: Vec<Arc<dyn CapabilityInvoker>> = Vec::new();
    let mut hook_handlers: Vec<Arc<dyn HookHandler>> = Vec::new();
    let mut all_skills: Vec<SkillSpec> = Vec::new();
    let mut loaded_count = 0usize;
    let mut failed_count = 0usize;
    let surface_materialization_id = Utc::now().timestamp_millis().to_string();

    let mut manifests = manifests;
    manifests.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.version.cmp(&right.version))
            .then_with(|| left.executable.cmp(&right.executable))
    });

    for manifest in manifests {
        plugin_entries.insert(manifest.name.clone(), make_discovered_entry(&manifest));

        let loaded_plugin = match initializer.initialize(&manifest).await {
            Ok(loaded_plugin) => loaded_plugin,
            Err(error) => {
                log::error!("failed to initialize plugin '{}': {}", manifest.name, error);
                plugin_entries.insert(
                    manifest.name.clone(),
                    make_failed_entry(manifest, Vec::new(), error.to_string(), Vec::new()),
                );
                failed_count += 1;
                continue;
            },
        };

        if let Some(failure) = invalid_capability_reason(&loaded_plugin.capabilities) {
            log::error!("failed to register plugin '{}': {}", manifest.name, failure);
            plugin_entries.insert(
                manifest.name.clone(),
                make_failed_entry(
                    manifest.clone(),
                    loaded_plugin.capabilities.clone(),
                    failure,
                    Vec::new(),
                ),
            );
            if let Err(error) = loaded_plugin.component.shutdown_component().await {
                log::warn!(
                    "failed to shut down rejected plugin component '{}': {}",
                    loaded_plugin.component.component_name(),
                    error
                );
            }
            failed_count += 1;
            continue;
        }

        if let Some(conflict) =
            conflicting_capability_name(&registered_capability_names, &loaded_plugin.capabilities)
        {
            let failure = format!(
                "capability '{}' conflicts with an already registered capability",
                conflict
            );
            log::error!("failed to register plugin '{}': {}", manifest.name, failure);
            plugin_entries.insert(
                manifest.name.clone(),
                make_failed_entry(
                    manifest.clone(),
                    loaded_plugin.capabilities.clone(),
                    failure,
                    Vec::new(),
                ),
            );
            if let Err(error) = loaded_plugin.component.shutdown_component().await {
                log::warn!(
                    "failed to shut down rejected plugin component '{}': {}",
                    loaded_plugin.component.component_name(),
                    error
                );
            }
            failed_count += 1;
            continue;
        }

        // 更新已注册能力名称集合
        for capability in &loaded_plugin.capabilities {
            registered_capability_names.insert(capability.name.clone());
        }

        // 构建可用工具名称集合（用于 skill 物化时的 allowed_tools 验证）
        let available_tool_names = registered_capability_names.clone();

        // 物化插件声明的 skills
        let materialized_skills = materialize_plugin_skills(
            &manifest,
            &surface_materialization_id,
            &loaded_plugin.declared_skills,
            &available_tool_names,
        );
        let plugin_warnings = materialized_skills.warnings.clone();

        // 合并 skills
        all_skills = merge_skill_layers(all_skills, materialized_skills.skills.clone());

        // 收集调用器
        for invoker in loaded_plugin.contribution.capability_invokers {
            invokers.push(Arc::new(GovernedPluginInvoker {
                plugin_name: manifest.name.clone(),
                inner: invoker,
                plugin_registry: Arc::clone(&plugin_registry),
            }));
        }

        prompt_declarations.extend(loaded_plugin.contribution.prompt_declarations.clone());
        hook_handlers.extend(loaded_plugin.contribution.hook_handlers.clone());
        plugin_entries.insert(
            manifest.name.clone(),
            make_initialized_entry(
                &manifest,
                loaded_plugin.capabilities.clone(),
                plugin_warnings,
            ),
        );
        log::info!("loaded plugin '{}'", manifest.name);
        managed_components
            .push(loaded_plugin.component.clone() as Arc<dyn ManagedRuntimeComponent>);
        managed_components.extend(materialized_skills.managed_components);
        active_plugins.push(ActivePluginRuntime {
            name: manifest.name,
            component: loaded_plugin.component,
        });
        loaded_count += 1;
    }

    Ok(AssembledPlugins {
        invokers,
        prompt_declarations,
        hook_handlers,
        skills: all_skills,
        plugin_entries: plugin_entries.into_values().collect(),
        managed_components,
        active_plugins,
        stats: PluginLoadStats {
            loaded_count,
            failed_count,
        },
    })
}

/// 检查插件能力列表中是否存在与已注册能力冲突的名称。
///
/// 返回第一个冲突的能力名，如果没有冲突则返回 `None`。
/// 同时检查插件内部是否存在重复的能力名。
pub(crate) fn conflicting_capability_name(
    registered_capability_names: &HashSet<String>,
    capabilities: &[CapabilityDescriptor],
) -> Option<String> {
    let mut plugin_local_names = HashSet::new();
    for capability in capabilities {
        if registered_capability_names.contains(&capability.name)
            || !plugin_local_names.insert(capability.name.clone())
        {
            return Some(capability.name.clone());
        }
    }
    None
}

/// 创建"已发现"状态的插件条目。
///
/// 表示插件清单已被扫描到，但尚未初始化。
fn make_discovered_entry(manifest: &PluginManifest) -> PluginEntry {
    PluginEntry {
        manifest: manifest.clone(),
        state: astrcode_core::PluginState::Discovered,
        health: PluginHealth::Unknown,
        failure_count: 0,
        capabilities: Vec::new(),
        failure: None,
        warnings: Vec::new(),
        last_checked_at: None,
    }
}

/// 创建"失败"状态的插件条目。
///
/// 表示插件初始化或注册过程中发生了错误。
fn make_failed_entry(
    manifest: PluginManifest,
    capabilities: Vec<CapabilityDescriptor>,
    failure: String,
    warnings: Vec<String>,
) -> PluginEntry {
    PluginEntry {
        manifest,
        state: astrcode_core::PluginState::Failed,
        health: PluginHealth::Unavailable,
        failure_count: 1,
        capabilities,
        failure: Some(failure),
        warnings,
        last_checked_at: None,
    }
}

/// 创建"已初始化"状态的插件条目。
///
/// 表示插件成功加载并注册到路由器中。
fn make_initialized_entry(
    manifest: &PluginManifest,
    capabilities: Vec<CapabilityDescriptor>,
    warnings: Vec<String>,
) -> PluginEntry {
    PluginEntry {
        manifest: manifest.clone(),
        state: astrcode_core::PluginState::Initialized,
        health: PluginHealth::Healthy,
        failure_count: 0,
        capabilities,
        failure: None,
        warnings,
        last_checked_at: None,
    }
}

/// 检查能力列表是否存在无效的条目。
///
/// 返回第一个验证失败的能力的错误信息，如果全部有效则返回 `None`。
fn invalid_capability_reason(capabilities: &[CapabilityDescriptor]) -> Option<String> {
    capabilities.iter().find_map(|capability| {
        capability.validate().err().map(|error| {
            let name = capability.name.trim();
            let label = if name.is_empty() { "<unnamed>" } else { name };
            format!("capability '{}' is invalid: {}", label, error)
        })
    })
}

fn host_peer_descriptor() -> PeerDescriptor {
    PeerDescriptor {
        id: "astrcode-runtime".to_string(),
        name: "astrcode-runtime".to_string(),
        role: PeerRole::Supervisor,
        version: env!("CARGO_PKG_VERSION").to_string(),
        supported_profiles: vec!["coding".to_string()],
        metadata: serde_json::Value::Null,
    }
}

fn normalize_prompt_declarations(plugin_name: &str, metadata: &Value) -> Vec<PromptDeclaration> {
    let Some(raw_declarations) = metadata.get("promptDeclarations") else {
        return Vec::new();
    };
    let Some(raw_declarations) = raw_declarations.as_array() else {
        log::warn!(
            "plugin '{}' metadata.promptDeclarations must be an array, got {}",
            plugin_name,
            raw_declarations
        );
        return Vec::new();
    };

    let mut seen_block_ids = HashSet::new();
    let mut declarations = Vec::new();
    for (index, raw_declaration) in raw_declarations.iter().enumerate() {
        match serde_json::from_value::<PromptDeclaration>(raw_declaration.clone()) {
            Ok(mut declaration) => {
                if let Some(message) = validate_prompt_declaration(&declaration) {
                    log::warn!(
                        "plugin '{}' prompt declaration {} rejected: {}",
                        plugin_name,
                        index,
                        message
                    );
                    continue;
                }
                if !seen_block_ids.insert(declaration.block_id.clone()) {
                    log::warn!(
                        "plugin '{}' prompt declaration '{}' is duplicated; keeping the first \
                         declaration only",
                        plugin_name,
                        declaration.block_id
                    );
                    continue;
                }
                declaration.source = PromptDeclarationSource::Plugin;
                declaration.origin = Some(plugin_name.to_string());
                declarations.push(declaration);
            },
            Err(error) => {
                log::warn!(
                    "plugin '{}' prompt declaration {} is invalid JSON schema: {}",
                    plugin_name,
                    index,
                    error
                );
            },
        }
    }

    declarations
}

fn validate_prompt_declaration(declaration: &PromptDeclaration) -> Option<&'static str> {
    if declaration.block_id.trim().is_empty() {
        return Some("blockId must not be empty");
    }
    if declaration.title.trim().is_empty() {
        return Some("title must not be empty");
    }
    if declaration.content.trim().is_empty() {
        return Some("content must not be empty");
    }
    None
}
