use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use astrcode_core::{
    plugin::PluginEntry, AstrError, CapabilityDescriptor, CapabilityExecutionResult,
    CapabilityInvoker, CapabilityRouter, ManagedRuntimeComponent, PluginHealth, PluginManifest,
    PluginRegistry,
};
use astrcode_plugin::{PluginLoader, Supervisor, SupervisorHealth};
use astrcode_protocol::plugin::{PeerDescriptor, PeerRole};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;

use crate::builtin_capabilities::built_in_capability_invokers;

pub(crate) struct AssembledRuntimeSurface {
    pub(crate) router: CapabilityRouter,
    pub(crate) plugin_entries: Vec<PluginEntry>,
    pub(crate) managed_components: Vec<Arc<dyn ManagedRuntimeComponent>>,
    pub(crate) active_plugins: Vec<ActivePluginRuntime>,
}

#[derive(Clone)]
pub(crate) struct ActivePluginRuntime {
    pub(crate) name: String,
    pub(crate) component: Arc<dyn ManagedPluginComponent>,
}

pub(crate) struct LoadedPlugin {
    pub(crate) component: Arc<dyn ManagedPluginComponent>,
    pub(crate) capabilities: Vec<CapabilityDescriptor>,
    pub(crate) invokers: Vec<Arc<dyn CapabilityInvoker>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManagedPluginHealth {
    pub(crate) health: PluginHealth,
    pub(crate) message: Option<String>,
}

#[async_trait]
pub(crate) trait ManagedPluginComponent: ManagedRuntimeComponent {
    async fn health_report(&self) -> std::result::Result<ManagedPluginHealth, AstrError>;
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
pub(crate) trait PluginInitializer: Send + Sync {
    async fn initialize(
        &self,
        manifest: &PluginManifest,
    ) -> std::result::Result<LoadedPlugin, AstrError>;
}

pub(crate) struct SupervisorPluginInitializer {
    loader: PluginLoader,
}

impl SupervisorPluginInitializer {
    pub(crate) fn new(search_paths: Vec<PathBuf>) -> Self {
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
                .start(manifest, host_peer_descriptor(), None)
                .await?,
        );
        Ok(LoadedPlugin {
            component: supervisor.clone(),
            capabilities: supervisor.core_capabilities(),
            invokers: supervisor.capability_invokers(),
        })
    }
}

pub(crate) async fn assemble_runtime_surface<I>(
    manifests: Vec<PluginManifest>,
    initializer: &I,
    plugin_registry: Arc<PluginRegistry>,
) -> std::result::Result<AssembledRuntimeSurface, AstrError>
where
    I: PluginInitializer,
{
    let built_in_invokers = built_in_capability_invokers()?;
    let mut registered_capability_names: HashSet<String> = built_in_invokers
        .iter()
        .map(|invoker| invoker.descriptor().name)
        .collect();
    let mut builder = CapabilityRouter::builder();
    for invoker in built_in_invokers {
        builder = builder.register_invoker(invoker);
    }
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

        if let Some(failure) = invalid_capability_reason(&loaded_plugin.capabilities) {
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
