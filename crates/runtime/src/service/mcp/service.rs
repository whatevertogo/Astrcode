use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use astrcode_core::AstrError;
use astrcode_runtime_config::{
    config_path, load_config_from_path, load_config_overlay_from_path, project_overlay_path,
    save_config_overlay_to_path, save_config_to_path,
};
use astrcode_runtime_mcp::{
    config::{McpConfigManager, McpConfigScope, McpJsonFile, McpJsonServerEntry, McpServerConfig},
    manager::McpServerStatusSnapshot,
};

use crate::service::{RuntimeService, ServiceError, ServiceResult};

pub(crate) struct McpService<'a> {
    runtime: &'a RuntimeService,
}

impl<'a> McpService<'a> {
    pub(crate) fn new(runtime: &'a RuntimeService) -> Self {
        Self { runtime }
    }

    pub(crate) async fn list_status(&self) -> Vec<McpServerStatusSnapshot> {
        let manager = self.runtime.mcp_manager.read().await.clone();
        match manager {
            Some(manager) => manager.list_status().await,
            None => Vec::new(),
        }
    }

    pub(crate) async fn approve_server(&self, server_signature: &str) -> ServiceResult<()> {
        let manager = self.require_manager().await?;
        manager
            .approve_server(server_signature)
            .map_err(service_error_to_astr)?;
        self.reload_manager_from_disk(&manager).await
    }

    pub(crate) async fn reject_server(&self, server_signature: &str) -> ServiceResult<()> {
        let manager = self.require_manager().await?;
        manager
            .reject_server(server_signature)
            .map_err(service_error_to_astr)?;
        self.reload_manager_from_disk(&manager).await
    }

    pub(crate) async fn upsert_config(&self, config: McpServerConfig) -> ServiceResult<()> {
        let manager = self.require_manager().await?;
        upsert_config_in_scope(&config).map_err(service_error_to_astr)?;
        self.reload_manager_from_disk(&manager).await
    }

    pub(crate) async fn remove_config(
        &self,
        scope: McpConfigScope,
        name: &str,
    ) -> ServiceResult<()> {
        let manager = self.require_manager().await?;
        remove_config_from_scope(scope, name).map_err(service_error_to_astr)?;
        self.reload_manager_from_disk(&manager).await
    }

    pub(crate) async fn set_enabled(
        &self,
        scope: McpConfigScope,
        name: &str,
        enabled: bool,
    ) -> ServiceResult<()> {
        let manager = self.require_manager().await?;
        set_config_enabled(scope, name, enabled).map_err(service_error_to_astr)?;
        self.reload_manager_from_disk(&manager).await
    }

    pub(crate) async fn reconnect_server(&self, server_name: &str) -> ServiceResult<()> {
        let manager = self.require_manager().await?;
        manager
            .reconnect_server(server_name)
            .await
            .map_err(service_error_to_astr)
    }

    pub(crate) async fn reset_project_choices(&self) -> ServiceResult<()> {
        let manager = self.require_manager().await?;
        let project_key = current_project_key().map_err(service_error_to_astr)?;
        let store_path = approval_store_path().map_err(service_error_to_astr)?;

        let mut document = load_approval_document(&store_path).map_err(service_error_to_astr)?;
        document.projects.remove(&project_key);
        save_approval_document(&store_path, &document).map_err(service_error_to_astr)?;

        self.reload_manager_from_disk(&manager).await
    }

    async fn require_manager(
        &self,
    ) -> ServiceResult<std::sync::Arc<astrcode_runtime_mcp::manager::McpConnectionManager>> {
        self.runtime
            .mcp_manager
            .read()
            .await
            .clone()
            .ok_or_else(|| ServiceError::InvalidInput("MCP not configured".to_string()))
    }

    async fn reload_manager_from_disk(
        &self,
        manager: &std::sync::Arc<astrcode_runtime_mcp::manager::McpConnectionManager>,
    ) -> ServiceResult<()> {
        let configs = load_mcp_declared_configs().map_err(service_error_to_astr)?;
        manager
            .reload_config(configs)
            .await
            .map_err(service_error_to_astr)?;
        Ok(())
    }
}

