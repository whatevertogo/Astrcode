//! 应用配置服务：配置用例、校验、装配、结果模型。
//!
//! IO 细节通过 `ConfigStore` 端口抽象，实际实现在 `adapter-storage` 的 `FileConfigStore`。
//!
//! 子模块：
//! - `constants`: 配置常量、默认值、环境变量分组、URL 标准化、`resolve_*` 解析函数
//! - `env_resolver`: 环境变量引用解析（`env:` / `literal:` / 裸值）
//! - `api_key`: Profile API key 解析
//! - `selection`: Profile/Model 选择与回退逻辑
//! - `validation`: 配置规范化与验证

pub mod api_key;
pub mod constants;
pub mod env_resolver;
pub mod selection;
pub mod validation;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

pub use astrcode_core::ports::{ConfigStore, McpConfigFileScope};
use astrcode_core::{Config, ConfigOverlay};
// 从 constants 模块重新导出常用常量和解析函数
pub use constants::{
    ALL_ASTRCODE_ENV_VARS, ANTHROPIC_API_KEY_ENV, ANTHROPIC_MESSAGES_API_URL,
    ANTHROPIC_MODELS_API_URL, ANTHROPIC_VERSION, ASTRCODE_HOME_DIR_ENV,
    ASTRCODE_MAX_TOOL_CONCURRENCY_ENV, ASTRCODE_PLUGIN_DIRS_ENV, ASTRCODE_TEST_HOME_ENV,
    ASTRCODE_TOOL_INLINE_LIMIT_PREFIX, ASTRCODE_TOOL_RESULT_INLINE_LIMIT_ENV, BUILD_ENV_VARS,
    CURRENT_CONFIG_VERSION, DEEPSEEK_API_KEY_ENV, DEFAULT_API_SESSION_TTL_HOURS,
    DEFAULT_AUTO_COMPACT_ENABLED, DEFAULT_COMPACT_KEEP_RECENT_TURNS,
    DEFAULT_COMPACT_THRESHOLD_PERCENT, DEFAULT_CONTINUATION_MIN_DELTA_TOKENS,
    DEFAULT_FINALIZED_AGENT_RETAIN_LIMIT, DEFAULT_INBOX_CAPACITY, DEFAULT_LLM_CONNECT_TIMEOUT_SECS,
    DEFAULT_LLM_MAX_RETRIES, DEFAULT_LLM_READ_TIMEOUT_SECS, DEFAULT_LLM_RETRY_BASE_DELAY_MS,
    DEFAULT_MAX_AGENT_DEPTH, DEFAULT_MAX_CONCURRENT_AGENTS, DEFAULT_MAX_CONCURRENT_BRANCH_DEPTH,
    DEFAULT_MAX_CONSECUTIVE_FAILURES, DEFAULT_MAX_CONTINUATIONS, DEFAULT_MAX_GREP_LINES,
    DEFAULT_MAX_IMAGE_SIZE, DEFAULT_MAX_OUTPUT_CONTINUATION_ATTEMPTS,
    DEFAULT_MAX_REACTIVE_COMPACT_ATTEMPTS, DEFAULT_MAX_RECOVERED_FILES, DEFAULT_MAX_SUBRUN_DEPTH,
    DEFAULT_MAX_TOOL_CONCURRENCY, DEFAULT_MAX_TRACKED_FILES, DEFAULT_OPENAI_CONTEXT_LIMIT,
    DEFAULT_PARENT_DELIVERY_CAPACITY, DEFAULT_RECOVERY_TOKEN_BUDGET,
    DEFAULT_RECOVERY_TRUNCATE_BYTES, DEFAULT_SESSION_BROADCAST_CAPACITY,
    DEFAULT_SESSION_RECENT_RECORD_LIMIT, DEFAULT_SUMMARY_RESERVE_TOKENS, DEFAULT_TOKEN_BUDGET,
    DEFAULT_TOOL_RESULT_INLINE_LIMIT, DEFAULT_TOOL_RESULT_MAX_BYTES,
    DEFAULT_TOOL_RESULT_PREVIEW_LIMIT, ENV_REFERENCE_PREFIX, HOME_ENV_VARS, LITERAL_VALUE_PREFIX,
    PLUGIN_ENV_VARS, PROVIDER_API_KEY_ENV_VARS, PROVIDER_KIND_ANTHROPIC, PROVIDER_KIND_OPENAI,
    RUNTIME_ENV_VARS, TAURI_ENV_TARGET_TRIPLE_ENV, max_tool_concurrency,
    resolve_agent_finalized_retain_limit, resolve_agent_inbox_capacity,
    resolve_agent_max_concurrent, resolve_agent_max_subrun_depth,
    resolve_agent_parent_delivery_capacity, resolve_aggregate_result_bytes_budget,
    resolve_anthropic_messages_api_url, resolve_anthropic_models_api_url,
    resolve_api_session_ttl_hours, resolve_auto_compact_enabled, resolve_compact_keep_recent_turns,
    resolve_compact_threshold_percent, resolve_continuation_min_delta_tokens,
    resolve_default_token_budget, resolve_llm_connect_timeout_secs, resolve_llm_max_retries,
    resolve_llm_read_timeout_secs, resolve_max_concurrent_branch_depth,
    resolve_max_consecutive_failures, resolve_max_continuations, resolve_max_grep_lines,
    resolve_max_image_size, resolve_max_output_continuation_attempts,
    resolve_max_reactive_compact_attempts, resolve_max_recovered_files,
    resolve_max_tool_concurrency, resolve_max_tracked_files,
    resolve_micro_compact_gap_threshold_secs, resolve_micro_compact_keep_recent_results,
    resolve_openai_chat_completions_api_url, resolve_recovery_token_budget,
    resolve_recovery_truncate_bytes, resolve_session_broadcast_capacity,
    resolve_session_recent_record_limit, resolve_summary_reserve_tokens,
    resolve_tool_result_inline_limit, resolve_tool_result_max_bytes,
    resolve_tool_result_preview_limit,
};
// 从 selection 模块重新导出公共 API，保持外部调用方兼容
pub use selection::{list_model_options, resolve_active_selection, resolve_current_model};
use serde_json::{Map, Value};
use tokio::sync::RwLock;

