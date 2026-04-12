use std::sync::Arc;

use astrcode_runtime_mcp::{
    config::{McpConfigScope, McpServerConfig},
    manager::McpServerStatusSnapshot,
};

use crate::service::{RuntimeService, ServiceResult};

mod service;

#[derive(Clone)]
pub struct McpServiceHandle {
    runtime: Arc<RuntimeService>,
}

impl McpServiceHandle {
    pub(super) fn new(runtime: Arc<RuntimeService>) -> Self {
        Self { runtime }
    }

    fn service(&self) -> service::McpService<'_> {
        service::McpService::new(self.runtime.as_ref())
    }

    pub async fn list_status(&self) -> Vec<McpServerStatusSnapshot> {
        self.service().list_status().await
    }

    pub async fn approve_server(&self, server_signature: &str) -> ServiceResult<()> {
        self.service().approve_server(server_signature).await
    }

    pub async fn reject_server(&self, server_signature: &str) -> ServiceResult<()> {
        self.service().reject_server(server_signature).await
    }

    pub async fn upsert_config(&self, config: McpServerConfig) -> ServiceResult<()> {
        self.service().upsert_config(config).await
    }

    pub async fn remove_config(&self, scope: McpConfigScope, name: &str) -> ServiceResult<()> {
        self.service().remove_config(scope, name).await
    }

    pub async fn set_enabled(
        &self,
        scope: McpConfigScope,
        name: &str,
        enabled: bool,
    ) -> ServiceResult<()> {
        self.service().set_enabled(scope, name, enabled).await
    }

    pub async fn reconnect_server(&self, server_name: &str) -> ServiceResult<()> {
        self.service().reconnect_server(server_name).await
    }

    pub async fn reset_project_choices(&self) -> ServiceResult<()> {
        self.service().reset_project_choices().await
    }
}
