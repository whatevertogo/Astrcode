//! 配置数据结构定义。
//!
//! 本模块定义了 Astrcode 运行时配置的核心数据类型，包括：
//! - [`Config`]：顶层应用配置，包含版本、活跃配置档、运行时参数和 Provider 列表
//! - [`ConfigOverlay`]：项目级配置覆盖层，使用 `Option` 字段实现选择性覆盖
//! - [`RuntimeConfig`]：进程级运行时调优参数（工具并发、上下文压缩等）
//! - [`Profile`]：LLM Provider 配置档（API 端点、密钥、可用模型列表）
//! - [`ModelConfig`]：单个模型的配置与手动 limits
//! - [`TestResult`]：连接测试的结果封装
//!
//! # 设计要点
//!
//! 配置只接受当前 schema，不再兼容旧版字符串模型列表或 profile 级模型 limits。
//! 这样可以把“模型 ID、最大输出、上下文窗口”固定在同一个对象里，避免 provider 和配置层
//! 再为旧结构保留分叉逻辑。

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::constants::{
    ANTHROPIC_API_KEY_ENV, CURRENT_CONFIG_VERSION, DEEPSEEK_API_KEY_ENV,
    DEFAULT_OPENAI_CONTEXT_LIMIT, PROVIDER_KIND_ANTHROPIC, PROVIDER_KIND_OPENAI,
};
use crate::env_resolver::env_reference;

/// 顶层应用配置
///
/// 对应 `~/.astrcode/config.json` 的完整结构，是配置加载和保存的核心类型。
///
/// # 字段说明
///
/// - `version`：配置 schema 版本，用于未来迁移逻辑（当前为 `"1"`）
/// - `active_profile`：当前活跃的 Provider 配置档名称，必须存在于 `profiles` 中
/// - `active_model`：当前活跃的模型名称，必须存在于 `active_profile` 的 `models` 列表中
/// - `runtime`：进程级运行时调优参数，不随项目 overlay 覆盖
/// - `profiles`：LLM Provider 配置档列表，至少包含一个
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
    pub profiles: Option<Vec<Profile>>,
}

/// 进程级运行时调优参数。
///
/// 所有字段都保持 `Option<T>`，让 runtime 配置块可以渐进扩展，而不会强制用户在每次
/// 新增字段后都立刻更新 `config.json`。
#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
#[serde(default)]
pub struct RuntimeConfig {
    /// `None` 表示回退到环境变量，再回退到内置默认值。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tool_concurrency: Option<usize>,
    /// 自动上下文压缩开关。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_compact_enabled: Option<bool>,
    /// 自动压缩阈值百分比。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compact_threshold_percent: Option<u8>,
    /// 单个工具结果可送入模型的最大字节数。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_result_max_bytes: Option<usize>,
    /// 自动压缩时保留的最近用户回合数。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compact_keep_recent_turns: Option<u8>,
    /// 自动续调 token 预算。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_token_budget: Option<u64>,
    /// 继续生成前要求的最小增量 token 数。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continuation_min_delta_tokens: Option<usize>,
    /// 自动续调次数上限。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_continuations: Option<u8>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CURRENT_CONFIG_VERSION.to_string(),
            active_profile: "deepseek".to_string(),
            active_model: "deepseek-chat".to_string(),
            runtime: RuntimeConfig::default(),
            profiles: default_config_profiles(),
        }
    }
}

/// 单个模型配置。
///
/// OpenAI-compatible profile 必须手动提供 `max_tokens` 与 `context_limit`，因为统一协议不
/// 暴露稳定的模型 limits；Anthropic 则优先通过 Models API 自动探测，配置中的这两个值
/// 只作为远端探测失败时的本地兜底。
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
            .field("default_token_budget", &self.default_token_budget)
            .field(
                "continuation_min_delta_tokens",
                &self.continuation_min_delta_tokens,
            )
            .field("max_continuations", &self.max_continuations)
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

/// 连接测试的结果。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TestResult {
    pub success: bool,
    pub provider: String,
    pub model: String,
    pub error: Option<String>,
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