use crate::{
    ApplicationError,
    mcp::{McpConfigScope, RegisterMcpServerInput},
};

// ============================================================
// 结果模型
// ============================================================

/// 模型连通性测试结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestConnectionResult {
    pub success: bool,
    pub provider: String,
    pub model: String,
    pub error: Option<String>,
}

// ============================================================
// 配置用例服务
// ============================================================

/// 配置用例入口：负责配置的读取、写入、校验和装配。
pub struct ConfigService {
    store: Arc<dyn ConfigStore>,
    config: Arc<RwLock<Config>>,
}

impl ConfigService {
    /// 从存储创建配置服务，加载并验证配置。
    pub fn new(store: Arc<dyn ConfigStore>) -> Self {
        let initial = store
            .load()
            .and_then(validation::normalize_config)
            .unwrap_or_default();
        Self {
            store,
            config: Arc::new(RwLock::new(initial)),
        }
    }

    pub async fn get_config(&self) -> Config {
        self.config.read().await.clone()
    }

    pub fn config_path(&self) -> PathBuf {
        self.store.path()
    }

    /// 读取项目私有 overlay。
    pub fn load_overlay(
        &self,
        working_dir: &Path,
    ) -> Result<Option<ConfigOverlay>, ApplicationError> {
        self.store.load_overlay(working_dir).map_err(Into::into)
    }

    /// 读取指定作用域的独立 `mcp.json`。
    pub fn load_mcp(
        &self,
        scope: McpConfigFileScope,
        working_dir: Option<&Path>,
    ) -> Result<Option<Value>, ApplicationError> {
        self.store.load_mcp(scope, working_dir).map_err(Into::into)
    }

    /// 保存活跃 profile/model 选择。
    pub async fn save_active_selection(
        &self,
        active_profile: String,
        active_model: String,
    ) -> Result<(), ApplicationError> {
        let mut config = self.config.write().await;
        let selection =
            selection::resolve_active_selection(&active_profile, &active_model, &config.profiles)?;
        config.active_profile = selection.active_profile;
        config.active_model = selection.active_model;
        validation::normalize_config(config.clone())?;
        self.store.save(&config)?;
        Ok(())
    }

