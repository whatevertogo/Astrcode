//! server-owned MCP bridge。
//!
//! server runtime / state / routes 通过这里的 contract 访问 MCP 用例。

use std::sync::Arc;

use async_trait::async_trait;

use crate::application_error_bridge::ServerRouteError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ServerMcpConfigScope {
    User,
    Project,
    Local,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServerMcpServerStatusSummary {
    pub name: String,
    pub scope: String,
    pub enabled: bool,
    pub status: String,
    pub error: Option<String>,
    pub tool_count: usize,
    pub prompt_count: usize,
    pub resource_count: usize,
    pub pending_approval: bool,
    pub server_signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServerMcpActionSummary {
    pub ok: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ServerRegisterMcpServerInput {
    pub name: String,
    pub scope: ServerMcpConfigScope,
    pub enabled: bool,
    pub timeout_secs: u64,
    pub init_timeout_secs: u64,
    pub max_reconnect_attempts: u32,
    pub transport_config: serde_json::Value,
}

#[async_trait]
pub(crate) trait ServerMcpPort: Send + Sync {
    async fn list_server_status_summary(&self) -> Vec<ServerMcpServerStatusSummary>;

    async fn approve_server(&self, server_signature: &str) -> Result<(), ServerRouteError>;

    async fn reject_server(&self, server_signature: &str) -> Result<(), ServerRouteError>;

    async fn reconnect_server(&self, name: &str) -> Result<(), ServerRouteError>;

    async fn reset_project_choices(&self) -> Result<(), ServerRouteError>;

    async fn upsert_server(
        &self,
        input: ServerRegisterMcpServerInput,
    ) -> Result<(), ServerRouteError>;

    async fn remove_server(
        &self,
        scope: ServerMcpConfigScope,
        name: &str,
    ) -> Result<(), ServerRouteError>;

    async fn set_server_enabled(
        &self,
        scope: ServerMcpConfigScope,
        name: &str,
        enabled: bool,
    ) -> Result<(), ServerRouteError>;
}

#[derive(Clone)]
pub(crate) struct ServerMcpService {
    port: Arc<dyn ServerMcpPort>,
}

impl ServerMcpService {
    pub(crate) fn new(port: Arc<dyn ServerMcpPort>) -> Self {
        Self { port }
    }

    pub(crate) async fn list_status_summary(&self) -> Vec<ServerMcpServerStatusSummary> {
        self.port.list_server_status_summary().await
    }

    pub(crate) async fn approve_server(
        &self,
        server_signature: &str,
    ) -> Result<(), ServerRouteError> {
        self.port.approve_server(server_signature).await
    }

    pub(crate) async fn reject_server(
        &self,
        server_signature: &str,
    ) -> Result<(), ServerRouteError> {
        self.port.reject_server(server_signature).await
    }

    pub(crate) async fn reconnect_server(&self, name: &str) -> Result<(), ServerRouteError> {
        self.port.reconnect_server(name).await
    }

    pub(crate) async fn reset_project_choices(&self) -> Result<(), ServerRouteError> {
        self.port.reset_project_choices().await
    }

    pub(crate) async fn upsert_config(
        &self,
        input: ServerRegisterMcpServerInput,
    ) -> Result<(), ServerRouteError> {
        self.port.upsert_server(input).await
    }

    pub(crate) async fn remove_config(
        &self,
        scope: ServerMcpConfigScope,
        name: &str,
    ) -> Result<(), ServerRouteError> {
        self.port.remove_server(scope, name).await
    }

    pub(crate) async fn set_enabled(
        &self,
        scope: ServerMcpConfigScope,
        name: &str,
        enabled: bool,
    ) -> Result<(), ServerRouteError> {
        self.port.set_server_enabled(scope, name, enabled).await
    }
}

impl ServerMcpActionSummary {
    pub(crate) fn ok() -> Self {
        Self {
            ok: true,
            message: None,
        }
    }
}