fn service_error_to_astr(error: AstrError) -> ServiceError {
    ServiceError::Internal(error)
}

fn load_mcp_declared_configs() -> std::result::Result<Vec<McpServerConfig>, AstrError> {
    let working_dir = std::env::current_dir()
        .map_err(|error| AstrError::io("failed to resolve current directory", error))?;
    let user_config_path = config_path()?;
    let user_config = load_config_from_path(&user_config_path)?;
    let local_overlay_path = project_overlay_path(&working_dir)?;
    let local_overlay = load_config_overlay_from_path(&local_overlay_path)?;
    let project_mcp_path = working_dir.join(".mcp.json");

    let mut configs = Vec::new();
    if let Some(raw) = user_config.mcp.as_ref() {
        configs.extend(McpConfigManager::load_from_value(
            raw,
            McpConfigScope::User,
        )?);
    }
    if let Some(raw) = local_overlay
        .as_ref()
        .and_then(|overlay| overlay.mcp.as_ref())
    {
        configs.extend(McpConfigManager::load_from_value(
            raw,
            McpConfigScope::Local,
        )?);
    }
    if project_mcp_path.exists() {
        configs.extend(McpConfigManager::load_from_file(
            &project_mcp_path,
            McpConfigScope::Project,
        )?);
    }

    Ok(merge_mcp_scoped_configs(configs))
}

fn merge_mcp_scoped_configs(configs: Vec<McpServerConfig>) -> Vec<McpServerConfig> {
    let mut by_signature = HashMap::<String, McpServerConfig>::new();
    for config in configs {
        let signature = McpConfigManager::compute_signature(&config);
        match by_signature.get(&signature) {
            Some(existing) if existing.scope > config.scope => {},
            _ => {
                by_signature.insert(signature, config);
            },
        }
    }
    let mut merged = by_signature.into_values().collect::<Vec<_>>();
    merged.sort_by(|left, right| left.name.cmp(&right.name));
    merged
}

fn upsert_config_in_scope(config: &McpServerConfig) -> std::result::Result<(), AstrError> {
    match config.scope {
        McpConfigScope::User => {
            let path = config_path()?;
            let mut document = load_user_mcp_document(&path)?;
            document
                .mcp_servers
                .insert(config.name.clone(), entry_from_config(config));
            save_user_mcp_document(&path, document)
        },
        McpConfigScope::Local => {
            let path =
                project_overlay_path(&std::env::current_dir().map_err(|error| {
                    AstrError::io("failed to resolve current directory", error)
                })?)?;
            let mut document = load_local_mcp_document(&path)?;
            document
                .mcp_servers
                .insert(config.name.clone(), entry_from_config(config));
            save_local_mcp_document(&path, document)
        },
        McpConfigScope::Project => {
            let path = std::env::current_dir()
                .map_err(|error| AstrError::io("failed to resolve current directory", error))?
                .join(".mcp.json");
            let mut document = load_project_mcp_document(&path)?;
            document
                .mcp_servers
                .insert(config.name.clone(), entry_from_config(config));
            save_project_mcp_document(&path, document)
        },
    }
}

