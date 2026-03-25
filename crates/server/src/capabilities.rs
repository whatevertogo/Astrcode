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
use astrcode_runtime::{RuntimeObservabilitySnapshot, RuntimeService, ServiceError};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use tokio::sync::{Mutex, RwLock};

pub(crate) struct RuntimeBootstrap {
    pub service: Arc<RuntimeService>,
    pub coordinator: Arc<RuntimeCoordinator>,
    pub governance: Arc<RuntimeGovernance>,
}

#[derive(Debug, Clone)]
pub(crate) struct RuntimeGovernanceSnapshot {
    pub runtime_name: String,
    pub runtime_kind: String,
    pub loaded_session_count: usize,
    pub running_session_ids: Vec<String>,
    pub plugin_search_paths: Vec<PathBuf>,
    pub metrics: RuntimeObservabilitySnapshot,
    pub capabilities: Vec<CapabilityDescriptor>,
    pub plugins: Vec<PluginEntry>,
}

#[derive(Debug, Clone)]
pub(crate) struct RuntimeReloadResult {
    pub snapshot: RuntimeGovernanceSnapshot,
    pub reloaded_at: DateTime<Utc>,
}

pub(crate) struct RuntimeGovernance {
    service: Arc<RuntimeService>,
    coordinator: Arc<RuntimeCoordinator>,
    active_plugins: RwLock<Vec<ActivePluginRuntime>>,
    reload_lock: Mutex<()>,
}

impl RuntimeGovernance {
    pub(crate) fn new(
        service: Arc<RuntimeService>,
        coordinator: Arc<RuntimeCoordinator>,
        active_plugins: Vec<ActivePluginRuntime>,
    ) -> Self {
        Self {
            service,
            coordinator,
            active_plugins: RwLock::new(active_plugins),
            reload_lock: Mutex::new(()),
        }
    }

    pub(crate) async fn snapshot(&self) -> RuntimeGovernanceSnapshot {
        self.refresh_plugin_health().await;
        self.snapshot_with_paths(configured_plugin_paths()).await
    }

    pub(crate) async fn reload(&self) -> Result<RuntimeReloadResult, ServiceError> {
        let search_paths = configured_plugin_paths();
        let manifests =
            discover_plugin_manifests_in(&search_paths).map_err(ServiceError::Internal)?;
        let initializer = SupervisorPluginInitializer {
            loader: PluginLoader {
                search_paths: search_paths.clone(),
            },
        };
        self.reload_from_manifests(manifests, &initializer, search_paths)
            .await
    }

    async fn reload_from_manifests<I>(
        &self,
        manifests: Vec<PluginManifest>,
        initializer: &I,
        plugin_search_paths: Vec<PathBuf>,
    ) -> Result<RuntimeReloadResult, ServiceError>
    where
        I: PluginInitializer,
    {
        let _guard = self.reload_lock.lock().await;
        let running_session_ids = self.service.running_session_ids();
        if !running_session_ids.is_empty() {
            return Err(ServiceError::Conflict(format!(
                "cannot reload runtime capabilities while sessions are running: {}",
                running_session_ids.join(", ")
            )));
        }

        let assembled =
            assemble_runtime_surface(manifests, initializer, self.coordinator.plugin_registry())
                .await
                .map_err(ServiceError::Internal)?;
        let capability_surface = assembled.router.descriptors();
        self.service.replace_capabilities(assembled.router).await?;
        let previous_active_plugins = {
            let mut guard = self.active_plugins.write().await;
            std::mem::replace(&mut *guard, assembled.active_plugins)
        };
        let retired_components = self.coordinator.replace_runtime_surface(
            assembled.plugin_entries,
            capability_surface,
            assembled.managed_components,
        );
        for component in retired_components {
            if let Err(error) = component.shutdown_component().await {
                log::warn!(
                    "failed to shut down retired managed component '{}' after reload: {}",
                    component.component_name(),
                    error
                );
            }
        }

        // RuntimeCoordinator owns managed-component shutdown so reload cannot race or double-close
        // the same supervisor through a second governance-side lifecycle path.
        drop(previous_active_plugins);

        Ok(RuntimeReloadResult {
            snapshot: self.snapshot_with_paths(plugin_search_paths).await,
            reloaded_at: Utc::now(),
        })
    }

    async fn snapshot_with_paths(
        &self,
        plugin_search_paths: Vec<PathBuf>,
    ) -> RuntimeGovernanceSnapshot {
        let runtime = self.coordinator.runtime();
        RuntimeGovernanceSnapshot {
            runtime_name: runtime.runtime_name().to_string(),
            runtime_kind: runtime.runtime_kind().to_string(),
            loaded_session_count: self.service.loaded_session_count(),
            running_session_ids: self.service.running_session_ids(),
            plugin_search_paths,
            metrics: self.service.observability_snapshot(),
            capabilities: self.coordinator.capabilities(),
            plugins: self.coordinator.plugin_registry().snapshot(),
        }
    }

