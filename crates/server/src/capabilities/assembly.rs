use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use astrcode_core::{
    plugin::PluginEntry, AstrError, CapabilityDescriptor, CapabilityExecutionResult,
    CapabilityInvoker, CapabilityRouter, ManagedRuntimeComponent, PluginHealth, PluginManifest,
    PluginRegistry, RuntimeCoordinator, RuntimeHandle, ToolRegistry,
};
use astrcode_plugin::{PluginLoader, Supervisor, SupervisorHealth};
use astrcode_protocol::plugin::{PeerDescriptor, PeerRole};
use astrcode_runtime::{RuntimeService, ServiceError};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;

use super::{
    ActivePluginRuntime, ManagedPluginComponent, ManagedPluginHealth, RuntimeBootstrap,
    RuntimeGovernance,
};

pub(super) struct AssembledRuntimeSurface {
    pub(super) router: CapabilityRouter,
    pub(super) plugin_entries: Vec<PluginEntry>,
    pub(super) managed_components: Vec<Arc<dyn ManagedRuntimeComponent>>,
    pub(super) active_plugins: Vec<ActivePluginRuntime>,
}

pub(super) struct LoadedPlugin {
    pub(super) component: Arc<dyn ManagedPluginComponent>,
    pub(super) capabilities: Vec<CapabilityDescriptor>,
    pub(super) invokers: Vec<Arc<dyn CapabilityInvoker>>,
}

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

        let started_at = Instant::now();
        let invocation = self.inner.invoke(payload, ctx).await;
        let checked_at = Utc::now().to_rfc3339();
        match &invocation {
            Ok(result) if result.success => {
                self.plugin_registry
                    .record_runtime_success(&self.plugin_name, checked_at);
            }
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
            }
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
            }
        }
        invocation
    }
}

#[async_trait]
pub(super) trait PluginInitializer: Send + Sync {
    async fn initialize(
        &self,
        manifest: &PluginManifest,
    ) -> std::result::Result<LoadedPlugin, AstrError>;
}

pub(super) struct SupervisorPluginInitializer {
    loader: PluginLoader,
}

impl SupervisorPluginInitializer {
    pub(super) fn new(search_paths: Vec<PathBuf>) -> Self {
        Self {
            loader: PluginLoader { search_paths },
        }
    }
}

#[async_trait]
impl PluginInitializer for SupervisorPluginInitializer {
    async fn initialize(
        &self,
        manifest: &PluginManifest,
    ) -> std::result::Result<LoadedPlugin, AstrError> {
        let supervisor = Arc::new(
            self.loader
                .start(manifest, server_peer_descriptor(), None)
                .await?,
        );
        Ok(LoadedPlugin {
            component: supervisor.clone(),
            capabilities: supervisor.core_capabilities(),
            invokers: supervisor.capability_invokers(),
        })
    }
}

pub(crate) async fn bootstrap_runtime() -> std::result::Result<RuntimeBootstrap, AstrError> {
    let search_paths = configured_plugin_paths();
    let manifests = discover_plugin_manifests_in(&search_paths)?;
    let initializer = SupervisorPluginInitializer::new(search_paths);
    bootstrap_runtime_from_manifests(manifests, &initializer).await
}

pub(super) async fn bootstrap_runtime_from_manifests<I>(
    manifests: Vec<PluginManifest>,
    initializer: &I,
) -> std::result::Result<RuntimeBootstrap, AstrError>
where
    I: PluginInitializer,
{
    let plugin_registry = Arc::new(PluginRegistry::default());
    let assembled =
        assemble_runtime_surface(manifests, initializer, Arc::clone(&plugin_registry)).await?;
    let capability_surface = assembled.router.descriptors();
    plugin_registry.replace_snapshot(assembled.plugin_entries);
    let service = Arc::new(
        RuntimeService::from_capabilities(assembled.router).map_err(service_error_to_astr)?,
    );
    let runtime: Arc<dyn RuntimeHandle> = service.clone();
    let coordinator = Arc::new(
        RuntimeCoordinator::new(runtime, plugin_registry, capability_surface)
            .with_managed_components(assembled.managed_components),
    );
    let governance = Arc::new(RuntimeGovernance::new(
        Arc::clone(&service),
        Arc::clone(&coordinator),
        assembled.active_plugins,
    ));

    Ok(RuntimeBootstrap {
        service,
        coordinator,
        governance,
    })
}

