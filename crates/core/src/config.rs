//! 运行时共享配置模型。
//!
//! 该模块只承载跨层共享的数据结构，不包含文件 IO、路径解析或环境变量读取流程。

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::env::{ANTHROPIC_API_KEY_ENV, DEEPSEEK_API_KEY_ENV};

const CURRENT_CONFIG_VERSION: &str = "1";
const PROVIDER_KIND_OPENAI: &str = "openai-compatible";
const PROVIDER_KIND_ANTHROPIC: &str = "anthropic";
const DEFAULT_OPENAI_CONTEXT_LIMIT: usize = 128_000;
const ENV_REFERENCE_PREFIX: &str = "env:";

/// 顶层应用配置。
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
#[serde(default)]
pub struct Config {
    #[serde(default = "default_config_version")]
    pub version: String,
    #[serde(default = "default_config_active_profile")]
    pub active_profile: String,
    #[serde(default = "default_config_active_model")]
    pub active_model: String,
    #[serde(default)]
    pub runtime: RuntimeConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp: Option<Value>,
    #[serde(default = "default_config_profiles")]
    pub profiles: Vec<Profile>,
}

/// 项目级私有配置覆盖层。
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
#[serde(default)]
pub struct ConfigOverlay {
    pub active_profile: Option<String>,
    pub active_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp: Option<Value>,
    pub profiles: Option<Vec<Profile>>,
}

/// 进程级运行时调优参数。
#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
#[serde(default)]
pub struct RuntimeConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tool_concurrency: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_compact_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compact_threshold_percent: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_result_max_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compact_keep_recent_turns: Option<u8>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_consecutive_failures: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery_truncate_bytes: Option<usize>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_token_budget: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continuation_min_delta_tokens: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_continuations: Option<u8>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_connect_timeout_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_read_timeout_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_max_retries: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_retry_base_delay_ms: Option<u64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_reactive_compact_attempts: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_continuation_attempts: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary_reserve_tokens: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tracked_files: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_recovered_files: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery_token_budget: Option<usize>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_result_inline_limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_result_preview_limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_image_size: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_grep_lines: Option<usize>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_broadcast_capacity: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_recent_record_limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_concurrent_branch_depth: Option<usize>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub aggregate_result_bytes_budget: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub micro_compact_gap_threshold_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub micro_compact_keep_recent_results: Option<usize>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_session_ttl_hours: Option<i64>,
}

/// 多 Agent 控制参数。
#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
#[serde(default)]
pub struct AgentConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_subrun_depth: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_concurrent: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finalized_retain_limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inbox_capacity: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_delivery_capacity: Option<usize>,
}

impl fmt::Debug for AgentConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentConfig")
            .field("max_subrun_depth", &self.max_subrun_depth)
            .field("max_concurrent", &self.max_concurrent)
            .field("finalized_retain_limit", &self.finalized_retain_limit)
            .field("inbox_capacity", &self.inbox_capacity)
            .field("parent_delivery_capacity", &self.parent_delivery_capacity)
            .finish()
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CURRENT_CONFIG_VERSION.to_string(),
            active_profile: "deepseek".to_string(),
            active_model: "deepseek-chat".to_string(),
            runtime: RuntimeConfig::default(),
            mcp: None,
            profiles: default_config_profiles(),
        }
    }
}

/// 单个模型配置。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ModelConfig {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_limit: Option<usize>,
}

impl ModelConfig {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            max_tokens: None,
            context_limit: None,
        }
    }
}

/// LLM Provider 配置档。
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
#[serde(default)]
pub struct Profile {
    #[serde(default = "default_profile_name")]
    pub name: String,
    #[serde(default = "default_profile_provider_kind")]
    pub provider_kind: String,
    #[serde(default = "default_profile_base_url")]
    pub base_url: String,
    #[serde(default = "default_profile_api_key")]
    pub api_key: Option<String>,
    #[serde(default = "default_profile_models")]
    pub models: Vec<ModelConfig>,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            name: "deepseek".to_string(),
            provider_kind: PROVIDER_KIND_OPENAI.to_string(),
            base_url: "https://api.deepseek.com".to_string(),
            api_key: Some(env_reference(DEEPSEEK_API_KEY_ENV)),
            models: default_profile_models(),
        }
    }
}

/// 应用 Profile/Model 回退后的最终选择结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveSelection {
    pub active_profile: String,
    pub active_model: String,
    pub warning: Option<String>,
}

/// 运行时当前将使用的有效模型信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentModelSelection {
    pub profile_name: String,
    pub model: String,
    pub provider_kind: String,
}

/// 扁平化的模型选项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelOption {
    pub profile_name: String,
    pub model: String,
    pub provider_kind: String,
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("version", &self.version)
            .field("active_profile", &self.active_profile)
            .field("active_model", &self.active_model)
            .field("runtime", &self.runtime)
            .field("profiles", &self.profiles)
            .finish()
    }
}

