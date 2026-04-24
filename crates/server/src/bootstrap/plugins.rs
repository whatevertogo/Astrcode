//! # 插件发现、装载与物化
//!
//! 独立于主组合根的插件装配模块，负责：
//! - 发现 `search_paths` 中的 `.toml` 插件清单
//! - 启动插件进程并完成握手
//! - 将插件能力物化为 `CapabilityInvoker` 列表
//! - 更新 `PluginRegistry` 的生命周期状态
//!
//! 组合根通过 `bootstrap_plugins` 获取物化结果，
//! 不需要了解 loader/supervisor/peer 的内部细节。

use std::{
    fs,
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use astrcode_adapter_skills::collect_asset_files;
use astrcode_core::{
    AstrError, CapabilityContext, CapabilityExecutionResult, CapabilityInvoker, CapabilitySpec,
    InvocationMode, Result, SkillSource, SkillSpec, is_valid_skill_name,
};
use astrcode_governance_contract::GovernanceModeSpec;
use astrcode_plugin_host::{
    PluginDescriptor, PluginLoader, PluginManifest, PluginRegistry, PluginSourceKind, PluginType,
    ResourceCatalog,
    backend::{ExternalPluginRuntimeHandle, PluginBackendPlan},
    default_initialize_message, default_local_peer_descriptor, default_profiles,
    resources_discover,
};
use astrcode_protocol::plugin::{EventPhase, InvocationContext, SkillDescriptor, WorkspaceRef};
use astrcode_runtime_contract::ManagedRuntimeComponent;
#[cfg(test)]
use astrcode_support::hostpaths::resolve_home_dir;
use async_trait::async_trait;
use log::warn;
use reqwest::Client;
use serde_json::{Value, json};
use tokio::sync::Mutex;

/// 插件装配结果。
pub(crate) struct PluginBootstrapResult {
    /// 物化后的插件能力调用器。
    pub invokers: Vec<Arc<dyn CapabilityInvoker>>,
    /// 物化后的插件 skill。
    pub skills: Vec<SkillSpec>,
    /// 插件声明的治理 mode。
    pub modes: Vec<GovernanceModeSpec>,
    /// 插件注册表引用（治理视图使用）。
    pub registry: Arc<PluginRegistry>,
    /// 活跃的插件宿主组件列表（shutdown/reload 时需要关闭）。
    pub managed_components: Vec<Arc<dyn ManagedRuntimeComponent>>,
    /// 插件搜索路径。
    pub search_paths: Vec<PathBuf>,
    /// plugin-host 聚合出的统一资源目录。
    pub resource_catalog: ResourceCatalog,
    /// plugin-host 发现出的完整 descriptor 贡献面。
    pub descriptors: Vec<PluginDescriptor>,
}

/// 发现、装载并物化所有插件。
///
/// 流程：
/// 1. 从 search_paths 发现 .toml 插件清单
/// 2. 逐个启动插件进程并完成握手
/// 3. 从握手结果中提取 CapabilityInvoker
/// 4. 更新 PluginRegistry 状态
///
/// 容错：单个插件失败不影响其他插件，失败信息记录到 registry。
#[cfg(test)]
pub(crate) async fn bootstrap_plugins(search_paths: Vec<PathBuf>) -> PluginBootstrapResult {
    let skill_root = resolve_default_plugin_skill_root();
    bootstrap_plugins_with_skill_root(search_paths, skill_root).await
}

pub(crate) async fn bootstrap_plugins_with_skill_root(
    search_paths: Vec<PathBuf>,
    plugin_skill_root: PathBuf,
) -> PluginBootstrapResult {
    let registry = Arc::new(PluginRegistry::default());
    let loader = PluginLoader {
        search_paths: search_paths.clone(),
    };

    let mut descriptors = match loader.discover_descriptors() {
        Ok(descriptors) => descriptors,
        Err(error) => {
            log::warn!("plugin discovery failed: {error}");
            return PluginBootstrapResult {
                invokers: Vec::new(),
                skills: Vec::new(),
                modes: Vec::new(),
                registry,
                managed_components: Vec::new(),
                search_paths,
                resource_catalog: ResourceCatalog::default(),
                descriptors: Vec::new(),
            };
        },
    };
    descriptors.sort_by(|left, right| left.plugin_id.cmp(&right.plugin_id));
    log::info!("discovered {} plugin(s)", descriptors.len());

    let local_peer = default_local_peer_descriptor();
    let init_message =
        default_initialize_message(local_peer.clone(), Vec::new(), default_profiles());

    let mut all_invokers: Vec<Arc<dyn CapabilityInvoker>> = Vec::new();
    let mut all_skills = Vec::new();
    let mut all_modes = Vec::new();
    let mut managed_components: Vec<Arc<dyn ManagedRuntimeComponent>> = Vec::new();

    for descriptor in &mut descriptors {
        let manifest = plugin_manifest_from_descriptor(descriptor);
        let name = descriptor.plugin_id.clone();
        log::info!("loading plugin '{name}'...");
        registry.record_discovered(manifest.clone());

        let bootstrap_result = match descriptor.source_kind {
            PluginSourceKind::Process | PluginSourceKind::Command => {
                bootstrap_external_plugin_runtime(descriptor, init_message.clone()).await
            },
            PluginSourceKind::Http => bootstrap_http_plugin_runtime(descriptor).await,
            PluginSourceKind::Builtin => Err(AstrError::Validation(format!(
                "plugin '{}' 不是 external plugin source，无法走 plugin-host 装配路径",
                descriptor.plugin_id
            ))),
        };

        match bootstrap_result {
            Ok(initialized) => {
                let (skills, mut warnings) = materialize_plugin_skills(
                    plugin_skill_root.as_path(),
                    &name,
                    initialized.declared_skills.clone(),
                );
                warnings.extend(initialized.warnings.clone());
                log::info!(
                    "plugin '{name}' initialized with {} capabilities, {} skills and {} modes",
                    initialized.capabilities.len(),
                    skills.len(),
                    initialized.modes.len()
                );

                registry.record_initialized(manifest, initialized.capabilities.clone(), warnings);
                apply_remote_descriptor_enrichment(descriptor, &initialized);
                all_invokers.extend(initialized.invokers);
                all_skills.extend(skills);
                all_modes.extend(initialized.modes);
                managed_components.push(initialized.runtime);
            },
            Err(error) => {
                log::error!("plugin '{name}' failed to initialize: {error}");
                registry.record_failed(
                    manifest,
                    error.to_string(),
                    descriptor.tools.clone(),
                    vec![format!("initialization failed: {error}")],
                );
            },
        }
    }

    let resource_catalog = resource_catalog_from_descriptors(&descriptors);

    PluginBootstrapResult {
        invokers: all_invokers,
        skills: all_skills,
        modes: all_modes,
        registry,
        managed_components,
        search_paths,
        resource_catalog,
        descriptors,
    }
}

fn resource_catalog_from_descriptors(descriptors: &[PluginDescriptor]) -> ResourceCatalog {
    match resources_discover(descriptors).map(|report| report.catalog) {
        Ok(catalog) => catalog,
        Err(error) => {
            warn!("plugin resource discovery failed: {error}");
            ResourceCatalog::default()
        },
    }
}

struct BootstrappedPluginRuntime {
    runtime: Arc<dyn ManagedRuntimeComponent>,
    invokers: Vec<Arc<dyn CapabilityInvoker>>,
    capabilities: Vec<CapabilitySpec>,
    declared_skills: Vec<SkillDescriptor>,
    modes: Vec<GovernanceModeSpec>,
    warnings: Vec<String>,
}

struct HostedExternalPluginRuntime {
    plugin_id: String,
    display_name: String,
    handle: Mutex<ExternalPluginRuntimeHandle>,
}

struct HostedHttpPluginRuntime {
    plugin_id: String,
    display_name: String,
    endpoint: String,
    client: Client,
}

impl HostedExternalPluginRuntime {
    fn new(plugin_id: String, display_name: String, handle: ExternalPluginRuntimeHandle) -> Self {
        Self {
            plugin_id,
            display_name,
            handle: Mutex::new(handle),
        }
    }

    async fn invoke(
        &self,
        capability_name: &str,
        payload: Value,
        ctx: &CapabilityContext,
        invocation_mode: InvocationMode,
    ) -> Result<CapabilityExecutionResult> {
        let started_at = Instant::now();
        let invocation = to_invocation_context(ctx, capability_name);
        let mut handle = self.handle.lock().await;

        if matches!(invocation_mode, InvocationMode::Streaming) {
            let events = handle
                .invoke_stream(&astrcode_protocol::plugin::InvokeMessage {
                    id: invocation.request_id.clone(),
                    capability: capability_name.to_string(),
                    input: payload,
                    context: invocation,
                    stream: true,
                })
                .await?;
            finish_stream_invocation(capability_name.to_string(), events, started_at)
        } else {
            let result = handle
                .invoke_unary(&astrcode_protocol::plugin::InvokeMessage {
                    id: invocation.request_id.clone(),
                    capability: capability_name.to_string(),
                    input: payload,
                    context: invocation,
                    stream: false,
                })
                .await?;
            let (success, error) = if result.success {
                (true, None)
            } else {
                let message = result
                    .error
                    .map(|value| value.message)
                    .unwrap_or_else(|| "plugin invocation failed".to_string());
                (false, Some(message))
            };
            Ok(CapabilityExecutionResult::from_common(
                capability_name.to_string(),
                success,
                result.output,
                None,
                astrcode_core::ExecutionResultCommon {
                    error,
                    metadata: Some(result.metadata),
                    duration_ms: started_at.elapsed().as_millis() as u64,
                    truncated: false,
                },
            ))
        }
    }
}

impl HostedHttpPluginRuntime {
    fn new(plugin_id: String, display_name: String, endpoint: String) -> Self {
        Self {
            plugin_id,
            display_name,
            endpoint,
            client: Client::new(),
        }
    }

    async fn invoke(
        &self,
        capability_name: &str,
        payload: Value,
        ctx: &CapabilityContext,
        invocation_mode: InvocationMode,
    ) -> Result<CapabilityExecutionResult> {
        if matches!(invocation_mode, InvocationMode::Streaming) {
            return Err(AstrError::Validation(format!(
                "plugin '{}' 的 HTTP backend 暂不支持流式 capability '{}'",
                self.plugin_id, capability_name
            )));
        }

        let started_at = Instant::now();
        let invocation = to_invocation_context(ctx, capability_name);
        let request = astrcode_protocol::plugin::InvokeMessage {
            id: invocation.request_id.clone(),
            capability: capability_name.to_string(),
            input: payload,
            context: invocation,
            stream: false,
        };
        let response = self
            .client
            .post(&self.endpoint)
            .json(&request)
            .send()
            .await
            .map_err(|error| {
                AstrError::http_with_source(
                    format!(
                        "failed to invoke HTTP plugin '{}' at '{}'",
                        self.plugin_id, self.endpoint
                    ),
                    is_retryable_http_error(&error),
                    error,
                )
            })?
            .error_for_status()
            .map_err(|error| {
                AstrError::http_with_source(
                    format!(
                        "HTTP plugin '{}' returned an error status from '{}'",
                        self.plugin_id, self.endpoint
                    ),
                    is_retryable_http_error(&error),
                    error,
                )
            })?;
        let result = response
            .json::<astrcode_protocol::plugin::ResultMessage>()
            .await
            .map_err(|error| {
                AstrError::http_with_source(
                    format!(
                        "failed to decode HTTP plugin response for '{}' from '{}'",
                        self.plugin_id, self.endpoint
                    ),
                    is_retryable_http_error(&error),
                    error,
                )
            })?;

        Ok(capability_execution_from_result_message(
            capability_name.to_string(),
            result,
            started_at,
        ))
    }
}

#[async_trait]
impl ManagedRuntimeComponent for HostedExternalPluginRuntime {
    fn component_name(&self) -> String {
        format!("plugin-host '{}' ({})", self.plugin_id, self.display_name)
    }

    async fn shutdown_component(&self) -> Result<()> {
        let mut handle = self.handle.lock().await;
        handle.shutdown().await
    }
}

#[async_trait]
impl ManagedRuntimeComponent for HostedHttpPluginRuntime {
    fn component_name(&self) -> String {
        format!("plugin-http '{}' ({})", self.plugin_id, self.display_name)
    }

    async fn shutdown_component(&self) -> Result<()> {
        Ok(())
    }
}

#[derive(Clone)]
struct HostedPluginCapabilityInvoker {
    runtime: Arc<HostedExternalPluginRuntime>,
    capability_spec: CapabilitySpec,
    remote_name: String,
}

#[derive(Clone)]
struct HostedHttpPluginCapabilityInvoker {
    runtime: Arc<HostedHttpPluginRuntime>,
    capability_spec: CapabilitySpec,
    remote_name: String,
}

#[async_trait]
impl CapabilityInvoker for HostedPluginCapabilityInvoker {
    fn capability_spec(&self) -> CapabilitySpec {
        self.capability_spec.clone()
    }

    async fn invoke(
        &self,
        payload: Value,
        ctx: &CapabilityContext,
    ) -> Result<CapabilityExecutionResult> {
        self.runtime
            .invoke(
                self.remote_name.as_str(),
                payload,
                ctx,
                self.capability_spec.invocation_mode,
            )
            .await
    }
}

#[async_trait]
impl CapabilityInvoker for HostedHttpPluginCapabilityInvoker {
    fn capability_spec(&self) -> CapabilitySpec {
        self.capability_spec.clone()
    }

    async fn invoke(
        &self,
        payload: Value,
        ctx: &CapabilityContext,
    ) -> Result<CapabilityExecutionResult> {
        self.runtime
            .invoke(
                self.remote_name.as_str(),
                payload,
                ctx,
                self.capability_spec.invocation_mode,
            )
            .await
    }
}

async fn bootstrap_external_plugin_runtime(
    descriptor: &PluginDescriptor,
    init_message: astrcode_protocol::plugin::InitializeMessage,
) -> Result<BootstrappedPluginRuntime> {
    if !matches!(
        descriptor.source_kind,
        PluginSourceKind::Process | PluginSourceKind::Command
    ) {
        return Err(AstrError::Validation(format!(
            "plugin '{}' 不是 external process backend，无法走 plugin-host 宿主路径",
            descriptor.plugin_id
        )));
    }

    let plan = PluginBackendPlan::from_descriptor(descriptor)?;
    let backend = plan.start_process().await?;
    let mut handle = ExternalPluginRuntimeHandle::from_backend(backend).with_initialize_state(
        astrcode_plugin_host::PluginInitializeState::new(init_message),
    );
    let remote_initialize = handle.initialize_remote().await?.clone();
    let runtime = Arc::new(HostedExternalPluginRuntime::new(
        descriptor.plugin_id.clone(),
        descriptor.display_name.clone(),
        handle,
    ));
    let capabilities = remote_initialize.capabilities.clone();
    let invokers = capabilities
        .iter()
        .cloned()
        .filter_map(|capability| {
            create_plugin_capability_invoker(Arc::clone(&runtime), capability.clone()).map_or_else(
                |error| {
                    log::error!(
                        "failed to adapt plugin capability '{}' for '{}': {}",
                        capability.name,
                        descriptor.plugin_id,
                        error
                    );
                    None
                },
                Some,
            )
        })
        .collect();

    Ok(BootstrappedPluginRuntime {
        runtime: runtime as Arc<dyn ManagedRuntimeComponent>,
        invokers,
        capabilities,
        declared_skills: remote_initialize.skills,
        modes: remote_initialize.modes,
        warnings: Vec::new(),
    })
}

async fn bootstrap_http_plugin_runtime(
    descriptor: &PluginDescriptor,
) -> Result<BootstrappedPluginRuntime> {
    if descriptor.source_ref.trim().is_empty() {
        return Err(AstrError::Validation(format!(
            "plugin '{}' 缺少 HTTP endpoint，无法走 HTTP 宿主路径",
            descriptor.plugin_id
        )));
    }

    let runtime = Arc::new(HostedHttpPluginRuntime::new(
        descriptor.plugin_id.clone(),
        descriptor.display_name.clone(),
        descriptor.source_ref.clone(),
    ));
    let invokers = descriptor
        .tools
        .iter()
        .cloned()
        .map(|capability| {
            capability.validate().map_err(|error| {
                AstrError::Validation(format!(
                    "invalid HTTP plugin capability '{}': {}",
                    capability.name, error
                ))
            })?;
            Ok(Arc::new(HostedHttpPluginCapabilityInvoker {
                runtime: Arc::clone(&runtime),
                remote_name: capability.name.to_string(),
                capability_spec: capability,
            }) as Arc<dyn CapabilityInvoker>)
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(BootstrappedPluginRuntime {
        runtime: runtime as Arc<dyn ManagedRuntimeComponent>,
        invokers,
        capabilities: descriptor.tools.clone(),
        declared_skills: Vec::new(),
        modes: descriptor.modes.clone(),
        warnings: Vec::new(),
    })
}

fn create_plugin_capability_invoker(
    runtime: Arc<HostedExternalPluginRuntime>,
    capability: CapabilitySpec,
) -> Result<Arc<dyn CapabilityInvoker>> {
    capability.validate().map_err(|error| {
        AstrError::Validation(format!(
            "invalid protocol capability wire descriptor '{}': {}",
            capability.name, error
        ))
    })?;
    Ok(Arc::new(HostedPluginCapabilityInvoker {
        runtime,
        remote_name: capability.name.to_string(),
        capability_spec: capability,
    }) as Arc<dyn CapabilityInvoker>)
}

fn apply_remote_descriptor_enrichment(
    descriptor: &mut PluginDescriptor,
    initialized: &BootstrappedPluginRuntime,
) {
    descriptor.tools = initialized.capabilities.clone();
    descriptor.modes = initialized.modes.clone();

    let mut known_skill_ids = descriptor
        .skills
        .iter()
        .map(|skill| skill.skill_id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    for skill in &initialized.declared_skills {
        if known_skill_ids.insert(skill.name.clone()) {
            descriptor
                .skills
                .push(astrcode_plugin_host::SkillDescriptor {
                    skill_id: skill.name.clone(),
                    entry_ref: format!("plugin://{}/skills/{}", descriptor.plugin_id, skill.name),
                });
        }
    }
}

fn plugin_manifest_from_descriptor(descriptor: &PluginDescriptor) -> PluginManifest {
    let mut plugin_type = Vec::new();
    if !descriptor.tools.is_empty() {
        plugin_type.push(PluginType::Tool);
    }
    if !descriptor.providers.is_empty() {
        plugin_type.push(PluginType::Provider);
    }
    if !descriptor.hooks.is_empty() {
        plugin_type.push(PluginType::Hook);
    }
    if plugin_type.is_empty() {
        plugin_type.push(PluginType::Tool);
    }

    PluginManifest {
        name: descriptor.plugin_id.clone(),
        version: descriptor.version.clone(),
        description: descriptor.display_name.clone(),
        plugin_type,
        capabilities: descriptor.tools.clone(),
        executable: descriptor.launch_command.clone(),
        args: descriptor.launch_args.clone(),
        working_dir: descriptor.working_dir.clone(),
        repository: descriptor.repository.clone(),
        resources: descriptor
            .resources
            .iter()
            .cloned()
            .map(|resource| astrcode_plugin_host::ResourceManifestEntry {
                id: resource.resource_id,
                kind: resource.kind,
                locator: resource.locator,
            })
            .collect(),
        commands: descriptor
            .commands
            .iter()
            .cloned()
            .map(|command| astrcode_plugin_host::CommandManifestEntry {
                id: command.command_id,
                entry_ref: command.entry_ref,
            })
            .collect(),
        themes: descriptor
            .themes
            .iter()
            .cloned()
            .map(|theme| astrcode_plugin_host::ThemeManifestEntry { id: theme.theme_id })
            .collect(),
        providers: descriptor
            .providers
            .iter()
            .cloned()
            .map(|provider| astrcode_plugin_host::ProviderManifestEntry {
                id: provider.provider_id,
                api_kind: provider.api_kind,
            })
            .collect(),
        prompts: descriptor
            .prompts
            .iter()
            .cloned()
            .map(|prompt| astrcode_plugin_host::PromptManifestEntry {
                id: prompt.prompt_id,
                body: prompt.body,
            })
            .collect(),
        skills: descriptor
            .skills
            .iter()
            .cloned()
            .map(|skill| astrcode_plugin_host::SkillManifestEntry {
                id: skill.skill_id,
                entry_ref: skill.entry_ref,
            })
            .collect(),
    }
}

fn finish_stream_invocation(
    capability_name: String,
    events: Vec<astrcode_protocol::plugin::EventMessage>,
    started_at: Instant,
) -> Result<CapabilityExecutionResult> {
    let mut deltas = Vec::new();
    for event in events {
        match event.phase {
            EventPhase::Started => {},
            EventPhase::Delta => deltas.push(json!({
                "event": event.event,
                "payload": event.payload,
                "seq": event.seq,
            })),
            EventPhase::Completed => {
                return Ok(CapabilityExecutionResult::from_common(
                    capability_name,
                    true,
                    event.payload,
                    None,
                    astrcode_core::ExecutionResultCommon::success(
                        Some(json!({ "streamEvents": deltas })),
                        started_at.elapsed().as_millis() as u64,
                        false,
                    ),
                ));
            },
            EventPhase::Failed => {
                let error = event
                    .error
                    .map(|value| value.message)
                    .unwrap_or_else(|| "stream invocation failed".to_string());
                return Ok(CapabilityExecutionResult::from_common(
                    capability_name,
                    false,
                    Value::Null,
                    None,
                    astrcode_core::ExecutionResultCommon::failure(
                        error,
                        Some(json!({ "streamEvents": deltas })),
                        started_at.elapsed().as_millis() as u64,
                        false,
                    ),
                ));
            },
        }
    }

    Err(AstrError::Internal(
        "plugin stream ended without terminal event".to_string(),
    ))
}

fn capability_execution_from_result_message(
    capability_name: String,
    result: astrcode_protocol::plugin::ResultMessage,
    started_at: Instant,
) -> CapabilityExecutionResult {
    let (success, error) = if result.success {
        (true, None)
    } else {
        let message = result
            .error
            .map(|value| value.message)
            .unwrap_or_else(|| "plugin invocation failed".to_string());
        (false, Some(message))
    };

    CapabilityExecutionResult::from_common(
        capability_name,
        success,
        result.output,
        None,
        astrcode_core::ExecutionResultCommon {
            error,
            metadata: Some(result.metadata),
            duration_ms: started_at.elapsed().as_millis() as u64,
            truncated: false,
        },
    )
}

fn is_retryable_http_error(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect() || error.is_body()
}

fn to_invocation_context(ctx: &CapabilityContext, capability_name: &str) -> InvocationContext {
    let working_dir = ctx.working_dir.to_string_lossy().into_owned();
    let request_id = ctx.request_id.clone().unwrap_or_else(|| {
        format!(
            "{}:{}:{}",
            ctx.session_id,
            capability_name,
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        )
    });

    InvocationContext {
        request_id,
        trace_id: ctx.trace_id.clone(),
        session_id: Some(ctx.session_id.to_string()),
        caller: None,
        workspace: Some(WorkspaceRef {
            working_dir: Some(working_dir.clone()),
            repo_root: Some(working_dir),
            branch: None,
            metadata: Value::Null,
        }),
        deadline_ms: None,
        budget: None,
        profile: ctx.profile.clone(),
        profile_context: ctx.profile_context.clone(),
        metadata: ctx.metadata.clone(),
    }
}

fn materialize_plugin_skills(
    plugin_skill_root: &Path,
    plugin_name: &str,
    skill_descriptors: Vec<SkillDescriptor>,
) -> (Vec<SkillSpec>, Vec<String>) {
    let mut skills = Vec::new();
    let mut warnings = Vec::new();

    for descriptor in skill_descriptors {
        if !is_valid_skill_name(&descriptor.name) {
            warnings.push(format!(
                "plugin '{}' declared invalid skill name '{}'; expected kebab-case",
                plugin_name, descriptor.name
            ));
            continue;
        }

        let (skill_root, asset_files, materialize_warning) =
            materialize_plugin_skill_assets(plugin_skill_root, plugin_name, &descriptor);
        if let Some(warning) = materialize_warning {
            warnings.push(warning);
        }

        skills.push(SkillSpec {
            id: descriptor.name.clone(),
            name: descriptor.name,
            description: descriptor.description,
            guide: descriptor.guide,
            skill_root,
            asset_files,
            allowed_tools: descriptor.allowed_tools,
            source: SkillSource::Plugin,
        });
    }

    (skills, warnings)
}

fn materialize_plugin_skill_assets(
    plugin_skill_root: &Path,
    plugin_name: &str,
    descriptor: &SkillDescriptor,
) -> (Option<String>, Vec<String>, Option<String>) {
    materialize_plugin_skill_assets_under_root(plugin_skill_root, plugin_name, descriptor)
}

fn materialize_plugin_skill_assets_under_root(
    plugin_skill_root: &Path,
    plugin_name: &str,
    descriptor: &SkillDescriptor,
) -> (Option<String>, Vec<String>, Option<String>) {
    let skill_root = plugin_skill_root
        .join(sanitize_path_segment(plugin_name))
        .join(&descriptor.name);

    if let Err(error) = fs::create_dir_all(&skill_root) {
        return (
            None,
            Vec::new(),
            Some(format!(
                "plugin '{}' skill '{}' could not create asset directory '{}': {}",
                plugin_name,
                descriptor.name,
                skill_root.display(),
                error
            )),
        );
    }

    let skill_markdown = format!(
        "---\nname: {}\ndescription: {}\n---\n\n{}\n",
        descriptor.name, descriptor.description, descriptor.guide
    );
    let skill_markdown_path = skill_root.join("SKILL.md");
    if let Err(error) = write_asset_if_changed(&skill_markdown_path, &skill_markdown) {
        return (
            None,
            Vec::new(),
            Some(format!(
                "plugin '{}' skill '{}' could not materialize SKILL.md: {}",
                plugin_name, descriptor.name, error
            )),
        );
    }

    for asset in &descriptor.assets {
        if !is_safe_relative_asset_path(&asset.relative_path) {
            return (
                None,
                Vec::new(),
                Some(format!(
                    "plugin '{}' skill '{}' contains unsafe asset path '{}'",
                    plugin_name, descriptor.name, asset.relative_path
                )),
            );
        }

        let asset_path = skill_root.join(
            asset
                .relative_path
                .replace('/', std::path::MAIN_SEPARATOR_STR),
        );
        if let Some(parent) = asset_path.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                return (
                    None,
                    Vec::new(),
                    Some(format!(
                        "plugin '{}' skill '{}' could not create asset directory '{}': {}",
                        plugin_name,
                        descriptor.name,
                        parent.display(),
                        error
                    )),
                );
            }
        }

        if !asset.encoding.eq_ignore_ascii_case("utf-8") {
            warn!(
                "plugin '{}' skill '{}' asset '{}' uses unsupported encoding '{}'; storing as raw \
                 text",
                plugin_name, descriptor.name, asset.relative_path, asset.encoding
            );
        }

        if let Err(error) = write_asset_if_changed(&asset_path, &asset.content) {
            return (
                None,
                Vec::new(),
                Some(format!(
                    "plugin '{}' skill '{}' could not materialize asset '{}': {}",
                    plugin_name, descriptor.name, asset.relative_path, error
                )),
            );
        }
    }

    (
        Some(skill_root.to_string_lossy().into_owned()),
        collect_asset_files(&skill_root),
        None,
    )
}

#[cfg(test)]
fn resolve_default_plugin_skill_root() -> PathBuf {
    match resolve_home_dir() {
        Ok(home_dir) => home_dir
            .join(".astrcode")
            .join("runtime")
            .join("plugin-skills"),
        Err(_) => PathBuf::from(".astrcode")
            .join("runtime")
            .join("plugin-skills"),
    }
}

fn sanitize_path_segment(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches(['-', '.', ' '])
        .to_string();

    if sanitized.is_empty() {
        "plugin".to_string()
    } else {
        sanitized
    }
}

fn is_safe_relative_asset_path(relative_path: &str) -> bool {
    let path = Path::new(relative_path);
    !path.is_absolute()
        && path.components().all(|component| {
            matches!(component, Component::Normal(_)) || matches!(component, Component::CurDir)
        })
}

fn write_asset_if_changed(path: &Path, content: &str) -> std::io::Result<()> {
    if fs::read_to_string(path).ok().as_deref() == Some(content) {
        return Ok(());
    }

    fs::write(path, content)
}

#[cfg(test)]
mod tests {
    use astrcode_core::SkillSource;
    use astrcode_governance_contract::{CapabilitySelector, ModeId};
    use astrcode_plugin_host::{PluginHealth, PluginState};
    use astrcode_protocol::plugin::{SkillAssetDescriptor, SkillDescriptor};
    use axum::{Json, Router, routing::post};
    use tokio::net::TcpListener;

    use super::*;

    fn node_protocol_script() -> &'static str {
        r#"
const readline = require('node:readline');
const rl = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
rl.on('line', (line) => {
  const msg = JSON.parse(line);
  if (msg.type === 'initialize') {
    console.log(JSON.stringify({
      type: 'result',
      id: msg.id,
      kind: 'initialize',
      success: true,
      output: {
        protocolVersion: '5',
        peer: {
          id: 'fixture-worker',
          name: 'fixture-worker',
          role: 'worker',
          version: '0.1.0',
          supportedProfiles: ['coding'],
          metadata: { fixture: true }
        },
        capabilities: [{
          name: 'tool.echo',
          kind: 'tool',
          description: 'Echo input',
          inputSchema: { type: 'object' },
          outputSchema: { type: 'object' },
          invocationMode: 'unary',
          concurrencySafe: false,
          compactClearable: false,
          profiles: ['coding'],
          tags: ['source:plugin'],
          permissions: [],
          sideEffect: 'none',
          stability: 'stable',
          metadata: null,
          maxResultInlineSize: null
        }],
        handlers: [],
        profiles: [{
          name: 'coding',
          version: '1',
          description: 'coding',
          contextSchema: null,
          metadata: null
        }],
        skills: [{
          name: 'repo-search',
          description: 'Search repo',
          guide: 'Use repo-search skill.',
          allowedTools: ['grep'],
          assets: [],
          metadata: null
        }],
        modes: [{
          id: 'plugin-review',
          name: 'Plugin Review',
          description: 'review via plugin',
          capabilitySelector: 'allTools',
          actionPolicies: {},
          childPolicy: {},
          executionPolicy: {},
          promptProgram: [],
          transitionPolicy: { allowedTargets: ['code'] }
        }],
        metadata: null
      },
      metadata: null
    }));
    return;
  }
  if (msg.type === 'invoke') {
    console.log(JSON.stringify({
      type: 'result',
      id: msg.id,
      kind: 'tool_result',
      success: true,
      output: { echoed: msg.input },
      metadata: null
    }));
  }
});
"#
    }

    #[tokio::test]
    async fn bootstrap_with_empty_paths_returns_empty() {
        let result = bootstrap_plugins(vec![]).await;
        assert!(result.invokers.is_empty());
        assert!(result.skills.is_empty());
        assert!(result.modes.is_empty());
        assert!(result.managed_components.is_empty());
        assert!(result.registry.snapshot().is_empty());
    }

    #[tokio::test]
    async fn bootstrap_with_nonexistent_path_returns_empty() {
        let result = bootstrap_plugins(vec![PathBuf::from("/nonexistent/path")]).await;
        assert!(result.invokers.is_empty());
        assert!(result.skills.is_empty());
        assert!(result.modes.is_empty());
        assert!(result.managed_components.is_empty());
    }

    #[tokio::test]
    async fn plugin_failure_is_recorded_in_registry() {
        // 创建一个包含无效 .toml 的临时目录
        let temp_dir = tempfile::tempdir().expect("tempdir should be created");
        let plugin_toml = temp_dir.path().join("broken.toml");
        std::fs::write(
            &plugin_toml,
            r#"
name = "broken-plugin"
version = "0.1.0"
description = "A broken plugin"
plugin_type = ["Tool"]
capabilities = []
executable = "nonexistent-binary"
"#,
        )
        .expect("toml should be written");

        let result = bootstrap_plugins(vec![temp_dir.path().to_path_buf()]).await;

        // 插件被发现了，但启动失败（进程不存在）
        assert!(
            result.managed_components.is_empty(),
            "不应有成功的托管插件组件"
        );
        let entries = result.registry.snapshot();
        assert_eq!(entries.len(), 1, "应有一个 registry 条目");

        let entry = &entries[0];
        assert_eq!(entry.manifest.name, "broken-plugin");
        // 插件发现成功但初始化失败
        assert!(
            matches!(entry.state, PluginState::Failed),
            "应为 Failed 状态: {:?}",
            entry.state
        );
        assert!(entry.failure.is_some(), "失败信息不应被静默吞掉");
        assert_eq!(entry.health, PluginHealth::Unavailable);
    }

    #[tokio::test]
    async fn multiple_plugins_partial_failure() {
        let temp_dir = tempfile::tempdir().expect("tempdir should be created");

        // 第一个无效插件
        std::fs::write(
            temp_dir.path().join("a-broken.toml"),
            r#"
name = "a-broken"
version = "0.1.0"
description = "Broken"
plugin_type = ["Tool"]
capabilities = []
executable = "no-such-binary"
"#,
        )
        .expect("toml should be written");

        // 第二个也是无效的（不同的名字）
        std::fs::write(
            temp_dir.path().join("b-broken.toml"),
            r#"
name = "b-broken"
version = "0.1.0"
description = "Also broken"
plugin_type = ["Tool"]
capabilities = []
executable = "also-missing"
"#,
        )
        .expect("toml should be written");

        let result = bootstrap_plugins(vec![temp_dir.path().to_path_buf()]).await;

        // 两个都失败
        let entries = result.registry.snapshot();
        assert_eq!(entries.len(), 2, "两个插件都应有 registry 条目");

        for entry in &entries {
            assert!(
                matches!(entry.state, PluginState::Failed),
                "{} 应为 Failed: {:?}",
                entry.manifest.name,
                entry.state
            );
            assert!(
                entry.failure.is_some(),
                "{} 的失败信息不应被静默吞掉",
                entry.manifest.name
            );
        }
    }

    #[tokio::test]
    async fn external_plugin_bootstrap_materializes_invokers_skills_and_modes() {
        let temp_dir = tempfile::tempdir().expect("tempdir should be created");
        let script_path = temp_dir.path().join("protocol-plugin.js");
        std::fs::write(&script_path, node_protocol_script()).expect("script should be written");
        std::fs::write(
            temp_dir.path().join("protocol-plugin.toml"),
            format!(
                r#"
name = "protocol-plugin"
version = "0.1.0"
description = "Protocol plugin"
plugin_type = ["Tool"]
capabilities = []
executable = "node"
args = ["{script_path}"]
"#,
                script_path = script_path.display().to_string().replace('\\', "\\\\")
            ),
        )
        .expect("toml should be written");

        let result = bootstrap_plugins(vec![temp_dir.path().to_path_buf()]).await;

        assert_eq!(result.managed_components.len(), 1);
        assert_eq!(result.invokers.len(), 1);
        assert_eq!(
            result.invokers[0].capability_spec().name.as_str(),
            "tool.echo"
        );
        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.skills[0].id, "repo-search");
        assert_eq!(result.skills[0].source, SkillSource::Plugin);
        assert_eq!(result.modes.len(), 1);
        assert_eq!(result.modes[0].id, ModeId::from("plugin-review"));
        assert_eq!(
            result.modes[0].capability_selector,
            CapabilitySelector::AllTools
        );
        assert!(result.descriptors.iter().any(|descriptor| {
            descriptor.plugin_id == "protocol-plugin"
                && descriptor
                    .tools
                    .iter()
                    .any(|tool| tool.name.as_str() == "tool.echo")
        }));

        let entry = result
            .registry
            .get("protocol-plugin")
            .expect("plugin registry entry should exist");
        assert!(matches!(entry.state, PluginState::Initialized));
        assert_eq!(entry.health, PluginHealth::Healthy);
    }

    #[tokio::test]
    async fn http_plugin_bootstrap_materializes_invoker_and_executes_unary_calls() {
        async fn invoke(
            Json(request): Json<astrcode_protocol::plugin::InvokeMessage>,
        ) -> Json<astrcode_protocol::plugin::ResultMessage> {
            Json(astrcode_protocol::plugin::ResultMessage {
                id: request.id,
                kind: Some("tool_result".to_string()),
                success: true,
                output: serde_json::json!({
                    "echoed": request.input,
                    "capability": request.capability,
                }),
                error: None,
                metadata: serde_json::json!({ "transport": "http" }),
            })
        }

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let address = listener
            .local_addr()
            .expect("listener should expose address");
        tokio::spawn(async move {
            axum::serve(listener, Router::new().route("/invoke", post(invoke)))
                .await
                .expect("server should run");
        });

        let mut descriptor = PluginDescriptor::builtin("remote-fetch", "Remote Fetch");
        descriptor.source_kind = PluginSourceKind::Http;
        descriptor.source_ref = format!("http://{address}/invoke");
        descriptor.tools.push(CapabilitySpec {
            name: "tool.fetch".into(),
            kind: astrcode_core::CapabilityKind::Tool,
            description: "fetch remote data".to_string(),
            input_schema: serde_json::json!({ "type": "object" }),
            output_schema: serde_json::json!({ "type": "object" }),
            invocation_mode: InvocationMode::Unary,
            concurrency_safe: false,
            compact_clearable: false,
            profiles: vec!["coding".to_string()],
            tags: Vec::new(),
            permissions: Vec::new(),
            side_effect: astrcode_core::SideEffect::External,
            stability: astrcode_core::Stability::Stable,
            metadata: serde_json::Value::Null,
            max_result_inline_size: None,
        });

        let bootstrapped = bootstrap_http_plugin_runtime(&descriptor)
            .await
            .expect("http plugin should bootstrap");

        assert_eq!(bootstrapped.invokers.len(), 1);
        let result = bootstrapped.invokers[0]
            .invoke(
                serde_json::json!({ "url": "https://example.com" }),
                &CapabilityContext {
                    request_id: Some("req-http-1".to_string()),
                    trace_id: None,
                    session_id: astrcode_core::SessionId::from("session-http"),
                    working_dir: PathBuf::from("."),
                    cancel: astrcode_core::CancelToken::new(),
                    turn_id: None,
                    agent: astrcode_core::AgentEventContext::root_execution("agent-1", "coding"),
                    current_mode_id: ModeId::from("coding").into(),
                    bound_mode_tool_contract: None,
                    execution_owner: None,
                    profile: "coding".to_string(),
                    profile_context: serde_json::Value::Null,
                    metadata: serde_json::Value::Null,
                    tool_output_sender: None,
                    event_sink: None,
                },
            )
            .await
            .expect("http plugin invoke should succeed");

        assert!(result.success);
        assert_eq!(
            result.output,
            serde_json::json!({
                "echoed": { "url": "https://example.com" },
                "capability": "tool.fetch",
            })
        );
    }

    #[test]
    fn plugin_declared_skills_materialize_into_skill_specs() {
        let temp_home = tempfile::tempdir().expect("temp home should be created");
        let plugin_skill_root = temp_home
            .path()
            .join(".astrcode")
            .join("runtime")
            .join("plugin-skills");
        let descriptor = SkillDescriptor {
            name: "repo-search".to_string(),
            description: "Search the repo".to_string(),
            guide: "Use references under ${ASTRCODE_SKILL_DIR}.".to_string(),
            allowed_tools: vec!["grep".to_string()],
            assets: vec![SkillAssetDescriptor {
                relative_path: "references/api.md".to_string(),
                content: "# API".to_string(),
                encoding: "utf-8".to_string(),
            }],
            metadata: serde_json::Value::Null,
        };
        let (skill_root, asset_files, warning) = materialize_plugin_skill_assets_under_root(
            &plugin_skill_root,
            "demo-plugin",
            &descriptor,
        );
        let (skills, warnings) =
            materialize_plugin_skills(&plugin_skill_root, "demo-plugin", vec![descriptor]);

        assert!(warning.is_none(), "direct materialization should not warn");
        assert!(
            warnings.is_empty(),
            "plugin skill materialization should not warn"
        );
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].source, SkillSource::Plugin);
        assert_eq!(asset_files, vec!["references/api.md".to_string()]);
        let skill_root = skill_root.expect("plugin skill root should be materialized");
        assert!(
            Path::new(&skill_root)
                .join("references")
                .join("api.md")
                .is_file()
        );
    }
}
