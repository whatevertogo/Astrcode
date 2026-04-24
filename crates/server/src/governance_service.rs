//! server-owned governance bridge。
//!
//! server 状态面只暴露本地治理 contract。

use std::{path::PathBuf, sync::Arc};

use astrcode_core::{CapabilitySpec, RuntimeObservabilitySnapshot};
use astrcode_plugin_host::PluginEntry;
use async_trait::async_trait;

#[derive(Debug, Clone)]
pub(crate) struct ServerGovernanceSnapshot {
    pub runtime_name: String,
    pub runtime_kind: String,
    pub loaded_session_count: usize,
    pub running_session_ids: Vec<String>,
    pub plugin_search_paths: Vec<PathBuf>,
    pub metrics: RuntimeObservabilitySnapshot,
    pub capabilities: Vec<CapabilitySpec>,
    pub plugins: Vec<PluginEntry>,
}

#[derive(Debug, Clone)]
pub(crate) struct ServerGovernanceReloadResult {
    pub snapshot: ServerGovernanceSnapshot,
    pub reloaded_at: chrono::DateTime<chrono::Utc>,
}

#[async_trait]
pub(crate) trait ServerGovernancePort: Send + Sync {
    fn capabilities(&self) -> Vec<CapabilitySpec>;

    async fn reload(
        &self,
    ) -> Result<ServerGovernanceReloadResult, crate::application_error_bridge::ServerRouteError>;

    async fn shutdown(
        &self,
        timeout_secs: u64,
    ) -> Result<(), crate::application_error_bridge::ServerRouteError>;
}

pub(crate) struct ServerGovernanceService {
    port: Arc<dyn ServerGovernancePort>,
}

impl ServerGovernanceService {
    pub(crate) fn new(port: Arc<dyn ServerGovernancePort>) -> Self {
        Self { port }
    }

    pub(crate) fn capabilities(&self) -> Vec<CapabilitySpec> {
        self.port.capabilities()
    }

    pub(crate) async fn reload(
        &self,
    ) -> Result<ServerGovernanceReloadResult, crate::application_error_bridge::ServerRouteError>
    {
        self.port.reload().await
    }

    pub(crate) async fn shutdown(
        &self,
        timeout_secs: u64,
    ) -> Result<(), crate::application_error_bridge::ServerRouteError> {
        self.port.shutdown(timeout_secs).await
    }
}

impl std::fmt::Debug for ServerGovernanceService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerGovernanceService")
            .finish_non_exhaustive()
    }
}
