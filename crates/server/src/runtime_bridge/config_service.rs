use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use astrcode_core::{
    Config, ConfigOverlay, ResolvedRuntimeConfig, TestConnectionResult, ports::McpConfigFileScope,
};
use serde_json::Value;

use crate::{
    McpConfigScope, RegisterMcpServerInput, ServerApplicationError,
    application_error_bridge::ServerRouteError,
    config::ConfigService,
    mcp_service::{ServerMcpConfigScope, ServerRegisterMcpServerInput},
};

#[derive(Clone)]
pub(crate) struct ServerConfigService {
    inner: Arc<ConfigService>,
}

impl ServerConfigService {
    pub(crate) fn new(inner: Arc<ConfigService>) -> Self {
        Self { inner }
    }

    pub(crate) fn inner(&self) -> &Arc<ConfigService> {
        &self.inner
    }

    pub(crate) async fn get_config(&self) -> Config {
        self.inner.get_config().await
    }

    pub(crate) fn config_path(&self) -> PathBuf {
        self.inner.config_path()
    }

    pub(crate) fn load_overlay(
        &self,
        working_dir: &Path,
    ) -> Result<Option<ConfigOverlay>, ServerRouteError> {
        self.inner
            .load_overlay(working_dir)
            .map_err(application_error_to_server)
    }

    pub(crate) fn load_overlayed_config(
        &self,
        working_dir: Option<&Path>,
    ) -> Result<Config, ServerRouteError> {
        self.inner
            .load_overlayed_config(working_dir)
            .map_err(application_error_to_server)
    }

    pub(crate) fn load_resolved_runtime_config(
        &self,
        working_dir: Option<&Path>,
    ) -> Result<ResolvedRuntimeConfig, ServerRouteError> {
        self.inner
            .load_resolved_runtime_config(working_dir)
            .map_err(application_error_to_server)
    }

    pub(crate) fn load_mcp(
        &self,
        scope: McpConfigFileScope,
        working_dir: Option<&Path>,
    ) -> Result<Option<Value>, ServerRouteError> {
        self.inner
            .load_mcp(scope, working_dir)
            .map_err(application_error_to_server)
    }

    pub(crate) async fn reload_from_disk(&self) -> Result<Config, ServerRouteError> {
        self.inner
            .reload_from_disk()
            .await
            .map_err(application_error_to_server)
    }

    pub(crate) async fn save_active_selection(
        &self,
        active_profile: String,
        active_model: String,
    ) -> Result<(), ServerRouteError> {
        self.inner
            .save_active_selection(active_profile, active_model)
            .await
            .map_err(application_error_to_server)
    }

    pub(crate) async fn test_connection(
        &self,
        profile_name: &str,
        model: &str,
    ) -> Result<TestConnectionResult, ServerRouteError> {
        self.inner
            .test_connection(profile_name, model)
            .await
            .map_err(application_error_to_server)
    }

    pub(crate) async fn upsert_mcp_server(
        &self,
        working_dir: &Path,
        input: &ServerRegisterMcpServerInput,
    ) -> Result<(), ServerRouteError> {
        self.inner
            .upsert_mcp_server(
                working_dir,
                &RegisterMcpServerInput {
                    name: input.name.clone(),
                    scope: server_scope_to_application(input.scope),
                    enabled: input.enabled,
                    timeout_secs: input.timeout_secs,
                    init_timeout_secs: input.init_timeout_secs,
                    max_reconnect_attempts: input.max_reconnect_attempts,
                    transport_config: input.transport_config.clone(),
                },
            )
            .await
            .map_err(application_error_to_server)
    }

    pub(crate) async fn remove_mcp_server(
        &self,
        working_dir: &Path,
        scope: ServerMcpConfigScope,
        name: &str,
    ) -> Result<(), ServerRouteError> {
        self.inner
            .remove_mcp_server(working_dir, server_scope_to_application(scope), name)
            .await
            .map_err(application_error_to_server)
    }

    pub(crate) async fn set_mcp_server_enabled(
        &self,
        working_dir: &Path,
        scope: ServerMcpConfigScope,
        name: &str,
        enabled: bool,
    ) -> Result<(), ServerRouteError> {
        self.inner
            .set_mcp_server_enabled(
                working_dir,
                server_scope_to_application(scope),
                name,
                enabled,
            )
            .await
            .map_err(application_error_to_server)
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

fn server_scope_to_application(scope: ServerMcpConfigScope) -> McpConfigScope {
    match scope {
        ServerMcpConfigScope::User => McpConfigScope::User,
        ServerMcpConfigScope::Project => McpConfigScope::Project,
        ServerMcpConfigScope::Local => McpConfigScope::Local,
    }
}
