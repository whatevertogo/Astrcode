//! # MCP 桥接装配
//!
//! 将 `adapter-mcp` 的 `McpConnectionManager` 适配为
//! `application` 层的 `McpPort` 端口契约，
//! 并把 MCP 的动态 capability surface 同步回 kernel。

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use astrcode_adapter_mcp::{
    config::{McpApprovalManager, McpConfigManager, McpConfigScope, McpServerConfig},
    manager::McpConnectionManager,
};
use astrcode_adapter_storage::mcp_settings_store::FileMcpSettingsStore;
use astrcode_application::{
    ApplicationError, ConfigService, McpConfigFileScope, McpPort, McpServerStatusView,
    RegisterMcpServerInput,
};
use async_trait::async_trait;

use super::capabilities::CapabilitySurfaceSync;

/// 构建并初始化 MCP 连接管理器。
pub(crate) async fn bootstrap_mcp_manager(
    config_service: Arc<ConfigService>,
    working_dir: &Path,
    approvals_path: PathBuf,
) -> astrcode_core::Result<Arc<McpConnectionManager>> {
    let approval_store = FileMcpSettingsStore::new(approvals_path);
    let manager = Arc::new(McpConnectionManager::new().with_approval(
        McpApprovalManager::new(Box::new(approval_store)),
        working_dir.to_string_lossy().to_string(),
    ));
    let configs = load_declared_configs(&config_service, working_dir)
        .await
        .map_err(|error| astrcode_core::AstrError::Internal(error.to_string()))?;
    let results = manager.connect_all(configs).await;
    for (name, error) in results.failed {
        log::warn!("MCP server '{}' 初始化失败: {}", name, error);
    }
    Ok(manager)
}

/// 构建 MCP 服务，使用 `McpConnectionManager` 作为实际端口实现。
pub(crate) fn build_mcp_service(
    config_service: Arc<ConfigService>,
    working_dir: PathBuf,
    manager: Arc<McpConnectionManager>,
    capability_sync: CapabilitySurfaceSync,
) -> Arc<astrcode_application::McpService> {
    Arc::new(astrcode_application::McpService::new(Arc::new(
        ManagerMcpPort {
            config_service,
            working_dir,
            manager,
            capability_sync,
        },
    )))
}

struct ManagerMcpPort {
    config_service: Arc<ConfigService>,
    working_dir: PathBuf,
    manager: Arc<McpConnectionManager>,
    capability_sync: CapabilitySurfaceSync,
}

impl std::fmt::Debug for ManagerMcpPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagerMcpPort").finish_non_exhaustive()
    }
}

#[async_trait]
impl McpPort for ManagerMcpPort {
    async fn list_server_status(&self) -> Vec<McpServerStatusView> {
        self.manager
            .list_status()
            .await
            .into_iter()
            .map(snapshot_to_view)
            .collect()
    }

    async fn approve_server(&self, server_signature: &str) -> Result<(), ApplicationError> {
        self.manager
            .approve_server(server_signature)
            .map_err(core_error_to_app)?;
        self.reload_from_source().await
    }

    async fn reject_server(&self, server_signature: &str) -> Result<(), ApplicationError> {
        self.manager
            .reject_server(server_signature)
            .map_err(core_error_to_app)?;
        self.reload_from_source().await
    }

    async fn reconnect_server(&self, name: &str) -> Result<(), ApplicationError> {
        self.manager
            .reconnect_server(name)
            .await
            .map_err(core_error_to_app)?;
        self.sync_capabilities().await
    }

    async fn reset_project_choices(&self) -> Result<(), ApplicationError> {
        self.manager
            .reset_project_choices()
            .map_err(core_error_to_app)?;
        self.reload_from_source().await
    }

    async fn upsert_server(&self, input: &RegisterMcpServerInput) -> Result<(), ApplicationError> {
        self.config_service
            .upsert_mcp_server(self.working_dir.as_path(), input)
            .await?;
        self.reload_from_source().await
    }

    async fn remove_server(
        &self,
        scope: astrcode_application::McpConfigScope,
        name: &str,
    ) -> Result<(), ApplicationError> {
        self.config_service
            .remove_mcp_server(self.working_dir.as_path(), scope, name)
            .await?;
        self.reload_from_source().await
    }

