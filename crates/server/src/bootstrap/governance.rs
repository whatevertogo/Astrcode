//! # 治理装配
//!
//! 负责把底层 `RuntimeCoordinator` 适配成应用层治理端口，
//! 并为治理入口接入真实 reload/observability 组合根。

use std::{path::PathBuf, sync::Arc};

use astrcode_adapter_mcp::manager::McpConnectionManager;
use astrcode_adapter_skills::{LayeredSkillCatalog, load_builtin_skills};
use astrcode_application::{
    AppGovernance, ApplicationError, ModeCatalog, RuntimeGovernancePort, RuntimeGovernanceSnapshot,
    RuntimeObservabilityCollector, RuntimeReloader, SessionInfoProvider, config::ConfigService,
    lifecycle::TaskRegistry,
};
use astrcode_plugin::Supervisor;
use async_trait::async_trait;

use super::{
    capabilities::CapabilitySurfaceSync,
    deps::{
        core::{AstrError, ManagedRuntimeComponent, RuntimeCoordinator, RuntimeHandle},
        session_runtime::SessionRuntime,
    },
    mcp::load_declared_configs,
    plugins::bootstrap_plugins_with_skill_root,
};

pub(crate) struct GovernanceBuildInput {
    pub session_runtime: Arc<SessionRuntime>,
    pub config_service: Arc<ConfigService>,
    pub coordinator: Arc<RuntimeCoordinator>,
    pub task_registry: Arc<TaskRegistry>,
    pub observability: Arc<RuntimeObservabilityCollector>,
    pub mcp_manager: Arc<McpConnectionManager>,
    pub capability_sync: CapabilitySurfaceSync,
    pub skill_catalog: Arc<LayeredSkillCatalog>,
    pub plugin_search_paths: Vec<PathBuf>,
    pub plugin_skill_root: PathBuf,
    pub plugin_supervisors: Vec<Arc<Supervisor>>,
    pub working_dir: PathBuf,
    pub mode_catalog: Option<Arc<ModeCatalog>>,
}

pub(crate) fn build_app_governance(input: GovernanceBuildInput) -> Arc<AppGovernance> {
    let runtime_port = Arc::new(CoordinatorGovernancePort {
        coordinator: Arc::clone(&input.coordinator),
    });
    let sessions = Arc::new(SessionRuntimeInfo {
        session_runtime: Arc::clone(&input.session_runtime),
    });
    let reloader: Arc<dyn RuntimeReloader> = Arc::new(ServerRuntimeReloader {
        config_service: Arc::clone(&input.config_service),
        coordinator: Arc::clone(&input.coordinator),
        mcp_manager: Arc::clone(&input.mcp_manager),
        capability_sync: input.capability_sync.clone(),
        skill_catalog: Arc::clone(&input.skill_catalog),
        plugin_search_paths: input.plugin_search_paths.clone(),
        plugin_skill_root: input.plugin_skill_root.clone(),
        working_dir: input.working_dir.clone(),
        mode_catalog: input.mode_catalog,
    });
    let managed_components: Vec<Arc<dyn ManagedRuntimeComponent>> = input
        .plugin_supervisors
        .into_iter()
        .map(|supervisor| supervisor as Arc<dyn ManagedRuntimeComponent>)
        .collect();
    input.coordinator.replace_runtime_surface(
        input.coordinator.plugin_registry().snapshot(),
        input.capability_sync.current_capabilities(),
        managed_components,
    );

    Arc::new(
        AppGovernance::new(
            runtime_port,
            input.task_registry,
            input.observability,
            sessions,
        )
        .with_reloader(reloader),
    )
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
            dyn std::future::Future<Output = std::result::Result<(), ApplicationError>> + Send + '_,
        >,
    > {
        Box::pin(async move {
            self.coordinator
                .shutdown(timeout_secs)
                .await
                .map_err(|error| ApplicationError::Internal(error.to_string()))
        })
    }
}

#[derive(Debug)]
pub(crate) struct AppRuntimeHandle;

#[async_trait]
impl RuntimeHandle for AppRuntimeHandle {
    fn runtime_name(&self) -> &'static str {
        "astrcode-application"
    }

    fn runtime_kind(&self) -> &'static str {
        "application"
    }

    async fn shutdown(&self, _timeout_secs: u64) -> std::result::Result<(), AstrError> {
        Ok(())
    }
}

