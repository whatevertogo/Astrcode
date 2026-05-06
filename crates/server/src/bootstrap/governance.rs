//! # 治理装配
//!
//! 负责把底层 `RuntimeCoordinator` 适配成应用层治理端口，
//! 并为治理入口接入真实 reload/observability 组合根。

use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{Arc, RwLock},
};

use astrcode_adapter_mcp::{
    config::McpServerConfig,
    manager::{McpConnectionManager, McpReloadSnapshot},
};
use astrcode_adapter_skills::{LayeredSkillCatalog, load_builtin_skills};
use astrcode_core::{CapabilityInvoker, CapabilitySpec, SkillSpec};
use astrcode_core::mode::GovernanceModeSpec;
use astrcode_plugin_host::{
    BuiltinHookRegistry, PluginActiveSnapshot, PluginDescriptor, PluginEntry,
    ProviderContributionCatalog, ResourceCatalog, build_skill_catalog_base,
    builtin_modes_descriptor, builtin_openai_provider_descriptor, builtin_tools_descriptor,
    resources_discover,
};
use astrcode_runtime_contract::{ManagedRuntimeComponent, RuntimeHandle};
use async_trait::async_trait;

use super::{
    builtin_plugins::{
        BUILTIN_GOVERNANCE_MODES_PLUGIN_ID, EXTERNAL_PLUGIN_MODES_PLUGIN_ID,
        builtin_composer_plugin_descriptor, builtin_permission_descriptor,
        builtin_planning_descriptor,
    },
    capabilities::CapabilitySurfaceSync,
    deps::core::AstrError,
    mcp::load_declared_configs,
    plugins::bootstrap_plugins_with_skill_root,
    runtime_coordinator::RuntimeCoordinator,
};
use crate::{
    AppGovernance, GovernanceSnapshot, RuntimeGovernancePort, RuntimeGovernanceSnapshot,
    RuntimeReloader, ServerApplicationError, SessionInfoProvider,
    application_error_bridge::ServerRouteError,
    config_service_bridge::ServerConfigService,
    governance_service::{
        ServerGovernancePort, ServerGovernanceReloadResult, ServerGovernanceService,
        ServerGovernanceSnapshot,
    },
    hook_adapter::PluginHostHookDispatcher,
    mode_catalog_service::{ServerModeCatalog, ServerModeCatalogSnapshot},
    runtime_owner_bridge::{ServerRuntimeObservability, ServerTaskRegistry},
};

pub(crate) struct GovernanceBuildInput {
    pub sessions: Arc<dyn SessionInfoProvider>,
    pub config_service: Arc<ServerConfigService>,
    pub coordinator: Arc<RuntimeCoordinator>,
    pub task_registry: Arc<ServerTaskRegistry>,
    pub observability: Arc<ServerRuntimeObservability>,
    pub mcp_manager: Arc<McpConnectionManager>,
    pub capability_sync: CapabilitySurfaceSync,
    pub skill_catalog: Arc<LayeredSkillCatalog>,
    pub resource_catalog: Arc<RwLock<ResourceCatalog>>,
    pub provider_catalog: Arc<RwLock<ProviderContributionCatalog>>,
    pub plugin_search_paths: Vec<PathBuf>,
    pub plugin_skill_root: PathBuf,
    pub managed_plugin_components: Vec<Arc<dyn ManagedRuntimeComponent>>,
    pub working_dir: PathBuf,
    pub mode_catalog: Option<Arc<ServerModeCatalog>>,
    pub core_tool_specs: Vec<CapabilitySpec>,
    pub planning_tool_specs: Vec<CapabilitySpec>,
    pub collaboration_tool_specs: Vec<CapabilitySpec>,
    pub builtin_mode_specs: Vec<GovernanceModeSpec>,
    pub builtin_hook_registry: Arc<BuiltinHookRegistry>,
    pub hook_dispatcher: Arc<PluginHostHookDispatcher>,
}