impl fmt::Debug for RuntimeConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeConfig")
            .field("max_tool_concurrency", &self.max_tool_concurrency)
            .field("auto_compact_enabled", &self.auto_compact_enabled)
            .field("compact_threshold_percent", &self.compact_threshold_percent)
            .field("tool_result_max_bytes", &self.tool_result_max_bytes)
            .field("compact_keep_recent_turns", &self.compact_keep_recent_turns)
            .field("agent", &self.agent)
            .field("max_consecutive_failures", &self.max_consecutive_failures)
            .field("recovery_truncate_bytes", &self.recovery_truncate_bytes)
            .field("default_token_budget", &self.default_token_budget)
            .field(
                "continuation_min_delta_tokens",
                &self.continuation_min_delta_tokens,
            )
            .field("max_continuations", &self.max_continuations)
            .field("llm_connect_timeout_secs", &self.llm_connect_timeout_secs)
            .field("llm_read_timeout_secs", &self.llm_read_timeout_secs)
            .field("llm_max_retries", &self.llm_max_retries)
            .field(
                "max_reactive_compact_attempts",
                &self.max_reactive_compact_attempts,
            )
            .field(
                "max_output_continuation_attempts",
                &self.max_output_continuation_attempts,
            )
            .field("summary_reserve_tokens", &self.summary_reserve_tokens)
            .field("max_tracked_files", &self.max_tracked_files)
            .field("max_recovered_files", &self.max_recovered_files)
            .field("recovery_token_budget", &self.recovery_token_budget)
            .field("tool_result_inline_limit", &self.tool_result_inline_limit)
            .field("tool_result_preview_limit", &self.tool_result_preview_limit)
            .field("max_image_size", &self.max_image_size)
            .field("max_grep_lines", &self.max_grep_lines)
            .field(
                "session_broadcast_capacity",
                &self.session_broadcast_capacity,
            )
            .field(
                "session_recent_record_limit",
                &self.session_recent_record_limit,
            )
            .field(
                "max_concurrent_branch_depth",
                &self.max_concurrent_branch_depth,
            )
            .field(
                "aggregate_result_bytes_budget",
                &self.aggregate_result_bytes_budget,
            )
            .field(
                "micro_compact_gap_threshold_secs",
                &self.micro_compact_gap_threshold_secs,
            )
            .field(
                "micro_compact_keep_recent_results",
                &self.micro_compact_keep_recent_results,
            )
            .field("api_session_ttl_hours", &self.api_session_ttl_hours)
            .finish()
    }
}

impl fmt::Debug for Profile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Profile")
            .field("name", &self.name)
            .field("provider_kind", &self.provider_kind)
            .field("base_url", &self.base_url)
            .field("api_key", &redacted_api_key(self.api_key.as_deref()))
            .field("models", &self.models)
            .finish()
    }
}

fn redacted_api_key(value: Option<&str>) -> &'static str {
    if value.is_some() {
        "<redacted>"
    } else {
        "<unset>"
    }
}

fn default_config_version() -> String {
    CURRENT_CONFIG_VERSION.to_string()
}

fn default_config_active_profile() -> String {
    "deepseek".to_string()
}

fn default_config_active_model() -> String {
    "deepseek-chat".to_string()
}

fn default_config_profiles() -> Vec<Profile> {
    vec![
        Profile {
            name: "deepseek".to_string(),
            provider_kind: PROVIDER_KIND_OPENAI.to_string(),
            base_url: "https://api.deepseek.com".to_string(),
            api_key: Some(env_reference(DEEPSEEK_API_KEY_ENV)),
            models: vec![
                ModelConfig {
                    id: "deepseek-chat".to_string(),
                    max_tokens: Some(8096),
                    context_limit: Some(DEFAULT_OPENAI_CONTEXT_LIMIT),
                },
                ModelConfig {
                    id: "deepseek-reasoner".to_string(),
                    max_tokens: Some(8096),
                    context_limit: Some(DEFAULT_OPENAI_CONTEXT_LIMIT),
                },
            ],
        },
        Profile {
            name: "anthropic".to_string(),
            provider_kind: PROVIDER_KIND_ANTHROPIC.to_string(),
            base_url: String::new(),
            api_key: Some(env_reference(ANTHROPIC_API_KEY_ENV)),
            models: vec![
                ModelConfig::new("claude-sonnet-4-5-20251001"),
                ModelConfig::new("claude-opus-4-5"),
            ],
        },
    ]
}

fn default_profile_name() -> String {
    "deepseek".to_string()
}

fn default_profile_provider_kind() -> String {
    PROVIDER_KIND_OPENAI.to_string()
}

fn default_profile_base_url() -> String {
    "https://api.deepseek.com".to_string()
}

fn default_profile_api_key() -> Option<String> {
    Some(env_reference(DEEPSEEK_API_KEY_ENV))
}

fn default_profile_models() -> Vec<ModelConfig> {
    vec![
        ModelConfig {
            id: "deepseek-chat".to_string(),
            max_tokens: Some(8096),
            context_limit: Some(DEFAULT_OPENAI_CONTEXT_LIMIT),
        },
        ModelConfig {
            id: "deepseek-reasoner".to_string(),
            max_tokens: Some(8096),
            context_limit: Some(DEFAULT_OPENAI_CONTEXT_LIMIT),
        },
    ]
}

fn env_reference(name: &str) -> String {
    format!("{ENV_REFERENCE_PREFIX}{name}")
}
