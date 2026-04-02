use std::path::PathBuf;
use std::sync::Arc;

use astrcode_core::{
    plugin::PluginEntry, CapabilityDescriptor, PluginHealth, PluginManifest, RuntimeCoordinator,
};
use chrono::{DateTime, Utc};
use tokio::sync::{Mutex, RwLock};

use crate::plugin_discovery::{configured_plugin_paths, discover_plugin_manifests_in};
use crate::runtime_surface_assembler::{
    assemble_runtime_surface, ActivePluginRuntime, PluginInitializer, SupervisorPluginInitializer,
};
use crate::{RuntimeObservabilitySnapshot, RuntimeService, ServiceError};

#[derive(Debug, Clone)]
pub struct RuntimeGovernanceSnapshot {
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
pub struct RuntimeReloadResult {
    pub snapshot: RuntimeGovernanceSnapshot,
    pub reloaded_at: DateTime<Utc>,
}

pub struct RuntimeGovernance {
    service: Arc<RuntimeService>,
    coordinator: Arc<RuntimeCoordinator>,
    active_plugins: RwLock<Vec<ActivePluginRuntime>>,
    reload_lock: Mutex<()>,
}

impl RuntimeGovernance {
    pub fn from_runtime(
        service: Arc<RuntimeService>,
        coordinator: Arc<RuntimeCoordinator>,
    ) -> Self {
        Self::with_active_plugins(service, coordinator, Vec::new())
    }

    pub(crate) fn with_active_plugins(
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

    pub async fn snapshot(&self) -> RuntimeGovernanceSnapshot {
        self.refresh_plugin_health().await;
        self.snapshot_with_paths(configured_plugin_paths()).await
    }

    pub async fn reload(&self) -> Result<RuntimeReloadResult, ServiceError> {
        let search_paths = configured_plugin_paths();
        let manifests =
            discover_plugin_manifests_in(&search_paths).map_err(ServiceError::Internal)?;
        let initializer = SupervisorPluginInitializer::new(search_paths.clone());
        self.reload_from_manifests(manifests, &initializer, search_paths)
            .await
    }

    pub(crate) async fn reload_from_manifests<I>(
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

        let builtin_skills = crate::builtin_skills::builtin_skills();
        let assembled = assemble_runtime_surface(
            manifests,
            initializer,
            self.coordinator.plugin_registry(),
            builtin_skills.clone(),
        )
        .await
        .map_err(ServiceError::Internal)?;
        let capability_surface = assembled.router.descriptors();
        self.service
            .replace_capabilities_with_prompt_inputs(
                assembled.router,
                assembled.prompt_declarations,
                builtin_skills,
            )
            .await?;
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