pub(crate) fn build_server_governance_service(
    input: GovernanceBuildInput,
) -> Arc<ServerGovernanceService> {
    let runtime_port = Arc::new(CoordinatorGovernancePort {
        coordinator: Arc::clone(&input.coordinator),
    });
    let reloader: Arc<dyn RuntimeReloader> = Arc::new(ServerRuntimeReloader {
        config_service: Arc::clone(&input.config_service),
        coordinator: Arc::clone(&input.coordinator),
        mcp_manager: Arc::clone(&input.mcp_manager),
        capability_sync: input.capability_sync.clone(),
        skill_catalog: Arc::clone(&input.skill_catalog),
        resource_catalog: Arc::clone(&input.resource_catalog),
        provider_catalog: Arc::clone(&input.provider_catalog),
        plugin_search_paths: input.plugin_search_paths.clone(),
        plugin_skill_root: input.plugin_skill_root.clone(),
        working_dir: input.working_dir.clone(),
        mode_catalog: input.mode_catalog,
        core_tool_specs: input.core_tool_specs,
        planning_tool_specs: input.planning_tool_specs,
        collaboration_tool_specs: input.collaboration_tool_specs,
        builtin_mode_specs: input.builtin_mode_specs,
        builtin_hook_registry: input.builtin_hook_registry,
        hook_dispatcher: input.hook_dispatcher,
    });
    let managed_components: Vec<Arc<dyn ManagedRuntimeComponent>> = input.managed_plugin_components;
    input.coordinator.replace_runtime_surface(
        input.coordinator.plugin_registry().snapshot(),
        input.capability_sync.current_capabilities(),
        managed_components,
    );

    let governance = Arc::new(
        AppGovernance::new(
            runtime_port,
            input.task_registry.inner(),
            input.observability,
            input.sessions,
        )
        .with_reloader(reloader),
    );
    Arc::new(ServerGovernanceService::new(Arc::new(
        ApplicationGovernancePort { inner: governance },
    )))
}

struct ApplicationGovernancePort {
    inner: Arc<AppGovernance>,
}

const SERVER_RUNTIME_NAME: &str = "astrcode-server";
const SERVER_RUNTIME_KIND: &str = "server";

#[async_trait]
impl ServerGovernancePort for ApplicationGovernancePort {
    fn capabilities(&self) -> Vec<astrcode_core::CapabilitySpec> {
        self.inner.runtime().snapshot().capabilities
    }

    async fn reload(&self) -> Result<ServerGovernanceReloadResult, ServerRouteError> {
        let reloaded = self
            .inner
            .reload()
            .await
            .map_err(application_error_to_server)?;
        Ok(ServerGovernanceReloadResult {
            snapshot: server_snapshot_from_application(reloaded.snapshot),
            reloaded_at: reloaded.reloaded_at,
        })
    }

    async fn shutdown(&self, timeout_secs: u64) -> Result<(), ServerRouteError> {
        self.inner
            .shutdown(timeout_secs)
            .await
            .map_err(application_error_to_server)
    }
}

fn server_snapshot_from_application(snapshot: GovernanceSnapshot) -> ServerGovernanceSnapshot {
    ServerGovernanceSnapshot {
        runtime_name: snapshot.runtime_name,
        runtime_kind: snapshot.runtime_kind,
        loaded_session_count: snapshot.loaded_session_count,
        running_session_ids: snapshot.running_session_ids,
        plugin_search_paths: snapshot.plugin_search_paths,
        metrics: snapshot.metrics,
        capabilities: snapshot.capabilities,
        plugins: snapshot.plugins,
    }
}

#[derive(Debug)]
struct CoordinatorGovernancePort {
    coordinator: Arc<RuntimeCoordinator>,
}

impl RuntimeGovernancePort for CoordinatorGovernancePort {
    fn snapshot(&self) -> RuntimeGovernanceSnapshot {
        let runtime = self.coordinator.runtime();
        RuntimeGovernanceSnapshot {
            runtime_name: runtime.runtime_name().to_string(),
            runtime_kind: runtime.runtime_kind().to_string(),
            capabilities: self.coordinator.capabilities(),
            plugins: self.coordinator.plugin_registry().snapshot(),
        }
    }

    fn shutdown(
        &self,
        timeout_secs: u64,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = std::result::Result<(), ServerApplicationError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            self.coordinator
                .shutdown(timeout_secs)
                .await
                .map_err(|error| ServerApplicationError::Internal(error.to_string()))
        })
    }
}

