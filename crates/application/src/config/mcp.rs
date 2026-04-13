//! MCP 配置相关用例与 JSON 结构辅助函数。
//!
//! 为什么单独拆模块：
//! `ConfigService` 的通用配置职责和 MCP 声明编排是两套问题域。
//! 把 MCP 读写、作用域决策、JSON 结构变换集中到这里，`mod.rs`
//! 就能只保留通用配置入口与非 MCP 逻辑。

use std::path::Path;

use serde_json::{Map, Value};

use super::{ConfigService, McpConfigFileScope, validation};
use crate::{
    ApplicationError,
    mcp::{McpConfigScope, RegisterMcpServerInput},
};

impl ConfigService {
    /// 读取指定作用域的独立 `mcp.json`。
    pub fn load_mcp(
        &self,
        scope: McpConfigFileScope,
        working_dir: Option<&Path>,
    ) -> std::result::Result<Option<Value>, ApplicationError> {
        self.store.load_mcp(scope, working_dir).map_err(Into::into)
    }

    /// 按 scope 持久化 MCP 服务器声明。
    pub async fn upsert_mcp_server(
        &self,
        working_dir: &Path,
        input: &RegisterMcpServerInput,
    ) -> std::result::Result<(), ApplicationError> {
        let entry = register_input_to_mcp_entry(input)?;
        match input.scope {
            McpConfigScope::User => {
                let user_sidecar = self.store.load_mcp(McpConfigFileScope::User, None)?;
                if user_sidecar.is_some() {
                    let next = upsert_mcp_entry(user_sidecar, &input.name, entry)?;
                    self.store
                        .save_mcp(McpConfigFileScope::User, None, Some(&next))?;
                    return Ok(());
                }

                let mut config = validation::normalize_config(self.store.load()?)?;
                if mcp_document_contains_server(config.mcp.as_ref(), &input.name)? {
                    config.mcp = Some(upsert_mcp_entry(config.mcp.take(), &input.name, entry)?);
                    self.store.save(&config)?;
                    let mut guard = self.config.write().await;
                    *guard = config;
                    return Ok(());
                }

                let next = upsert_mcp_entry(None, &input.name, entry)?;
                self.store
                    .save_mcp(McpConfigFileScope::User, None, Some(&next))?;
                Ok(())
            },
            McpConfigScope::Local => {
                let local_sidecar = self
                    .store
                    .load_mcp(McpConfigFileScope::Local, Some(working_dir))?;
                if local_sidecar.is_some() {
                    let next = upsert_mcp_entry(local_sidecar, &input.name, entry)?;
                    self.store.save_mcp(
                        McpConfigFileScope::Local,
                        Some(working_dir),
                        Some(&next),
                    )?;
                    return Ok(());
                }

                let mut overlay = self.store.load_overlay(working_dir)?.unwrap_or_default();
                if mcp_document_contains_server(overlay.mcp.as_ref(), &input.name)? {
                    overlay.mcp = Some(upsert_mcp_entry(overlay.mcp.take(), &input.name, entry)?);
                    self.store.save_overlay(working_dir, &overlay)?;
                    return Ok(());
                }

                let next = upsert_mcp_entry(None, &input.name, entry)?;
                self.store
                    .save_mcp(McpConfigFileScope::Local, Some(working_dir), Some(&next))?;
                Ok(())
            },
            McpConfigScope::Project => {
                let project_mcp = self
                    .store
                    .load_mcp(McpConfigFileScope::Project, Some(working_dir))?;
                let next = upsert_mcp_entry(project_mcp, &input.name, entry)?;
                self.store
                    .save_mcp(McpConfigFileScope::Project, Some(working_dir), Some(&next))?;
                Ok(())
            },
        }
    }

