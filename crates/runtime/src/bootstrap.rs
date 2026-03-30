use std::sync::Arc;

use astrcode_core::{AstrError, PluginManifest, PluginRegistry, RuntimeCoordinator, RuntimeHandle};

use crate::plugin_discovery::{configured_plugin_paths, discover_plugin_manifests_in};
use crate::runtime_governance::RuntimeGovernance;
use crate::runtime_surface_assembler::{
    assemble_runtime_surface, PluginInitializer, SupervisorPluginInitializer,
};
use crate::{RuntimeService, ServiceError};

pub struct RuntimeBootstrap {
    pub service: Arc<RuntimeService>,
    pub coordinator: Arc<RuntimeCoordinator>,
    pub governance: Arc<RuntimeGovernance>,
}

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
    let governance = Arc::new(RuntimeGovernance::with_active_plugins(
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

fn service_error_to_astr(error: ServiceError) -> AstrError {
    match error {
        ServiceError::NotFound(message)
        | ServiceError::Conflict(message)
        | ServiceError::InvalidInput(message) => AstrError::Validation(message),
        ServiceError::Internal(error) => AstrError::Internal(error.to_string()),
    }
}