    /// 从磁盘重新加载配置（热重载用例）。
    pub async fn reload_from_disk(&self) -> Result<Config, ApplicationError> {
        let loaded = self.store.load()?;
        let normalized = validation::normalize_config(loaded)?;
        let mut guard = self.config.write().await;
        *guard = normalized.clone();
        Ok(normalized)
    }

    /// 按 scope 持久化 MCP 服务器声明。
    pub async fn upsert_mcp_server(
        &self,
        working_dir: &Path,
        input: &RegisterMcpServerInput,
    ) -> Result<(), ApplicationError> {
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
    ) -> Result<(), ApplicationError> {
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
    ) -> Result<(), ApplicationError> {
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

    /// 加载带项目 overlay 的完整配置。
    pub fn load_resolved_config(
        &self,
        working_dir: Option<&Path>,
    ) -> Result<Config, ApplicationError> {
        let mut config = validation::normalize_config(self.store.load()?)?;
        if let Some(working_dir) = working_dir {
            if let Some(overlay) = self.store.load_overlay(working_dir)? {
                config = apply_overlay(config, overlay);
            }
        }
        Ok(config)
    }

    /// 测试模型连通性（检查 profile 和 model 是否存在）。
    pub async fn test_connection(
        &self,
        profile_name: &str,
        model: &str,
    ) -> Result<TestConnectionResult, ApplicationError> {
        let config = self.config.read().await;
        let profile = config
            .profiles
            .iter()
            .find(|p| p.name == profile_name)
            .ok_or_else(|| {
                ApplicationError::InvalidArgument(format!("profile '{}' not found", profile_name))
            })?;
        let model_exists = profile.models.iter().any(|m| m.id == model);
        Ok(TestConnectionResult {
            success: model_exists,
            provider: profile.provider_kind.clone(),
            model: model.to_string(),
            error: (!model_exists).then_some(format!(
                "model '{}' not configured under profile '{}'",
                model, profile_name
            )),
        })
    }

    /// 解析指定 profile 的 API key。
    pub fn resolve_api_key_for_profile(
        &self,
        profile_name: &str,
    ) -> Result<String, ApplicationError> {
        let config = self.config.blocking_read();
        let profile = config
            .profiles
            .iter()
            .find(|p| p.name == profile_name)
            .ok_or_else(|| {
                ApplicationError::NotFound(format!("profile '{}' not found", profile_name))
            })?;
        api_key::resolve_api_key(profile).map_err(|e| ApplicationError::Internal(e.to_string()))
    }
}

// ============================================================
// 配置装配
// ============================================================

/// 应用项目 overlay 到基础配置（仅覆盖显式设置的字段）。
fn apply_overlay(mut base: Config, overlay: ConfigOverlay) -> Config {
    if let Some(active_profile) = overlay.active_profile {
        base.active_profile = active_profile;
    }
    if let Some(active_model) = overlay.active_model {
        base.active_model = active_model;
    }
    if let Some(mcp) = overlay.mcp {
        base.mcp = Some(mcp);
    }
    if let Some(profiles) = overlay.profiles {
        base.profiles = profiles;
    }
    base
}

pub fn is_env_var_name(value: &str) -> bool {
    env_resolver::is_env_var_name(value)
}

fn upsert_mcp_entry(
    current: Option<Value>,
    name: &str,
    entry: Value,
) -> Result<Value, ApplicationError> {
    let mut doc = current.unwrap_or_else(empty_mcp_document);
    let servers = mcp_servers_mut(&mut doc)?;
    servers.insert(name.to_string(), entry);
    Ok(doc)
}

fn remove_mcp_entry(current: Option<Value>, name: &str) -> Result<Option<Value>, ApplicationError> {
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
) -> Result<Option<Value>, ApplicationError> {
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

fn register_input_to_mcp_entry(input: &RegisterMcpServerInput) -> Result<Value, ApplicationError> {
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
) -> Result<bool, ApplicationError> {
    let Some(doc) = current else {
        return Ok(false);
    };
    Ok(mcp_servers(doc)?.contains_key(name))
}

fn mcp_servers_mut(doc: &mut Value) -> Result<&mut Map<String, Value>, ApplicationError> {
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

fn mcp_servers(doc: &Value) -> Result<&Map<String, Value>, ApplicationError> {
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