    /// 删除指定 scope 的 MCP 声明。
    pub async fn remove_mcp_server(
        &self,
        working_dir: &Path,
        scope: McpConfigScope,
        name: &str,
    ) -> std::result::Result<(), ApplicationError> {
        match scope {
            McpConfigScope::User => {
                let user_sidecar = self.store.load_mcp(McpConfigFileScope::User, None)?;
                if mcp_document_contains_server(user_sidecar.as_ref(), name)? {
                    let next = remove_mcp_entry(user_sidecar, name)?;
                    self.store
                        .save_mcp(McpConfigFileScope::User, None, next.as_ref())?;
                    return Ok(());
                }

                let mut config = validation::normalize_config(self.store.load()?)?;
                config.mcp = remove_mcp_entry(config.mcp.take(), name)?;
                self.store.save(&config)?;
                let mut guard = self.config.write().await;
                *guard = config;
                Ok(())
            },
            McpConfigScope::Local => {
                let local_sidecar = self
                    .store
                    .load_mcp(McpConfigFileScope::Local, Some(working_dir))?;
                if mcp_document_contains_server(local_sidecar.as_ref(), name)? {
                    let next = remove_mcp_entry(local_sidecar, name)?;
                    self.store.save_mcp(
                        McpConfigFileScope::Local,
                        Some(working_dir),
                        next.as_ref(),
                    )?;
                    return Ok(());
                }

                let mut overlay = self.store.load_overlay(working_dir)?.unwrap_or_default();
                overlay.mcp = remove_mcp_entry(overlay.mcp.take(), name)?;
                self.store.save_overlay(working_dir, &overlay)?;
                Ok(())
            },
            McpConfigScope::Project => {
                let project_mcp = self
                    .store
                    .load_mcp(McpConfigFileScope::Project, Some(working_dir))?;
                let next = remove_mcp_entry(project_mcp, name)?;
                self.store.save_mcp(
                    McpConfigFileScope::Project,
                    Some(working_dir),
                    next.as_ref(),
                )?;
                Ok(())
            },
        }
    }

    /// 启用或禁用已声明的 MCP 服务器。
    pub async fn set_mcp_server_enabled(
        &self,
        working_dir: &Path,
        scope: McpConfigScope,
        name: &str,
        enabled: bool,
    ) -> std::result::Result<(), ApplicationError> {
        match scope {
            McpConfigScope::User => {
                let user_sidecar = self.store.load_mcp(McpConfigFileScope::User, None)?;
                if mcp_document_contains_server(user_sidecar.as_ref(), name)? {
                    let next = set_mcp_entry_enabled(user_sidecar, name, enabled)?;
                    self.store
                        .save_mcp(McpConfigFileScope::User, None, next.as_ref())?;
                    return Ok(());
                }

                let mut config = validation::normalize_config(self.store.load()?)?;
                config.mcp = set_mcp_entry_enabled(config.mcp.take(), name, enabled)?;
                self.store.save(&config)?;
                let mut guard = self.config.write().await;
                *guard = config;
                Ok(())
            },
            McpConfigScope::Local => {
                let local_sidecar = self
                    .store
                    .load_mcp(McpConfigFileScope::Local, Some(working_dir))?;
                if mcp_document_contains_server(local_sidecar.as_ref(), name)? {
                    let next = set_mcp_entry_enabled(local_sidecar, name, enabled)?;
                    self.store.save_mcp(
                        McpConfigFileScope::Local,
                        Some(working_dir),
                        next.as_ref(),
                    )?;
                    return Ok(());
                }

                let mut overlay = self.store.load_overlay(working_dir)?.unwrap_or_default();
                overlay.mcp = set_mcp_entry_enabled(overlay.mcp.take(), name, enabled)?;
                self.store.save_overlay(working_dir, &overlay)?;
                Ok(())
            },
            McpConfigScope::Project => {
                let project_mcp = self
                    .store
                    .load_mcp(McpConfigFileScope::Project, Some(working_dir))?;
                let next = set_mcp_entry_enabled(project_mcp, name, enabled)?;
                self.store.save_mcp(
                    McpConfigFileScope::Project,
                    Some(working_dir),
                    next.as_ref(),
                )?;
                Ok(())
            },
        }
    }
}

fn upsert_mcp_entry(
    current: Option<Value>,
    name: &str,
    entry: Value,
) -> std::result::Result<Value, ApplicationError> {
    let mut doc = current.unwrap_or_else(empty_mcp_document);
    let servers = mcp_servers_mut(&mut doc)?;
    servers.insert(name.to_string(), entry);
    Ok(doc)
}

fn remove_mcp_entry(
    current: Option<Value>,
    name: &str,
) -> std::result::Result<Option<Value>, ApplicationError> {
    let Some(mut doc) = current else {
        return Ok(None);
    };
    let servers = mcp_servers_mut(&mut doc)?;
    servers.remove(name);
    Ok(normalize_mcp_document(doc))
}

fn set_mcp_entry_enabled(
    current: Option<Value>,
    name: &str,
    enabled: bool,
) -> std::result::Result<Option<Value>, ApplicationError> {
    let Some(mut doc) = current else {
        return Err(ApplicationError::NotFound(format!(
            "MCP server '{}' not found",
            name
        )));
    };
    let servers = mcp_servers_mut(&mut doc)?;
    let entry = servers
        .get_mut(name)
        .ok_or_else(|| ApplicationError::NotFound(format!("MCP server '{}' not found", name)))?;
    let object = entry.as_object_mut().ok_or_else(|| {
        ApplicationError::InvalidArgument(format!("MCP server '{}' config is not an object", name))
    })?;
    if enabled {
        object.remove("disabled");
    } else {
        object.insert("disabled".to_string(), Value::Bool(true));
    }
    Ok(normalize_mcp_document(doc))
}