#[derive(Debug)]
pub(crate) struct AppRuntimeHandle;

#[async_trait]
impl RuntimeHandle for AppRuntimeHandle {
    fn runtime_name(&self) -> &'static str {
        SERVER_RUNTIME_NAME
    }

    fn runtime_kind(&self) -> &'static str {
        SERVER_RUNTIME_KIND
    }

    async fn shutdown(&self, _timeout_secs: u64) -> std::result::Result<(), AstrError> {
        Ok(())
    }
}

#[derive(Clone)]
struct ServerRuntimeReloader {
    config_service: Arc<ServerConfigService>,
    coordinator: Arc<RuntimeCoordinator>,
    mcp_manager: Arc<McpConnectionManager>,
    capability_sync: CapabilitySurfaceSync,
    skill_catalog: Arc<LayeredSkillCatalog>,
    resource_catalog: Arc<RwLock<ResourceCatalog>>,
    provider_catalog: Arc<RwLock<ProviderContributionCatalog>>,
    plugin_search_paths: Vec<PathBuf>,
    plugin_skill_root: PathBuf,
    working_dir: PathBuf,
    mode_catalog: Option<Arc<ServerModeCatalog>>,
    core_tool_specs: Vec<CapabilitySpec>,
    planning_tool_specs: Vec<CapabilitySpec>,
    collaboration_tool_specs: Vec<CapabilitySpec>,
    builtin_mode_specs: Vec<GovernanceModeSpec>,
    builtin_hook_registry: Arc<BuiltinHookRegistry>,
    hook_dispatcher: Arc<PluginHostHookDispatcher>,
}

impl std::fmt::Debug for ServerRuntimeReloader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerRuntimeReloader")
            .field("plugin_search_paths", &self.plugin_search_paths)
            .field("plugin_skill_root", &self.plugin_skill_root)
            .finish_non_exhaustive()
    }
}

struct PreparedGovernanceReload {
    search_paths: Vec<PathBuf>,
    mcp_configs: Vec<McpServerConfig>,
    mode_snapshot: Option<ServerModeCatalogSnapshot>,
    base_skills: Vec<SkillSpec>,
    plugin_modes: Vec<GovernanceModeSpec>,
    external_descriptors: Vec<PluginDescriptor>,
    plugin_invokers: Vec<Arc<dyn CapabilityInvoker>>,
    plugin_entries: Vec<PluginEntry>,
    managed_components: Vec<Arc<dyn ManagedRuntimeComponent>>,
}

struct GovernanceReloadRollback {
    mcp_snapshot: McpReloadSnapshot,
    plugin_invokers: Vec<Arc<dyn CapabilityInvoker>>,
}

impl GovernanceReloadRollback {
    async fn capture(
        mcp_manager: &McpConnectionManager,
        capability_sync: &CapabilitySurfaceSync,
    ) -> Self {
        let mcp_snapshot = mcp_manager.capture_reload_snapshot().await;
        let mcp_surface = mcp_manager.current_surface().await;
        let mcp_capability_names = mcp_surface
            .capability_invokers
            .into_iter()
            .map(|invoker| invoker.capability_spec().name.to_string())
            .collect::<HashSet<_>>();
        let plugin_invokers = capability_sync
            .current_external_invokers()
            .into_iter()
            .filter(|invoker| {
                !mcp_capability_names.contains(invoker.capability_spec().name.as_str())
            })
            .collect();
        Self {
            mcp_snapshot,
            plugin_invokers,
        }
    }

    async fn restore(
        self,
        mcp_manager: &McpConnectionManager,
        capability_sync: &CapabilitySurfaceSync,
    ) -> Result<(), ServerApplicationError> {
        let mut external_invokers = mcp_manager
            .restore_reload_snapshot(&self.mcp_snapshot)
            .await
            .map_err(|error| ServerApplicationError::Internal(error.to_string()))?;
        external_invokers.extend(self.plugin_invokers);
        capability_sync
            .apply_external_invokers(external_invokers)
            .map_err(|error| ServerApplicationError::Internal(error.to_string()))
    }
}