    async fn refresh_plugin_health(&self) {
        let active_plugins = self.active_plugins.read().await.clone();
        for active_plugin in active_plugins {
            let checked_at = Utc::now().to_rfc3339();
            match active_plugin.component.health_report().await {
                Ok(report) => match report.health {
                    PluginHealth::Healthy => {
                        self.coordinator.plugin_registry().record_health_probe(
                            &active_plugin.name,
                            PluginHealth::Healthy,
                            None,
                            checked_at,
                        );
                    }
                    PluginHealth::Unavailable | PluginHealth::Degraded | PluginHealth::Unknown => {
                        let message = report
                            .message
                            .unwrap_or_else(|| "plugin supervisor unavailable".to_string());
                        self.coordinator.plugin_registry().record_health_probe(
                            &active_plugin.name,
                            PluginHealth::Unavailable,
                            Some(message),
                            checked_at,
                        );
                    }
                },
                Err(error) => {
                    self.coordinator.plugin_registry().record_health_probe(
                        &active_plugin.name,
                        PluginHealth::Unavailable,
                        Some(error.to_string()),
                        checked_at,
                    );
                }
            }
        }
    }
}

#[derive(Clone)]
pub(crate) struct ActivePluginRuntime {
    name: String,
    component: Arc<dyn ManagedPluginComponent>,
}

struct AssembledRuntimeSurface {
    router: CapabilityRouter,
    plugin_entries: Vec<PluginEntry>,
    managed_components: Vec<Arc<dyn ManagedRuntimeComponent>>,
    active_plugins: Vec<ActivePluginRuntime>,
}