fn remove_config_from_scope(
    scope: McpConfigScope,
    name: &str,
) -> std::result::Result<(), AstrError> {
    match scope {
        McpConfigScope::User => {
            let path = config_path()?;
            let mut document = load_user_mcp_document(&path)?;
            if document.mcp_servers.remove(name).is_none() {
                return Err(AstrError::Validation(format!(
                    "MCP server '{}' not found in user config",
                    name
                )));
            }
            save_user_mcp_document(&path, document)
        },
        McpConfigScope::Local => {
            let path =
                project_overlay_path(&std::env::current_dir().map_err(|error| {
                    AstrError::io("failed to resolve current directory", error)
                })?)?;
            let mut document = load_local_mcp_document(&path)?;
            if document.mcp_servers.remove(name).is_none() {
                return Err(AstrError::Validation(format!(
                    "MCP server '{}' not found in local config",
                    name
                )));
            }
            save_local_mcp_document(&path, document)
        },
        McpConfigScope::Project => {
            let path = std::env::current_dir()
                .map_err(|error| AstrError::io("failed to resolve current directory", error))?
                .join(".mcp.json");
            let mut document = load_project_mcp_document(&path)?;
            if document.mcp_servers.remove(name).is_none() {
                return Err(AstrError::Validation(format!(
                    "MCP server '{}' not found in project config",
                    name
                )));
            }
            save_project_mcp_document(&path, document)
        },
    }
}

fn set_config_enabled(
    scope: McpConfigScope,
    name: &str,
    enabled: bool,
) -> std::result::Result<(), AstrError> {
    match scope {
        McpConfigScope::User => {
            let path = config_path()?;
            let mut document = load_user_mcp_document(&path)?;
            let entry = document.mcp_servers.get_mut(name).ok_or_else(|| {
                AstrError::Validation(format!("MCP server '{}' not found in user config", name))
            })?;
            entry.disabled = Some(!enabled);
            save_user_mcp_document(&path, document)
        },
        McpConfigScope::Local => {
            let path =
                project_overlay_path(&std::env::current_dir().map_err(|error| {
                    AstrError::io("failed to resolve current directory", error)
                })?)?;
            let mut document = load_local_mcp_document(&path)?;
            let entry = document.mcp_servers.get_mut(name).ok_or_else(|| {
                AstrError::Validation(format!("MCP server '{}' not found in local config", name))
            })?;
            entry.disabled = Some(!enabled);
            save_local_mcp_document(&path, document)
        },
        McpConfigScope::Project => {
            let path = std::env::current_dir()
                .map_err(|error| AstrError::io("failed to resolve current directory", error))?
                .join(".mcp.json");
            let mut document = load_project_mcp_document(&path)?;
            let entry = document.mcp_servers.get_mut(name).ok_or_else(|| {
                AstrError::Validation(format!("MCP server '{}' not found in project config", name))
            })?;
            entry.disabled = Some(!enabled);
            save_project_mcp_document(&path, document)
        },
    }
}

fn load_user_mcp_document(path: &Path) -> std::result::Result<McpJsonFile, AstrError> {
    let config = load_config_from_path(path)?;
    Ok(config
        .mcp
        .map(serde_json::from_value)
        .transpose()
        .map_err(|error| AstrError::parse("user MCP config", error))?
        .unwrap_or_else(empty_mcp_document))
}

fn save_user_mcp_document(
    path: &Path,
    document: McpJsonFile,
) -> std::result::Result<(), AstrError> {
    let mut config = load_config_from_path(path)?;
    config.mcp = if document.mcp_servers.is_empty() {
        None
    } else {
        Some(
            serde_json::to_value(&document)
                .map_err(|error| AstrError::parse("serialize user MCP config", error))?,
        )
    };
    save_config_to_path(path, &config)
}

fn load_local_mcp_document(path: &Path) -> std::result::Result<McpJsonFile, AstrError> {
    let overlay = load_config_overlay_from_path(path)?.unwrap_or_default();
    Ok(overlay
        .mcp
        .map(serde_json::from_value)
        .transpose()
        .map_err(|error| AstrError::parse("local MCP config", error))?
        .unwrap_or_else(empty_mcp_document))
}

fn save_local_mcp_document(
    path: &Path,
    document: McpJsonFile,
) -> std::result::Result<(), AstrError> {
    let mut overlay = load_config_overlay_from_path(path)?.unwrap_or_default();
    overlay.mcp = if document.mcp_servers.is_empty() {
        None
    } else {
        Some(
            serde_json::to_value(&document)
                .map_err(|error| AstrError::parse("serialize local MCP config", error))?,
        )
    };
    save_config_overlay_to_path(path, &overlay)
}