struct SessionRuntimeInfo {
    session_runtime: Arc<SessionRuntime>,
}

impl SessionInfoProvider for SessionRuntimeInfo {
    fn loaded_session_count(&self) -> usize {
        self.session_runtime.list_sessions().len()
    }

    fn running_session_ids(&self) -> Vec<String> {
        self.session_runtime
            .list_running_sessions()
            .into_iter()
            .map(|id| id.to_string())
            .collect()
    }
}

impl std::fmt::Debug for SessionRuntimeInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionRuntimeInfo").finish_non_exhaustive()
    }
}

#[derive(Clone)]
struct ServerRuntimeReloader {
    config_service: Arc<ConfigService>,
    coordinator: Arc<RuntimeCoordinator>,
    mcp_manager: Arc<McpConnectionManager>,
    capability_sync: CapabilitySurfaceSync,
    skill_catalog: Arc<LayeredSkillCatalog>,
    plugin_search_paths: Vec<PathBuf>,
    plugin_skill_root: PathBuf,
    working_dir: PathBuf,
    mode_catalog: Option<Arc<ModeCatalog>>,
}

impl std::fmt::Debug for ServerRuntimeReloader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerRuntimeReloader")
            .field("plugin_search_paths", &self.plugin_search_paths)
            .field("plugin_skill_root", &self.plugin_skill_root)
            .finish_non_exhaustive()
    }
}

impl RuntimeReloader for ServerRuntimeReloader {
    fn reload(
        &self,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Vec<PathBuf>, ApplicationError>> + Send + '_>,
    > {
        Box::pin(async move {
            self.config_service.reload_from_disk().await?;
            let mcp_configs =
                load_declared_configs(&self.config_service, self.working_dir.as_path()).await?;
            let plugin_bootstrap = bootstrap_plugins_with_skill_root(
                self.plugin_search_paths.clone(),
                self.plugin_skill_root.clone(),
            )
            .await;

            let previous_base_skills = self.skill_catalog.base_skills();
            let mut next_base_skills = load_builtin_skills();
            next_base_skills.extend(plugin_bootstrap.skills.clone());
            if let Some(mode_catalog) = &self.mode_catalog {
                mode_catalog
                    .replace_plugin_modes(plugin_bootstrap.modes.clone())
                    .map_err(ApplicationError::from)?;
            }

            let previous_capabilities = self.capability_sync.current_capabilities();
            let previous_plugins = self.coordinator.plugin_registry().snapshot();
            let previous_components = self.coordinator.managed_components();

            let mcp_invokers = self
                .mcp_manager
                .reload_config(mcp_configs)
                .await
                .map_err(|error| ApplicationError::Internal(error.to_string()))?;
            let mut external_invokers = mcp_invokers;
            external_invokers.extend(plugin_bootstrap.invokers.clone());

            self.skill_catalog.replace_base_skills(next_base_skills);
            if let Err(error) = self
                .capability_sync
                .apply_external_invokers(external_invokers.clone())
            {
                self.skill_catalog.replace_base_skills(previous_base_skills);
                self.coordinator.replace_runtime_surface(
                    previous_plugins,
                    previous_capabilities,
                    previous_components,
                );
                return Err(ApplicationError::Internal(error.to_string()));
            }

            let managed_components: Vec<Arc<dyn ManagedRuntimeComponent>> = plugin_bootstrap
                .supervisors
                .iter()
                .cloned()
                .map(|supervisor| supervisor as Arc<dyn ManagedRuntimeComponent>)
                .collect();
            let previous_components = self.coordinator.replace_runtime_surface(
                plugin_bootstrap.registry.snapshot(),
                self.capability_sync.current_capabilities(),
                managed_components,
            );

            for component in previous_components {
                if let Err(error) = component.shutdown_component().await {
                    log::warn!(
                        "failed to shut down replaced managed component '{}': {}",
                        component.component_name(),
                        error
                    );
                }
            }

            Ok(self.plugin_search_paths.clone())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bootstrap::deps::core::{CapabilityKind, CapabilitySpec, PluginRegistry};

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
        assert_eq!(snapshot.runtime_name, "astrcode-application");
        assert_eq!(snapshot.runtime_kind, "application");
        assert_eq!(snapshot.capabilities.len(), 1);
        assert!(snapshot.plugins.is_empty());

        port.shutdown(1).await.expect("shutdown should succeed");
    }
}
