//! 应用配置服务：配置用例、校验、装配、结果模型。
//!
//! IO 细节通过 `ConfigStore` 端口抽象，实际实现在 `adapter-storage` 的 `FileConfigStore`。
//!
//! 子模块：
//! - `constants`: 配置常量、环境变量分组与 URL 标准化辅助
//! - `env_resolver`: 环境变量引用解析（`env:` / `literal:` / 裸值）
//! - `api_key`: Profile API key 解析
//! - `mcp`: MCP 声明读写、作用域覆盖与 JSON 结构变换
//! - `selection`: Profile/Model 选择与回退逻辑
//! - `validation`: 配置规范化与验证

pub mod api_key;
pub mod constants;
pub mod env_resolver;
pub mod mcp;
pub mod selection;
#[cfg(test)]
pub(crate) mod test_support;
pub mod validation;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use astrcode_core::{Config, ConfigOverlay, TestConnectionResult};
pub use astrcode_core::{
    config::{
        DEFAULT_API_SESSION_TTL_HOURS, DEFAULT_AUTO_COMPACT_ENABLED,
        DEFAULT_COMPACT_KEEP_RECENT_TURNS, DEFAULT_COMPACT_THRESHOLD_PERCENT,
        DEFAULT_FINALIZED_AGENT_RETAIN_LIMIT, DEFAULT_INBOX_CAPACITY,
        DEFAULT_LLM_CONNECT_TIMEOUT_SECS, DEFAULT_LLM_MAX_RETRIES, DEFAULT_LLM_READ_TIMEOUT_SECS,
        DEFAULT_LLM_RETRY_BASE_DELAY_MS, DEFAULT_MAX_CONCURRENT_AGENTS,
        DEFAULT_MAX_CONCURRENT_BRANCH_DEPTH, DEFAULT_MAX_CONSECUTIVE_FAILURES,
        DEFAULT_MAX_GREP_LINES, DEFAULT_MAX_IMAGE_SIZE, DEFAULT_MAX_REACTIVE_COMPACT_ATTEMPTS,
        DEFAULT_MAX_RECOVERED_FILES, DEFAULT_MAX_SPAWN_PER_TURN, DEFAULT_MAX_SUBRUN_DEPTH,
        DEFAULT_MAX_TOOL_CONCURRENCY, DEFAULT_MAX_TRACKED_FILES, DEFAULT_PARENT_DELIVERY_CAPACITY,
        DEFAULT_RECOVERY_TOKEN_BUDGET, DEFAULT_RECOVERY_TRUNCATE_BYTES,
        DEFAULT_RESERVED_CONTEXT_SIZE, DEFAULT_SESSION_BROADCAST_CAPACITY,
        DEFAULT_SESSION_RECENT_RECORD_LIMIT, DEFAULT_SUMMARY_RESERVE_TOKENS,
        DEFAULT_TOOL_RESULT_INLINE_LIMIT, DEFAULT_TOOL_RESULT_MAX_BYTES,
        DEFAULT_TOOL_RESULT_PREVIEW_LIMIT, ResolvedAgentConfig, ResolvedRuntimeConfig,
        max_tool_concurrency, resolve_agent_config, resolve_runtime_config,
    },
    ports::{ConfigStore, McpConfigFileScope},
};
pub use constants::{
    ALL_ASTRCODE_ENV_VARS, ASTRCODE_HOME_DIR_ENV, ASTRCODE_MAX_TOOL_CONCURRENCY_ENV,
    ASTRCODE_PLUGIN_DIRS_ENV, ASTRCODE_TEST_HOME_ENV, ASTRCODE_TOOL_INLINE_LIMIT_PREFIX,
    ASTRCODE_TOOL_RESULT_INLINE_LIMIT_ENV, BUILD_ENV_VARS, CURRENT_CONFIG_VERSION,
    DEEPSEEK_API_KEY_ENV, DEFAULT_OPENAI_CONTEXT_LIMIT, ENV_REFERENCE_PREFIX, HOME_ENV_VARS,
    LITERAL_VALUE_PREFIX, OPENAI_API_KEY_ENV, OPENAI_CHAT_COMPLETIONS_API_URL,
    OPENAI_RESPONSES_API_URL, PLUGIN_ENV_VARS, PROVIDER_API_KEY_ENV_VARS, PROVIDER_KIND_OPENAI,
    RUNTIME_ENV_VARS, TAURI_ENV_TARGET_TRIPLE_ENV, resolve_openai_chat_completions_api_url,
    resolve_openai_responses_api_url,
};
pub use selection::{list_model_options, resolve_active_selection, resolve_current_model};
use tokio::sync::RwLock;