    async fn set_server_enabled(
        &self,
        scope: astrcode_application::McpConfigScope,
        name: &str,
        enabled: bool,
    ) -> Result<(), ApplicationError> {
        self.config_service
            .set_mcp_server_enabled(self.working_dir.as_path(), scope, name, enabled)
            .await?;
        self.reload_from_source().await
    }
}

impl ManagerMcpPort {
    async fn reload_from_source(&self) -> Result<(), ApplicationError> {
        let configs =
            load_declared_configs(&self.config_service, self.working_dir.as_path()).await?;
        self.manager
            .reload_config(configs)
            .await
            .map_err(core_error_to_app)?;
        self.sync_capabilities().await
    }

    async fn sync_capabilities(&self) -> Result<(), ApplicationError> {
        let surface = self.manager.current_surface().await;
        self.capability_sync
            .apply_external_invokers(surface.capability_invokers)
            .map_err(core_error_to_app)
    }
}

fn snapshot_to_view(
    snapshot: astrcode_adapter_mcp::manager::surface::McpServerStatusSnapshot,
) -> McpServerStatusView {
    McpServerStatusView {
        name: snapshot.name,
        scope: snapshot.scope,
        enabled: snapshot.enabled,
        state: snapshot.state,
        error: snapshot.error,
        tool_count: snapshot.tool_count,
        prompt_count: snapshot.prompt_count,
        resource_count: snapshot.resource_count,
        pending_approval: snapshot.pending_approval,
        server_signature: snapshot.server_signature,
    }
}

fn core_error_to_app(error: astrcode_core::AstrError) -> ApplicationError {
    ApplicationError::Internal(error.to_string())
}

pub(crate) async fn load_declared_configs(
    config_service: &Arc<ConfigService>,
    working_dir: &Path,
) -> Result<Vec<McpServerConfig>, ApplicationError> {
    let mut merged = HashMap::new();
    let working_dir_display = working_dir.display().to_string();

    let user_config = config_service.get_config().await;
    if let Some(mcp) = &user_config.mcp {
        merge_configs_or_warn(
            &mut merged,
            "user config.json mcp",
            McpConfigManager::load_from_value(mcp, McpConfigScope::User).map_err(core_error_to_app),
        );
    }

    match config_service.load_mcp(McpConfigFileScope::User, None) {
        Ok(Some(mcp)) => merge_configs_or_warn(
            &mut merged,
            "user mcp.json",
            McpConfigManager::load_from_value(&mcp, McpConfigScope::User)
                .map_err(core_error_to_app),
        ),
        Ok(None) => {},
        Err(error) => log::warn!("failed to load user mcp.json, skipping source: {error}"),
    }

    let project_file = working_dir.join(".mcp.json");
    if project_file.is_file() {
        merge_configs_or_warn(
            &mut merged,
            &format!("project MCP file '{}'", project_file.display()),
            McpConfigManager::load_from_file(&project_file, McpConfigScope::Project)
                .map_err(core_error_to_app),
        );
    }

    match config_service.load_overlay(working_dir) {
        Ok(Some(overlay)) => {
            if let Some(mcp) = overlay.mcp {
                merge_configs_or_warn(
                    &mut merged,
                    &format!("local overlay mcp for '{}'", working_dir_display),
                    McpConfigManager::load_from_value(&mcp, McpConfigScope::Local)
                        .map_err(core_error_to_app),
                );
            }
        },
        Ok(None) => {},
        Err(error) => log::warn!(
            "failed to load local overlay MCP config for '{}', skipping source: {error}",
            working_dir_display
        ),
    }

    match config_service.load_mcp(McpConfigFileScope::Local, Some(working_dir)) {
        Ok(Some(mcp)) => merge_configs_or_warn(
            &mut merged,
            &format!("local sidecar mcp for '{}'", working_dir_display),
            McpConfigManager::load_from_value(&mcp, McpConfigScope::Local)
                .map_err(core_error_to_app),
        ),
        Ok(None) => {},
        Err(error) => log::warn!(
            "failed to load local sidecar MCP config for '{}', skipping source: {error}",
            working_dir_display
        ),
    }

    Ok(merged.into_values().collect())
}