fn register_input_to_mcp_entry(
    input: &RegisterMcpServerInput,
) -> std::result::Result<Value, ApplicationError> {
    let transport_type = input
        .transport_config
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| ApplicationError::InvalidArgument("MCP transport missing 'type'".into()))?;
    let mut entry = Map::new();
    match transport_type {
        "stdio" => {
            let command = input
                .transport_config
                .get("command")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    ApplicationError::InvalidArgument(
                        "stdio MCP transport missing 'command'".into(),
                    )
                })?;
            entry.insert("command".to_string(), Value::String(command.to_string()));
            entry.insert(
                "args".to_string(),
                input
                    .transport_config
                    .get("args")
                    .cloned()
                    .unwrap_or_else(|| Value::Array(Vec::new())),
            );
            entry.insert(
                "env".to_string(),
                input
                    .transport_config
                    .get("env")
                    .cloned()
                    .unwrap_or_else(|| Value::Object(Map::new())),
            );
        },
        "http" | "sse" => {
            let url = input
                .transport_config
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    ApplicationError::InvalidArgument(format!(
                        "{} MCP transport missing 'url'",
                        transport_type
                    ))
                })?;
            entry.insert(
                "type".to_string(),
                Value::String(transport_type.to_string()),
            );
            entry.insert("url".to_string(), Value::String(url.to_string()));
            entry.insert(
                "headers".to_string(),
                input
                    .transport_config
                    .get("headers")
                    .cloned()
                    .unwrap_or_else(|| Value::Object(Map::new())),
            );
        },
        other => {
            return Err(ApplicationError::InvalidArgument(format!(
                "unsupported MCP transport type '{}'",
                other
            )));
        },
    }

    if !input.enabled {
        entry.insert("disabled".to_string(), Value::Bool(true));
    }
    if input.timeout_secs != 120 {
        entry.insert(
            "timeout".to_string(),
            Value::Number(input.timeout_secs.into()),
        );
    }
    if input.init_timeout_secs != 30 {
        entry.insert(
            "initTimeout".to_string(),
            Value::Number(input.init_timeout_secs.into()),
        );
    }
    if input.max_reconnect_attempts != 5 {
        entry.insert(
            "maxReconnectAttempts".to_string(),
            Value::Number(input.max_reconnect_attempts.into()),
        );
    }

    Ok(Value::Object(entry))
}

fn empty_mcp_document() -> Value {
    let mut root = Map::new();
    root.insert("mcpServers".to_string(), Value::Object(Map::new()));
    Value::Object(root)
}

fn normalize_mcp_document(doc: Value) -> Option<Value> {
    let mut doc = doc;
    if let Ok(servers) = mcp_servers_mut(&mut doc) {
        if servers.is_empty() {
            return None;
        }
    }
    Some(doc)
}

fn mcp_document_contains_server(
    current: Option<&Value>,
    name: &str,
) -> std::result::Result<bool, ApplicationError> {
    let Some(doc) = current else {
        return Ok(false);
    };
    Ok(mcp_servers(doc)?.contains_key(name))
}