impl ServerRuntimeReloader {
    async fn prepare_reload_candidate(
        &self,
    ) -> Result<PreparedGovernanceReload, ServerApplicationError> {
        let mcp_configs = load_declared_configs(&self.config_service, self.working_dir.as_path())
            .await
            .map_err(server_error_to_application)?;
        let plugin_bootstrap = bootstrap_plugins_with_skill_root(
            self.plugin_search_paths.clone(),
            self.plugin_skill_root.clone(),
        )
        .await;
        let mode_snapshot = match &self.mode_catalog {
            Some(mode_catalog) => Some(
                mode_catalog
                    .preview_plugin_modes(plugin_bootstrap.modes.clone())
                    .map_err(ServerApplicationError::from)?,
            ),
            None => None,
        };

        let base_skills = build_skill_catalog_base(
            load_builtin_skills(),
            plugin_bootstrap.skills.clone(),
            &plugin_bootstrap.resource_catalog,
        )
        .base_skills;
        Ok(PreparedGovernanceReload {
            search_paths: plugin_bootstrap.search_paths,
            mcp_configs,
            mode_snapshot,
            base_skills,
            plugin_modes: plugin_bootstrap.modes,
            external_descriptors: plugin_bootstrap.descriptors,
            plugin_invokers: plugin_bootstrap.invokers,
            plugin_entries: plugin_bootstrap.registry.snapshot(),
            managed_components: plugin_bootstrap.managed_components,
        })
    }

    fn build_plugin_host_descriptors(
        &self,
        mcp_invokers: &[Arc<dyn CapabilityInvoker>],
        plugin_modes: Vec<GovernanceModeSpec>,
        mut external_descriptors: Vec<PluginDescriptor>,
    ) -> Vec<PluginDescriptor> {
        let mut descriptors = vec![
            builtin_openai_provider_descriptor(),
            builtin_modes_descriptor(
                BUILTIN_GOVERNANCE_MODES_PLUGIN_ID,
                "Builtin Governance Modes",
                self.builtin_mode_specs
                    .iter()
                    .filter(|mode| mode.id.as_str() != "plan")
                    .cloned()
                    .collect(),
            ),
            builtin_permission_descriptor(),
            builtin_planning_descriptor(
                self.planning_tool_specs.clone(),
                self.builtin_mode_specs
                    .iter()
                    .filter(|mode| mode.id.as_str() == "plan")
                    .cloned()
                    .collect(),
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
                self.core_tool_specs.clone(),
            ),
            builtin_tools_descriptor(
                "external-mcp-tools",
                "External MCP Tools",
                mcp_invokers
                    .iter()
                    .map(|invoker| invoker.capability_spec())
                    .collect(),
            ),
            builtin_tools_descriptor(
                "builtin-collaboration-tools",
                "Builtin Collaboration Tools",
                self.collaboration_tool_specs.clone(),
            ),
        ];
        descriptors.append(&mut external_descriptors);
        descriptors
    }

    fn stage_plugin_host_snapshot(
        &self,
        descriptors: Vec<PluginDescriptor>,
    ) -> Result<PluginActiveSnapshot, ServerApplicationError> {
        self.coordinator
            .plugin_registry()
            .stage_candidate_with_hook_registry(descriptors, self.builtin_hook_registry.as_ref())
            .map_err(ServerApplicationError::from)
    }

    fn rollback_plugin_host_candidate(&self) {
        self.coordinator.plugin_registry().rollback_candidate();
    }

    fn commit_plugin_host_candidate(
        &self,
        expected_snapshot_id: &str,
    ) -> Result<PluginActiveSnapshot, ServerApplicationError> {
        let snapshot = self
            .coordinator
            .plugin_registry()
            .commit_candidate()
            .ok_or_else(|| {
                ServerApplicationError::Internal(
                    "plugin-host active snapshot commit unexpectedly failed".to_string(),
                )
            })?;
        if snapshot.snapshot_id != expected_snapshot_id {
            return Err(ServerApplicationError::Internal(format!(
                "plugin-host committed snapshot '{}' but candidate was '{}'",
                snapshot.snapshot_id, expected_snapshot_id
            )));
        }
        self.hook_dispatcher
            .replace_active_snapshot(snapshot.snapshot_id.clone(), snapshot.hook_bindings.clone());
        Ok(snapshot)
    }