pub(super) async fn assemble_runtime_surface<I>(
    manifests: Vec<PluginManifest>,
    initializer: &I,
    plugin_registry: Arc<PluginRegistry>,
) -> std::result::Result<AssembledRuntimeSurface, AstrError>
where
    I: PluginInitializer,
{
    let built_in_registry = built_in_tool_registry();
    let mut registered_capability_names: HashSet<String> =
        built_in_registry.names().into_iter().collect();
    let mut builder = CapabilityRouter::builder().register_tool_registry(built_in_registry);
    let mut plugin_entries = BTreeMap::new();
    let mut managed_components = Vec::new();
    let mut active_plugins = Vec::new();

    let mut manifests = manifests;
    manifests.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.version.cmp(&right.version))
            .then_with(|| left.executable.cmp(&right.executable))
    });

    for manifest in manifests {
        plugin_entries.insert(
            manifest.name.clone(),
            PluginEntry {
                manifest: manifest.clone(),
                state: astrcode_core::PluginState::Discovered,
                health: PluginHealth::Unknown,
                failure_count: 0,
                capabilities: Vec::new(),
                failure: None,
                last_checked_at: None,
            },
        );

        let loaded_plugin = match initializer.initialize(&manifest).await {
            Ok(loaded_plugin) => loaded_plugin,
            Err(error) => {
                log::error!("failed to initialize plugin '{}': {}", manifest.name, error);
                plugin_entries.insert(
                    manifest.name.clone(),
                    PluginEntry {
                        manifest,
                        state: astrcode_core::PluginState::Failed,
                        health: PluginHealth::Unavailable,
                        failure_count: 1,
                        capabilities: Vec::new(),
                        failure: Some(error.to_string()),
                        last_checked_at: None,
                    },
                );
                continue;
            }
        };

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
                PluginEntry {
                    manifest: manifest.clone(),
                    state: astrcode_core::PluginState::Failed,
                    health: PluginHealth::Unavailable,
                    failure_count: 1,
                    capabilities: loaded_plugin.capabilities.clone(),
                    failure: Some(failure),
                    last_checked_at: None,
                },
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

        for capability in &loaded_plugin.capabilities {
            registered_capability_names.insert(capability.name.clone());
        }
        for invoker in loaded_plugin.invokers {
            builder = builder.register_invoker(Arc::new(GovernedPluginInvoker {
                plugin_name: manifest.name.clone(),
                inner: invoker,
                plugin_registry: Arc::clone(&plugin_registry),
            }));
        }
        plugin_entries.insert(
            manifest.name.clone(),
            PluginEntry {
                manifest: manifest.clone(),
                state: astrcode_core::PluginState::Initialized,
                health: PluginHealth::Healthy,
                failure_count: 0,
                capabilities: loaded_plugin.capabilities.clone(),
                failure: None,
                last_checked_at: None,
            },
        );
        log::info!("loaded plugin '{}'", manifest.name);
        managed_components
            .push(loaded_plugin.component.clone() as Arc<dyn ManagedRuntimeComponent>);
        active_plugins.push(ActivePluginRuntime {
            name: manifest.name,
            component: loaded_plugin.component,
        });
    }

    Ok(AssembledRuntimeSurface {
        router: builder.build()?,
        plugin_entries: plugin_entries.into_values().collect(),
        managed_components,
        active_plugins,
    })
}

pub(super) fn configured_plugin_paths() -> Vec<PathBuf> {
    match std::env::var_os("ASTRCODE_PLUGIN_DIRS") {
        Some(raw_paths) => std::env::split_paths(&raw_paths).collect(),
        None => Vec::new(),
    }
}

pub(super) fn discover_plugin_manifests_in(
    search_paths: &[PathBuf],
) -> std::result::Result<Vec<PluginManifest>, AstrError> {
    if search_paths.is_empty() {
        return Ok(Vec::new());
    }
    PluginLoader {
        search_paths: search_paths.to_vec(),
    }
    .discover()
}

pub(super) fn conflicting_capability_name(
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

fn service_error_to_astr(error: ServiceError) -> AstrError {
    match error {
        ServiceError::NotFound(message)
        | ServiceError::Conflict(message)
        | ServiceError::InvalidInput(message) => AstrError::Validation(message),
        ServiceError::Internal(error) => AstrError::Internal(error.to_string()),
    }
}

fn built_in_tool_registry() -> ToolRegistry {
    ToolRegistry::builder()
        .register(Box::new(astrcode_tools::tools::shell::ShellTool::default()))
        .register(Box::new(
            astrcode_tools::tools::list_dir::ListDirTool::default(),
        ))
        .register(Box::new(
            astrcode_tools::tools::read_file::ReadFileTool::default(),
        ))
        .register(Box::new(
            astrcode_tools::tools::write_file::WriteFileTool::default(),
        ))
        .register(Box::new(
            astrcode_tools::tools::edit_file::EditFileTool::default(),
        ))
        .register(Box::new(
            astrcode_tools::tools::find_files::FindFilesTool::default(),
        ))
        .register(Box::new(astrcode_tools::tools::grep::GrepTool::default()))
        .build()
}

fn server_peer_descriptor() -> PeerDescriptor {
    PeerDescriptor {
        id: "astrcode-server".to_string(),
        name: "astrcode-server".to_string(),
        role: PeerRole::Supervisor,
        version: env!("CARGO_PKG_VERSION").to_string(),
        supported_profiles: vec!["coding".to_string()],
        metadata: serde_json::Value::Null,
    }
}