fn mcp_servers_mut(
    doc: &mut Value,
) -> std::result::Result<&mut Map<String, Value>, ApplicationError> {
    let root = doc.as_object_mut().ok_or_else(|| {
        ApplicationError::InvalidArgument("MCP config root must be an object".into())
    })?;
    let servers = root
        .entry("mcpServers".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    servers
        .as_object_mut()
        .ok_or_else(|| ApplicationError::InvalidArgument("'mcpServers' must be an object".into()))
}

fn mcp_servers(doc: &Value) -> std::result::Result<&Map<String, Value>, ApplicationError> {
    let root = doc.as_object().ok_or_else(|| {
        ApplicationError::InvalidArgument("MCP config root must be an object".into())
    })?;
    let servers = root.get("mcpServers").ok_or_else(|| {
        ApplicationError::InvalidArgument("MCP config missing 'mcpServers'".into())
    })?;
    servers
        .as_object()
        .ok_or_else(|| ApplicationError::InvalidArgument("'mcpServers' must be an object".into()))
}

#[cfg(test)]
mod tests {
    use std::{
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
    };

    use astrcode_core::{Config, ConfigOverlay, Result, ports::ConfigStore};
    use serde_json::json;

    use super::*;

    #[derive(Default)]
    struct TestConfigStore {
        config: Mutex<Config>,
        overlay: Mutex<Option<ConfigOverlay>>,
        user_mcp: Mutex<Option<Value>>,
        local_mcp: Mutex<Option<Value>>,
    }

    impl ConfigStore for TestConfigStore {
        fn load(&self) -> Result<Config> {
            Ok(self.config.lock().expect("config mutex").clone())
        }

        fn save(&self, config: &Config) -> Result<()> {
            *self.config.lock().expect("config mutex") = config.clone();
            Ok(())
        }

        fn path(&self) -> PathBuf {
            PathBuf::from("test-config.json")
        }

        fn load_overlay(&self, _working_dir: &Path) -> Result<Option<ConfigOverlay>> {
            Ok(self.overlay.lock().expect("overlay mutex").clone())
        }

        fn save_overlay(&self, _working_dir: &Path, overlay: &ConfigOverlay) -> Result<()> {
            *self.overlay.lock().expect("overlay mutex") = Some(overlay.clone());
            Ok(())
        }

        fn load_mcp(
            &self,
            scope: McpConfigFileScope,
            _working_dir: Option<&Path>,
        ) -> Result<Option<Value>> {
            match scope {
                McpConfigFileScope::User => {
                    Ok(self.user_mcp.lock().expect("user mcp mutex").clone())
                },
                McpConfigFileScope::Project => Ok(None),
                McpConfigFileScope::Local => {
                    Ok(self.local_mcp.lock().expect("local mcp mutex").clone())
                },
            }
        }

        fn save_mcp(
            &self,
            scope: McpConfigFileScope,
            _working_dir: Option<&Path>,
            mcp: Option<&Value>,
        ) -> Result<()> {
            match scope {
                McpConfigFileScope::User => {
                    *self.user_mcp.lock().expect("user mcp mutex") = mcp.cloned();
                },
                McpConfigFileScope::Project => {},
                McpConfigFileScope::Local => {
                    *self.local_mcp.lock().expect("local mcp mutex") = mcp.cloned();
                },
            }
            Ok(())
        }
    }

    fn demo_stdio_input(scope: McpConfigScope, name: &str) -> RegisterMcpServerInput {
        RegisterMcpServerInput {
            name: name.to_string(),
            scope,
            enabled: true,
            timeout_secs: 120,
            init_timeout_secs: 30,
            max_reconnect_attempts: 5,
            transport_config: json!({
                "type": "stdio",
                "command": "cmd",
                "args": ["/c", "demo"]
            }),
        }
    }

    #[tokio::test]
    async fn upsert_user_mcp_prefers_existing_sidecar() {
        let store = Arc::new(TestConfigStore::default());
        let mut config = store.load().expect("config should load");
        config.mcp = Some(json!({
            "mcpServers": {
                "from-config": { "command": "config-cmd" }
            }
        }));
        store.save(&config).expect("config should save");
        store
            .save_mcp(
                McpConfigFileScope::User,
                None,
                Some(&json!({
                    "mcpServers": {
                        "from-sidecar": { "command": "sidecar-cmd" }
                    }
                })),
            )
            .expect("user sidecar should save");

        let service = ConfigService::new(store.clone());
        service
            .upsert_mcp_server(
                Path::new("."),
                &demo_stdio_input(McpConfigScope::User, "from-sidecar"),
            )
            .await
            .expect("upsert should succeed");

        let sidecar = store
            .load_mcp(McpConfigFileScope::User, None)
            .expect("sidecar should load")
            .expect("sidecar should exist");
        assert!(
            mcp_document_contains_server(Some(&sidecar), "from-sidecar")
                .expect("sidecar shape should be valid")
        );

        let persisted = store.load().expect("config should reload");
        assert!(
            mcp_document_contains_server(persisted.mcp.as_ref(), "from-config")
                .expect("config shape should be valid")
        );
    }

    #[tokio::test]
    async fn upsert_local_mcp_creates_sidecar_when_overlay_has_no_entry() {
        let project = tempfile::tempdir().expect("project tempdir should exist");
        let store = Arc::new(TestConfigStore::default());
        let service = ConfigService::new(store.clone());

        service
            .upsert_mcp_server(
                project.path(),
                &demo_stdio_input(McpConfigScope::Local, "local-demo"),
            )
            .await
            .expect("upsert should succeed");

        let local_sidecar = store
            .load_mcp(McpConfigFileScope::Local, Some(project.path()))
            .expect("local sidecar should load")
            .expect("local sidecar should exist");
        assert!(
            mcp_document_contains_server(Some(&local_sidecar), "local-demo")
                .expect("local sidecar shape should be valid")
        );
        assert!(
            store
                .load_overlay(project.path())
                .expect("overlay should load")
                .is_none(),
            "新写入的本地 MCP 应优先进入独立 sidecar，而不是污染 overlay 配置"
        );
    }
}