fn merge_configs_or_warn(
    merged: &mut HashMap<String, McpServerConfig>,
    source_label: &str,
    configs: Result<Vec<McpServerConfig>, ApplicationError>,
) {
    match configs {
        Ok(configs) => {
            for config in configs {
                merged.insert(config.name.clone(), config);
            }
        },
        Err(error) => {
            log::warn!(
                "failed to parse MCP declarations from {}, skipping source: {}",
                source_label,
                error
            );
        },
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, fs};

    use astrcode_adapter_storage::config_store::FileConfigStore;
    use astrcode_core::{Config, ConfigOverlay};
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn load_declared_configs_merges_config_and_sidecars_by_scope_precedence() {
        let temp = tempfile::tempdir().expect("tempdir should exist");
        let project = tempfile::tempdir().expect("project should exist");
        let store = Arc::new(FileConfigStore::new(
            temp.path().join(".astrcode").join("config.json"),
        ));

        let config = Config {
            mcp: Some(json!({
                "mcpServers": {
                    "shared": { "command": "user-config" },
                    "user-config-only": { "command": "user-config-only" }
                }
            })),
            ..Config::default()
        };
        store.save(&config).expect("config should save");

        store
            .save_mcp(
                McpConfigFileScope::User,
                None,
                Some(&json!({
                    "mcpServers": {
                        "shared": { "command": "user-sidecar" },
                        "user-sidecar-only": { "command": "user-sidecar-only" }
                    }
                })),
            )
            .expect("user sidecar should save");

        store
            .save_mcp(
                McpConfigFileScope::Project,
                Some(project.path()),
                Some(&json!({
                    "mcpServers": {
                        "shared": { "command": "project-shared" },
                        "project-only": { "command": "project-only" }
                    }
                })),
            )
            .expect("project mcp should save");

        store
            .save_overlay(
                project.path(),
                &ConfigOverlay {
                    mcp: Some(json!({
                        "mcpServers": {
                            "shared": { "command": "local-config" },
                            "local-config-only": { "command": "local-config-only" }
                        }
                    })),
                    ..ConfigOverlay::default()
                },
            )
            .expect("overlay should save");

        store
            .save_mcp(
                McpConfigFileScope::Local,
                Some(project.path()),
                Some(&json!({
                    "mcpServers": {
                        "shared": { "command": "local-sidecar" },
                        "local-sidecar-only": { "command": "local-sidecar-only" }
                    }
                })),
            )
            .expect("local sidecar should save");

        let config_service = Arc::new(ConfigService::new(store));
        let configs = load_declared_configs(&config_service, project.path())
            .await
            .expect("configs should load");
        let by_name: HashMap<_, _> = configs
            .into_iter()
            .map(|config| (config.name.clone(), config))
            .collect();

        assert_eq!(by_name["shared"].scope, McpConfigScope::Local);
        assert!(matches!(
            &by_name["shared"].transport,
            astrcode_adapter_mcp::config::McpTransportConfig::Stdio { command, .. }
                if command == "local-sidecar"
        ));
        assert!(by_name.contains_key("user-config-only"));
        assert!(by_name.contains_key("user-sidecar-only"));
        assert!(by_name.contains_key("project-only"));
        assert!(by_name.contains_key("local-config-only"));
        assert!(by_name.contains_key("local-sidecar-only"));
    }

    #[tokio::test]
    async fn load_declared_configs_skips_invalid_project_mcp_source() {
        let temp = tempfile::tempdir().expect("tempdir should exist");
        let project = tempfile::tempdir().expect("project should exist");
        let store = Arc::new(FileConfigStore::new(
            temp.path().join(".astrcode").join("config.json"),
        ));

        let config = Config {
            mcp: Some(json!({
                "mcpServers": {
                    "from-user-config": { "command": "user-config" }
                }
            })),
            ..Config::default()
        };
        store.save(&config).expect("config should save");

        fs::write(project.path().join(".mcp.json"), "{ invalid json")
            .expect("broken project mcp should be written");

        let config_service = Arc::new(ConfigService::new(store));
        let configs = load_declared_configs(&config_service, project.path())
            .await
            .expect("invalid project source should be skipped");
        let by_name: HashMap<_, _> = configs
            .into_iter()
            .map(|config| (config.name.clone(), config))
            .collect();

        assert!(by_name.contains_key("from-user-config"));
        assert_eq!(by_name.len(), 1);
    }
}
