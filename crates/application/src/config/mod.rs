//! 应用配置服务：配置用例、校验、装配、结果模型。
//!
//! IO 细节通过 `ConfigStore` 端口抽象，实际实现在 `adapter-storage` 的 `FileConfigStore`。
//!
//! 子模块：
//! - `constants`: 配置常量、默认值、环境变量分组、URL 标准化、`resolve_*` 解析函数
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
pub mod validation;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

pub use astrcode_core::ports::{ConfigStore, McpConfigFileScope};
use astrcode_core::{Config, ConfigOverlay};
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
pub use selection::{list_model_options, resolve_active_selection, resolve_current_model};
use tokio::sync::RwLock;

use crate::ApplicationError;

/// 模型连通性测试结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestConnectionResult {
    pub success: bool,
    pub provider: String,
    pub model: String,
    pub error: Option<String>,
}

/// 配置用例入口：负责配置的读取、写入、校验和装配。
pub struct ConfigService {
    pub(super) store: Arc<dyn ConfigStore>,
    pub(super) config: Arc<RwLock<Config>>,
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