    async fn shutdown_replaced_components(
        &self,
        previous_components: Vec<Arc<dyn ManagedRuntimeComponent>>,
    ) {
        for component in previous_components {
            if let Err(error) = component.shutdown_component().await {
                log::warn!(
                    "failed to shut down replaced managed component '{}': {}",
                    component.component_name(),
                    error
                );
            }
        }
    }

    async fn restore_external_surface_after_failure(
        &self,
        rollback: GovernanceReloadRollback,
        error: ServerApplicationError,
    ) -> ServerApplicationError {
        log::error!("governance reload failed while preparing plugin-host snapshot: {error}");
        if let Err(rollback_error) = rollback
            .restore(&self.mcp_manager, &self.capability_sync)
            .await
        {
            return ServerApplicationError::Internal(format!(
                "governance reload failed: {}; rollback failed: {}",
                error, rollback_error
            ));
        }
        error
    }
}

impl RuntimeReloader for ServerRuntimeReloader {
    fn reload(
        &self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<Vec<PathBuf>, ServerApplicationError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            self.config_service
                .reload_from_disk()
                .await
                .map_err(server_error_to_application)?;
            let candidate = self.prepare_reload_candidate().await?;
            let rollback =
                GovernanceReloadRollback::capture(&self.mcp_manager, &self.capability_sync).await;

            let mcp_invokers = self
                .mcp_manager
                .reload_config(candidate.mcp_configs)
                .await
                .map_err(|error| ServerApplicationError::Internal(error.to_string()))?;
            let plugin_host_descriptors = self.build_plugin_host_descriptors(
                &mcp_invokers,
                candidate.plugin_modes.clone(),
                candidate.external_descriptors.clone(),
            );
            let resource_catalog =
                match resources_discover(&plugin_host_descriptors).map(|report| report.catalog) {
                    Ok(catalog) => catalog,
                    Err(error) => {
                        return Err(self
                            .restore_external_surface_after_failure(
                                rollback,
                                ServerApplicationError::from(error),
                            )
                            .await);
                    },
                };
            let provider_catalog =
                match ProviderContributionCatalog::from_descriptors(&plugin_host_descriptors) {
                    Ok(catalog) => catalog,
                    Err(error) => {
                        return Err(self
                            .restore_external_surface_after_failure(
                                rollback,
                                ServerApplicationError::from(error),
                            )
                            .await);
                    },
                };
            let staged_snapshot = match self.stage_plugin_host_snapshot(plugin_host_descriptors) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    return Err(self
                        .restore_external_surface_after_failure(rollback, error)
                        .await);
                },
            };
            let mut external_invokers = mcp_invokers;
            external_invokers.extend(candidate.plugin_invokers.clone());

            if let Err(error) = self
                .capability_sync
                .apply_external_invokers(external_invokers)
            {
                self.rollback_plugin_host_candidate();
                let error = ServerApplicationError::Internal(error.to_string());
                log::error!(
                    "governance reload failed while applying candidate capability surface: {error}"
                );
                if let Err(rollback_error) = rollback
                    .restore(&self.mcp_manager, &self.capability_sync)
                    .await
                {
                    return Err(ServerApplicationError::Internal(format!(
                        "governance reload failed: {}; rollback failed: {}",
                        error, rollback_error
                    )));
                }
                log::warn!(
                    "governance reload rolled back to previous external capability snapshot"
                );
                return Err(error);
            }
            let committed_snapshot =
                match self.commit_plugin_host_candidate(&staged_snapshot.snapshot_id) {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        if let Err(rollback_error) = rollback
                            .restore(&self.mcp_manager, &self.capability_sync)
                            .await
                        {
                            return Err(ServerApplicationError::Internal(format!(
                                "governance reload failed: {}; rollback failed: {}",
                                error, rollback_error
                            )));
                        }
                        return Err(error);
                    },
                };

            self.skill_catalog
                .replace_base_skills(candidate.base_skills);
            *self
                .resource_catalog
                .write()
                .expect("plugin resource catalog lock poisoned") = resource_catalog;
            *self
                .provider_catalog
                .write()
                .expect("provider catalog lock poisoned") = provider_catalog;
            if let (Some(mode_catalog), Some(mode_snapshot)) =
                (&self.mode_catalog, candidate.mode_snapshot)
            {
                mode_catalog.replace_snapshot(mode_snapshot);
            }
            let previous_components = self.coordinator.replace_runtime_surface(
                candidate.plugin_entries,
                self.capability_sync.current_capabilities(),
                candidate.managed_components,
            );
            self.shutdown_replaced_components(previous_components).await;
            log::info!(
                "governance reload committed: plugin_search_paths={}, base_skills={}, \
                 capability_count={}, hook_snapshot={}",
                candidate.search_paths.len(),
                self.skill_catalog.base_skills().len(),
                self.capability_sync.current_capabilities().len(),
                committed_snapshot.snapshot_id
            );

            Ok(candidate.search_paths)
        })
    }
}

