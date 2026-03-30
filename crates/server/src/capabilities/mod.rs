use std::path::PathBuf;
use std::sync::Arc;

use astrcode_core::{
    plugin::PluginEntry, AstrError, CapabilityDescriptor, ManagedRuntimeComponent, PluginHealth,
    RuntimeCoordinator,
};
use astrcode_runtime::{RuntimeObservabilitySnapshot, RuntimeService};
use async_trait::async_trait;
use chrono::{DateTime, Utc};

mod assembly;
mod governance;
#[cfg(test)]
mod tests;

pub(crate) use assembly::bootstrap_runtime;
pub(crate) use governance::RuntimeGovernance;

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

#[derive(Clone)]
pub(crate) struct ActivePluginRuntime {
    name: String,
    component: Arc<dyn ManagedPluginComponent>,
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