struct LoadedPlugin {
    component: Arc<dyn ManagedPluginComponent>,
    capabilities: Vec<CapabilityDescriptor>,
    invokers: Vec<Arc<dyn CapabilityInvoker>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ManagedPluginHealth {
    health: PluginHealth,
    message: Option<String>,
}

#[async_trait]
trait ManagedPluginComponent: ManagedRuntimeComponent {
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
trait PluginInitializer: Send + Sync {
    async fn initialize(
        &self,
        manifest: &PluginManifest,
    ) -> std::result::Result<LoadedPlugin, AstrError>;
}

struct SupervisorPluginInitializer {
    loader: PluginLoader,
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
    let initializer = SupervisorPluginInitializer {
        loader: PluginLoader { search_paths },
    };
    bootstrap_runtime_from_manifests(manifests, &initializer).await
}

async fn bootstrap_runtime_from_manifests<I>(
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

async fn assemble_runtime_surface<I>(
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

fn configured_plugin_paths() -> Vec<PathBuf> {
    match std::env::var_os("ASTRCODE_PLUGIN_DIRS") {
        Some(raw_paths) => std::env::split_paths(&raw_paths).collect(),
        None => Vec::new(),
    }
}

fn discover_plugin_manifests_in(
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

fn conflicting_capability_name(
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use serde_json::{json, Value};

    use super::{
        bootstrap_runtime_from_manifests, conflicting_capability_name, LoadedPlugin,
        ManagedPluginComponent, ManagedPluginHealth, PluginInitializer, RuntimeBootstrap,
    };
    use crate::test_support::ServerTestEnvGuard;
    use astrcode_core::{
        CapabilityContext, CapabilityDescriptor, CapabilityExecutionResult, CapabilityInvoker,
        CapabilityKind, ManagedRuntimeComponent, PluginHealth, PluginManifest, PluginState,
        PluginType, Result, SideEffectLevel, StabilityLevel,
    };

    struct FakeInitializer {
        responses: HashMap<String, FakePluginResponse>,
    }

    enum FakePluginResponse {
        Loaded(LoadedPlugin),
        Failed(String),
    }

    #[async_trait]
    impl PluginInitializer for FakeInitializer {
        async fn initialize(
            &self,
            manifest: &PluginManifest,
        ) -> std::result::Result<LoadedPlugin, astrcode_core::AstrError> {
            match self
                .responses
                .get(&manifest.name)
                .expect("initializer response should exist")
            {
                FakePluginResponse::Loaded(loaded) => Ok(LoadedPlugin {
                    component: loaded.component.clone(),
                    capabilities: loaded.capabilities.clone(),
                    invokers: loaded.invokers.clone(),
                }),
                FakePluginResponse::Failed(message) => {
                    Err(astrcode_core::AstrError::Internal(message.clone()))
                }
            }
        }
    }

    struct FakeManagedComponent {
        name: String,
        shutdowns: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl ManagedRuntimeComponent for FakeManagedComponent {
        fn component_name(&self) -> String {
            self.name.clone()
        }

        async fn shutdown_component(&self) -> Result<()> {
            self.shutdowns
                .lock()
                .expect("shutdown log lock")
                .push(self.name.clone());
            Ok(())
        }
    }

    #[async_trait]
    impl ManagedPluginComponent for FakeManagedComponent {
        async fn health_report(
            &self,
        ) -> std::result::Result<ManagedPluginHealth, astrcode_core::AstrError> {
            Ok(ManagedPluginHealth {
                health: PluginHealth::Healthy,
                message: None,
            })
        }
    }

    struct FakeCapabilityInvoker {
        descriptor: CapabilityDescriptor,
    }

    #[async_trait]
    impl CapabilityInvoker for FakeCapabilityInvoker {
        fn descriptor(&self) -> CapabilityDescriptor {
            self.descriptor.clone()
        }

        async fn invoke(
            &self,
            _payload: Value,
            _ctx: &CapabilityContext,
        ) -> Result<CapabilityExecutionResult> {
            Ok(CapabilityExecutionResult::ok(
                self.descriptor.name.clone(),
                Value::Null,
            ))
        }
    }

    fn manifest(name: &str) -> PluginManifest {
        PluginManifest {
            name: name.to_string(),
            version: "0.1.0".to_string(),
            description: format!("{name} plugin"),
            plugin_type: vec![PluginType::Tool],
            capabilities: Vec::new(),
            executable: Some("plugin.exe".to_string()),
            args: Vec::new(),
            working_dir: None,
            repository: None,
        }
    }

    fn capability(name: &str) -> CapabilityDescriptor {
        CapabilityDescriptor {
            name: name.to_string(),
            kind: CapabilityKind::Tool,
            description: format!("{name} capability"),
            input_schema: json!({ "type": "object" }),
            output_schema: json!({ "type": "object" }),
            streaming: false,
            profiles: vec!["coding".to_string()],
            tags: Vec::new(),
            permissions: Vec::new(),
            side_effect: SideEffectLevel::None,
            stability: StabilityLevel::Stable,
        }
    }

    fn loaded_plugin(
        plugin_name: &str,
        capability_names: &[&str],
        shutdowns: Arc<Mutex<Vec<String>>>,
    ) -> LoadedPlugin {
        let capabilities = capability_names
            .iter()
            .map(|name| capability(name))
            .collect::<Vec<_>>();
        let invokers = capabilities
            .iter()
            .cloned()
            .map(|descriptor| {
                Arc::new(FakeCapabilityInvoker { descriptor }) as Arc<dyn CapabilityInvoker>
            })
            .collect();

        LoadedPlugin {
            component: Arc::new(FakeManagedComponent {
                name: plugin_name.to_string(),
                shutdowns,
            }),
            capabilities,
            invokers,
        }
    }

    fn bootstrap_from(
        manifests: Vec<PluginManifest>,
        initializer: FakeInitializer,
    ) -> (RuntimeBootstrap, ServerTestEnvGuard) {
        let guard = ServerTestEnvGuard::new();
        let runtime = tokio::runtime::Runtime::new().expect("tokio runtime should build");
        let bootstrap = runtime
            .block_on(async { bootstrap_runtime_from_manifests(manifests, &initializer).await })
            .expect("runtime bootstrap should succeed");
        (bootstrap, guard)
    }

    #[test]
    fn bootstrap_without_plugins_keeps_builtin_capabilities() {
        let initializer = FakeInitializer {
            responses: Default::default(),
        };
        let (bootstrap, _guard) = bootstrap_from(Vec::new(), initializer);

        assert!(bootstrap
            .coordinator
            .capabilities()
            .iter()
            .any(|descriptor| descriptor.name == "shell"));
        assert!(bootstrap
            .coordinator
            .plugin_registry()
            .snapshot()
            .is_empty());
    }

    #[test]
    fn bootstrap_records_initialized_and_failed_plugins_without_aborting_server() {
        let shutdowns = Arc::new(Mutex::new(Vec::new()));
        let initializer = FakeInitializer {
            responses: HashMap::from([
                (
                    "alpha".to_string(),
                    FakePluginResponse::Loaded(loaded_plugin(
                        "alpha",
                        &["tool.alpha"],
                        Arc::clone(&shutdowns),
                    )),
                ),
                (
                    "beta".to_string(),
                    FakePluginResponse::Failed("handshake failed".to_string()),
                ),
            ]),
        };

        let (bootstrap, _guard) =
            bootstrap_from(vec![manifest("alpha"), manifest("beta")], initializer);
        let registry = bootstrap.coordinator.plugin_registry();
        let alpha = registry.get("alpha").expect("alpha entry should exist");
        let beta = registry.get("beta").expect("beta entry should exist");

        assert_eq!(alpha.state, PluginState::Initialized);
        assert_eq!(alpha.capabilities.len(), 1);
        assert_eq!(beta.state, PluginState::Failed);
        assert_eq!(
            beta.failure.as_deref(),
            Some("internal error: handshake failed")
        );
        assert!(bootstrap
            .coordinator
            .capabilities()
            .iter()
            .any(|descriptor| descriptor.name == "tool.alpha"));
    }

    #[test]
    fn bootstrap_rejects_duplicate_plugin_capabilities_deterministically() {
        let shutdowns = Arc::new(Mutex::new(Vec::new()));
        let initializer = FakeInitializer {
            responses: HashMap::from([
                (
                    "alpha".to_string(),
                    FakePluginResponse::Loaded(loaded_plugin(
                        "alpha",
                        &["tool.shared"],
                        Arc::clone(&shutdowns),
                    )),
                ),
                (
                    "beta".to_string(),
                    FakePluginResponse::Loaded(loaded_plugin(
                        "beta",
                        &["tool.shared"],
                        Arc::clone(&shutdowns),
                    )),
                ),
            ]),
        };

        let (bootstrap, _guard) =
            bootstrap_from(vec![manifest("beta"), manifest("alpha")], initializer);
        let snapshot = bootstrap.coordinator.plugin_registry().snapshot();
        let alpha = snapshot
            .iter()
            .find(|entry| entry.manifest.name == "alpha")
            .expect("alpha entry should exist");
        let beta = snapshot
            .iter()
            .find(|entry| entry.manifest.name == "beta")
            .expect("beta entry should exist");

        assert_eq!(alpha.state, PluginState::Initialized);
        assert_eq!(beta.state, PluginState::Failed);
        assert_eq!(
            shutdowns.lock().expect("shutdown log").clone(),
            vec!["beta".to_string()]
        );
    }

    #[tokio::test]
    async fn governance_reload_swaps_runtime_surface_and_shutdowns_retired_plugins() {
        let _guard = ServerTestEnvGuard::new();
        let shutdowns = Arc::new(Mutex::new(Vec::new()));
        let initial = FakeInitializer {
            responses: HashMap::from([(
                "alpha".to_string(),
                FakePluginResponse::Loaded(loaded_plugin(
                    "alpha",
                    &["tool.alpha"],
                    Arc::clone(&shutdowns),
                )),
            )]),
        };
        let bootstrap = bootstrap_runtime_from_manifests(vec![manifest("alpha")], &initial)
            .await
            .expect("initial bootstrap should succeed");

        let replacement = FakeInitializer {
            responses: HashMap::from([(
                "beta".to_string(),
                FakePluginResponse::Loaded(loaded_plugin(
                    "beta",
                    &["tool.beta"],
                    Arc::clone(&shutdowns),
                )),
            )]),
        };
        let reload = bootstrap
            .governance
            .reload_from_manifests(
                vec![manifest("beta")],
                &replacement,
                vec![PathBuf::from("plugins")],
            )
            .await
            .expect("reload should succeed");

        assert_eq!(
            bootstrap
                .coordinator
                .plugin_registry()
                .snapshot()
                .into_iter()
                .map(|entry| entry.manifest.name)
                .collect::<Vec<_>>(),
            vec!["beta".to_string()]
        );
        assert_eq!(
            bootstrap
                .coordinator
                .capabilities()
                .into_iter()
                .map(|descriptor| descriptor.name)
                .filter(|name| name.starts_with("tool."))
                .collect::<Vec<_>>(),
            vec!["tool.beta".to_string()]
        );
        assert_eq!(
            shutdowns.lock().expect("shutdown log").clone(),
            vec!["alpha".to_string()]
        );
        assert_eq!(
            reload.snapshot.plugin_search_paths,
            vec![PathBuf::from("plugins")]
        );
    }

    #[test]
    fn conflicting_capability_name_detects_existing_and_local_duplicates() {
        let registered = std::collections::HashSet::from(["tool.shared".to_string()]);
        assert_eq!(
            conflicting_capability_name(&registered, &[capability("tool.shared")]),
            Some("tool.shared".to_string())
        );
        assert_eq!(
            conflicting_capability_name(
                &std::collections::HashSet::new(),
                &[capability("tool.local"), capability("tool.local")]
            ),
            Some("tool.local".to_string())
        );
    }
}