fn server_error_to_application(error: ServerRouteError) -> ServerApplicationError {
    match error {
        ServerRouteError::NotFound(message) => ServerApplicationError::NotFound(message),
        ServerRouteError::Conflict(message) => ServerApplicationError::Conflict(message),
        ServerRouteError::InvalidArgument(message) => {
            ServerApplicationError::InvalidArgument(message)
        },
        ServerRouteError::PermissionDenied(message) => {
            ServerApplicationError::PermissionDenied(message)
        },
        ServerRouteError::Internal(message) => ServerApplicationError::Internal(message),
    }
}

fn application_error_to_server(error: ServerApplicationError) -> ServerRouteError {
    match error {
        ServerApplicationError::NotFound(message) => ServerRouteError::NotFound(message),
        ServerApplicationError::Conflict(message) => ServerRouteError::Conflict(message),
        ServerApplicationError::InvalidArgument(message) => {
            ServerRouteError::InvalidArgument(message)
        },
        ServerApplicationError::PermissionDenied(message) => {
            ServerRouteError::PermissionDenied(message)
        },
        ServerApplicationError::Internal(message) => ServerRouteError::Internal(message),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, HashSet},
        sync::{Arc, RwLock},
    };

    use astrcode_plugin_host::PluginRegistry;
    use astrcode_tool_contract::{Tool, ToolContext, ToolDefinition, ToolExecutionResult};
    use async_trait::async_trait;
    use serde_json::{Value, json};

    use super::*;
    use crate::{
        bootstrap::deps::core::{
            AstrError, CapabilityInvoker, CapabilityKind, CapabilitySpec, CapabilitySpecBuildError,
            Result,
        },
        tool_capability_invoker::ToolCapabilityInvoker,
    };

    #[derive(Debug)]
    struct StaticTool {
        name: &'static str,
        tags: &'static [&'static str],
    }

    #[async_trait]
    impl Tool for StaticTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: self.name.to_string(),
                description: format!("tool {}", self.name),
                parameters: json!({"type": "object"}),
            }
        }

        fn capability_spec(&self) -> std::result::Result<CapabilitySpec, CapabilitySpecBuildError> {
            CapabilitySpec::builder(self.name, CapabilityKind::Tool)
                .description(format!("tool {}", self.name))
                .schema(json!({"type": "object"}), json!({"type": "string"}))
                .tags(self.tags.iter().copied())
                .build()
        }

        async fn execute(
            &self,
            tool_call_id: String,
            _input: Value,
            _ctx: &ToolContext,
        ) -> Result<ToolExecutionResult> {
            Ok(ToolExecutionResult {
                tool_call_id,
                tool_name: self.name.to_string(),
                ok: true,
                output: String::new(),
                continuation: None,
                error: None,
                metadata: None,
                duration_ms: 0,
                truncated: false,
            })
        }
    }

    fn invoker(name: &'static str, tags: &'static [&'static str]) -> Arc<dyn CapabilityInvoker> {
        Arc::new(
            ToolCapabilityInvoker::new(Arc::new(StaticTool { name, tags }))
                .expect("static tool should build"),
        ) as Arc<dyn CapabilityInvoker>
    }

    #[derive(Default)]
    struct TestCapabilitySurface {
        invokers: RwLock<Vec<Arc<dyn CapabilityInvoker>>>,
    }

    impl crate::session_runtime_owner_bridge::ServerCapabilitySurfacePort for TestCapabilitySurface {
        fn replace_capability_invokers(
            &self,
            invokers: Vec<Arc<dyn CapabilityInvoker>>,
        ) -> Result<()> {
            let mut seen = HashSet::new();
            for invoker in &invokers {
                let name = invoker.capability_spec().name.to_string();
                if !seen.insert(name.clone()) {
                    return Err(AstrError::Validation(format!(
                        "duplicate capability '{}'",
                        name
                    )));
                }
            }
            *self
                .invokers
                .write()
                .expect("test capability surface lock should not be poisoned") = invokers;
            Ok(())
        }
    }

    #[tokio::test]
    async fn governance_port_exposes_runtime_snapshot_and_shutdown() {
        let coordinator = Arc::new(RuntimeCoordinator::new(
            Arc::new(AppRuntimeHandle),
            Arc::new(PluginRegistry::default()),
            vec![
                CapabilitySpec::builder("test_tool", CapabilityKind::Tool)
                    .description("test")
                    .schema(
                        serde_json::json!({"type":"object"}),
                        serde_json::json!({"type":"string"}),
                    )
                    .build()
                    .expect("static capability should build"),
            ],
        ));
        let port = CoordinatorGovernancePort { coordinator };

        let snapshot = port.snapshot();
        assert_eq!(snapshot.runtime_name, SERVER_RUNTIME_NAME);
        assert_eq!(snapshot.runtime_kind, SERVER_RUNTIME_KIND);
        assert_eq!(snapshot.capabilities.len(), 1);
        assert!(snapshot.plugins.is_empty());

        port.shutdown(1).await.expect("shutdown should succeed");
    }

    #[tokio::test]
    async fn rollback_restores_previous_mcp_and_plugin_external_layers() {
        let mcp_manager = McpConnectionManager::new();
        let alpha = McpServerConfig {
            name: "alpha".to_string(),
            transport: astrcode_adapter_mcp::config::McpTransportConfig::Stdio {
                command: "echo".to_string(),
                args: Vec::new(),
                env: HashMap::new(),
            },
            scope: astrcode_adapter_mcp::config::McpConfigScope::User,
            enabled: false,
            timeout_secs: 120,
            init_timeout_secs: 30,
            max_reconnect_attempts: 5,
        };
        let beta = McpServerConfig {
            name: "beta".to_string(),
            ..alpha.clone()
        };
        mcp_manager
            .reload_config(vec![alpha])
            .await
            .expect("alpha config should apply");

        let stable_local_invokers = vec![invoker("read_file", &["source:builtin"])];
        let capability_surface = Arc::new(TestCapabilitySurface::default());
        let tool_search_index =
            Arc::new(astrcode_adapter_tools::builtin_tools::tool_search::ToolSearchIndex::new());
        let capability_sync = CapabilitySurfaceSync::new(
            capability_surface,
            stable_local_invokers,
            Arc::clone(&tool_search_index),
        );
        capability_sync
            .apply_external_invokers(vec![invoker("plugin.search", &["source:plugin"])])
            .expect("previous plugin surface should apply");

        let rollback = GovernanceReloadRollback::capture(&mcp_manager, &capability_sync).await;

        mcp_manager
            .reload_config(vec![beta])
            .await
            .expect("beta config should apply");
        capability_sync
            .apply_external_invokers(Vec::new())
            .expect("candidate external surface should apply");

        rollback
            .restore(&mcp_manager, &capability_sync)
            .await
            .expect("rollback should succeed");

        let declared_names = mcp_manager
            .current_surface()
            .await
            .server_statuses
            .into_iter()
            .map(|status| status.name)
            .collect::<Vec<_>>();
        assert_eq!(declared_names, vec!["alpha".to_string()]);
        let external_names = capability_sync
            .current_external_invokers()
            .into_iter()
            .map(|invoker| invoker.capability_spec().name.to_string())
            .collect::<Vec<_>>();
        assert_eq!(external_names, vec!["plugin.search".to_string()]);
    }
}