fn load_project_mcp_document(path: &Path) -> std::result::Result<McpJsonFile, AstrError> {
    if !path.exists() {
        return Ok(empty_mcp_document());
    }
    let content = fs::read_to_string(path)
        .map_err(|error| AstrError::io(format!("failed to read {}", path.display()), error))?;
    serde_json::from_str(&content)
        .map_err(|error| AstrError::parse(path.display().to_string(), error))
}

fn save_project_mcp_document(
    path: &Path,
    document: McpJsonFile,
) -> std::result::Result<(), AstrError> {
    if document.mcp_servers.is_empty() {
        if path.exists() {
            fs::remove_file(path).map_err(|error| {
                AstrError::io(format!("failed to remove {}", path.display()), error)
            })?;
        }
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            AstrError::io(format!("failed to create {}", parent.display()), error)
        })?;
    }
    let content = serde_json::to_vec_pretty(&document)
        .map_err(|error| AstrError::parse("serialize project MCP config", error))?;
    fs::write(path, content)
        .map_err(|error| AstrError::io(format!("failed to write {}", path.display()), error))?;
    Ok(())
}

fn entry_from_config(config: &McpServerConfig) -> McpJsonServerEntry {
    let (command, args, env, transport_type, url, headers) = match &config.transport {
        astrcode_runtime_mcp::config::McpTransportConfig::Stdio { command, args, env } => (
            Some(command.clone()),
            Some(args.clone()),
            Some(env.clone()),
            None,
            None,
            None,
        ),
        astrcode_runtime_mcp::config::McpTransportConfig::StreamableHttp {
            url, headers, ..
        } => (
            None,
            None,
            None,
            Some("http".to_string()),
            Some(url.clone()),
            Some(headers.clone()),
        ),
        astrcode_runtime_mcp::config::McpTransportConfig::Sse { url, headers, .. } => (
            None,
            None,
            None,
            Some("sse".to_string()),
            Some(url.clone()),
            Some(headers.clone()),
        ),
    };

    McpJsonServerEntry {
        command,
        args,
        env,
        transport_type,
        url,
        headers,
        disabled: Some(!config.enabled),
        timeout: Some(config.timeout_secs),
    }
}

fn empty_mcp_document() -> McpJsonFile {
    McpJsonFile {
        mcp_servers: HashMap::new(),
    }
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredMcpApprovals {
    #[serde(default)]
    projects: HashMap<String, Vec<astrcode_runtime_mcp::config::McpApprovalData>>,
}

fn approval_store_path() -> std::result::Result<PathBuf, AstrError> {
    Ok(astrcode_core::project::astrcode_dir()?.join("mcp-approvals.json"))
}

fn current_project_key() -> std::result::Result<String, AstrError> {
    let working_dir = std::env::current_dir()
        .map_err(|error| AstrError::io("failed to resolve current directory", error))?;
    Ok(fs::canonicalize(&working_dir)
        .unwrap_or(working_dir)
        .to_string_lossy()
        .to_string())
}

fn load_approval_document(path: &Path) -> std::result::Result<StoredMcpApprovals, AstrError> {
    if !path.exists() {
        return Ok(StoredMcpApprovals::default());
    }
    let content = fs::read_to_string(path)
        .map_err(|error| AstrError::io(format!("failed to read {}", path.display()), error))?;
    serde_json::from_str(&content)
        .map_err(|error| AstrError::parse(format!("failed to parse {}", path.display()), error))
}

fn save_approval_document(
    path: &Path,
    document: &StoredMcpApprovals,
) -> std::result::Result<(), AstrError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            AstrError::io(format!("failed to create {}", parent.display()), error)
        })?;
    }
    let content = serde_json::to_vec_pretty(document)
        .map_err(|error| AstrError::parse("serialize MCP approval document", error))?;
    fs::write(path, content)
        .map_err(|error| AstrError::io(format!("failed to write {}", path.display()), error))?;
    Ok(())
}