use crate::ApplicationError;

/// 配置用例入口：负责配置的读取、写入、校验和装配。
pub struct ConfigService {
    pub(super) store: Arc<dyn ConfigStore>,
    pub(super) config: Arc<RwLock<Config>>,
}

/// 单个 profile 的摘要输入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigProfileSummary {
    pub name: String,
    pub base_url: String,
    pub api_key_preview: String,
    pub models: Vec<String>,
}

/// 已解析的配置摘要输入。
///
/// 这是 protocol `ConfigView` 的共享 projection input，
/// server 只需要补上 `config_path` 和协议外层壳。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedConfigSummary {
    pub active_profile: String,
    pub active_model: String,
    pub profiles: Vec<ConfigProfileSummary>,
    pub warning: Option<String>,
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

    /// 加载带项目 overlay 的完整配置。
    pub fn load_overlayed_config(
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

    /// 加载指定工作目录下已解析并补齐默认值的运行时配置。
    pub fn load_resolved_runtime_config(
        &self,
        working_dir: Option<&Path>,
    ) -> Result<ResolvedRuntimeConfig, ApplicationError> {
        let config = self.load_overlayed_config(working_dir)?;
        Ok(resolve_runtime_config(&config.runtime))
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

/// 生成配置摘要输入，供协议层投影复用。
pub fn resolve_config_summary(config: &Config) -> Result<ResolvedConfigSummary, ApplicationError> {
    if config.profiles.is_empty() {
        return Ok(ResolvedConfigSummary {
            active_profile: String::new(),
            active_model: String::new(),
            profiles: Vec::new(),
            warning: Some("no profiles configured".to_string()),
        });
    }

    let profiles = config
        .profiles
        .iter()
        .map(|profile| ConfigProfileSummary {
            name: profile.name.clone(),
            base_url: profile.base_url.clone(),
            api_key_preview: api_key_preview(profile.api_key.as_deref()),
            models: profile
                .models
                .iter()
                .map(|model| model.id.clone())
                .collect(),
        })
        .collect();

    let selection = selection::resolve_active_selection(
        &config.active_profile,
        &config.active_model,
        &config.profiles,
    )?;

    Ok(ResolvedConfigSummary {
        active_profile: selection.active_profile,
        active_model: selection.active_model,
        profiles,
        warning: selection.warning,
    })
}

/// 生成 API key 的安全预览字符串。
///
/// 规则：
/// - `None` 或空字符串 → "未配置"
/// - `env:VAR_NAME` 前缀 → "环境变量: VAR_NAME"（不读取实际值）
/// - `literal:KEY` 前缀 → 显示 **** + 最后 4 个字符
/// - 纯大写+下划线且是有效环境变量名 → "环境变量: NAME"
/// - 长度 > 4 → 显示 "****" + 最后 4 个字符
/// - 其他 → "****"
pub fn api_key_preview(api_key: Option<&str>) -> String {
    match api_key.map(str::trim) {
        None | Some("") => "未配置".to_string(),
        Some(value) if value.starts_with("env:") => {
            let env_name = value.trim_start_matches("env:").trim();
            if env_name.is_empty() {
                "未配置".to_string()
            } else {
                format!("环境变量: {}", env_name)
            }
        },
        Some(value) if value.starts_with("literal:") => {
            let key = value.trim_start_matches("literal:").trim();
            masked_key_preview(key)
        },
        Some(value) if is_env_var_name(value) && std::env::var_os(value).is_some() => {
            format!("环境变量: {}", value)
        },
        Some(value) => masked_key_preview(value),
    }
}

fn masked_key_preview(value: &str) -> String {
    let char_starts: Vec<usize> = value.char_indices().map(|(index, _)| index).collect();

    if char_starts.len() <= 4 {
        "****".to_string()
    } else {
        let suffix_start = char_starts[char_starts.len() - 4];
        format!("****{}", &value[suffix_start..])
    }
}

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

#[cfg(test)]
mod tests {
    use astrcode_core::{ModelConfig, Profile};

    use super::*;
    use crate::config::test_support::TestConfigStore;

    fn model(id: &str) -> ModelConfig {
        ModelConfig::new(id)
    }

    #[test]
    fn load_resolved_runtime_config_materializes_defaults() {
        let service = ConfigService::new(Arc::new(TestConfigStore::default()));

        let runtime = service
            .load_resolved_runtime_config(None)
            .expect("resolved runtime should load");

        assert_eq!(runtime.agent.max_subrun_depth, DEFAULT_MAX_SUBRUN_DEPTH);
        assert_eq!(runtime.agent.max_spawn_per_turn, DEFAULT_MAX_SPAWN_PER_TURN);
        assert_eq!(
            runtime.tool_result_inline_limit,
            DEFAULT_TOOL_RESULT_INLINE_LIMIT
        );
    }

    #[test]
    fn load_resolved_runtime_config_honors_runtime_overrides() {
        let store = Arc::new(TestConfigStore::default());
        {
            let mut config = store.config.lock().expect("config mutex");
            config.runtime.llm_read_timeout_secs = Some(120);
            config.runtime.agent = Some(astrcode_core::AgentConfig {
                max_subrun_depth: Some(5),
                max_spawn_per_turn: Some(2),
                ..astrcode_core::AgentConfig::default()
            });
        }

        let service = ConfigService::new(store);
        let runtime = service
            .load_resolved_runtime_config(None)
            .expect("resolved runtime should load");

        assert_eq!(runtime.llm_read_timeout_secs, 120);
        assert_eq!(runtime.agent.max_subrun_depth, 5);
        assert_eq!(runtime.agent.max_spawn_per_turn, 2);
    }

    #[test]
    fn api_key_preview_masks_utf8_literal_without_panicking() {
        assert_eq!(
            api_key_preview(Some("literal:令牌甲乙丙丁")),
            "****甲乙丙丁"
        );
    }

    #[test]
    fn api_key_preview_masks_utf8_plain_value_without_panicking() {
        assert_eq!(api_key_preview(Some("令牌甲乙丙丁戊")), "****乙丙丁戊");
    }

    #[test]
    fn resolve_config_summary_builds_preview_and_selection() {
        let config = Config {
            active_profile: "missing".to_string(),
            active_model: "missing-model".to_string(),
            profiles: vec![Profile {
                name: "deepseek".to_string(),
                base_url: "https://example.com".to_string(),
                api_key: Some("literal:abc12345".to_string()),
                models: vec![model("deepseek-chat"), model("deepseek-reasoner")],
                ..Profile::default()
            }],
            ..Config::default()
        };

        let summary = resolve_config_summary(&config).expect("summary should resolve");

        assert_eq!(summary.active_profile, "deepseek");
        assert_eq!(summary.active_model, "deepseek-chat");
        assert!(summary.warning.is_some());
        assert_eq!(summary.profiles.len(), 1);
        assert_eq!(summary.profiles[0].api_key_preview, "****2345");
        assert_eq!(
            summary.profiles[0].models,
            vec!["deepseek-chat".to_string(), "deepseek-reasoner".to_string()]
        );
    }

    #[test]
    fn resolve_config_summary_returns_empty_state_for_missing_profiles() {
        let config = Config {
            profiles: Vec::new(),
            ..Config::default()
        };

        let summary = resolve_config_summary(&config).expect("summary should resolve");

        assert_eq!(summary.active_profile, "");
        assert_eq!(summary.active_model, "");
        assert!(summary.profiles.is_empty());
        assert_eq!(summary.warning.as_deref(), Some("no profiles configured"));
    }
}
