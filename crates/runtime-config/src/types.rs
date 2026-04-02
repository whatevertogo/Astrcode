//! 配置数据结构定义。
//!
//! 本模块定义了 Astrcode 运行时配置的核心数据类型，包括：
//! - [`Config`]：顶层应用配置，包含版本、活跃配置档、运行时参数和 Provider 列表
//! - [`ConfigOverlay`]：项目级配置覆盖层，使用 `Option` 字段实现选择性覆盖
//! - [`RuntimeConfig`]：进程级运行时调优参数（工具并发、上下文压缩等）
//! - [`Profile`]：LLM Provider 配置档（API 端点、密钥、可用模型列表）
//! - [`TestResult`]：连接测试的结果封装
//!
//! # 设计要点
//!
//! 所有类型都实现了 `Serialize` / `Deserialize`，使用 `camelCase` 命名以匹配 JSON 配置文件。
//! `#[serde(default)]` 确保向后兼容——旧版配置文件缺失的字段会自动填充默认值。
//! `#[serde(deny_unknown_fields)]` 防止拼写错误导致配置项被静默忽略。

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::constants::{
    ANTHROPIC_API_KEY_ENV, CURRENT_CONFIG_VERSION, DEEPSEEK_API_KEY_ENV, PROVIDER_KIND_ANTHROPIC,
    PROVIDER_KIND_OPENAI,
};
use crate::env_resolver::env_reference;

/// 顶层应用配置。
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
///
/// # 序列化行为
///
/// 使用 `#[serde(default)]` 确保每个字段在 JSON 缺失时都有合理的默认值。
/// 首次启动时会自动生成包含默认 Deepseek 和 Anthropic 配置档的配置文件。
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
    #[serde(default = "default_config_runtime")]
    pub runtime: RuntimeConfig,
    #[serde(default = "default_config_profiles")]
    pub profiles: Vec<Profile>,
}

/// 项目级私有配置覆盖层。
///
/// 对应 `<project>/.astrcode/config.json`，允许项目覆盖用户级配置中的特定字段。
///
/// # 设计意图
///
/// 本类型刻意使用 `Option<T>` 字段而非完整结构：只有显式设置的字段才会覆盖
/// 用户级基线配置，未设置的字段保持原值不变。这使得项目配置可以非常精简，
/// 只需声明与全局配置不同的部分。
///
/// # 不包含运行时调优参数的原因
///
/// `RuntimeConfig` 中的参数（如 `max_tool_concurrency`、`auto_compact_enabled`）
/// 不在 overlay 范围内，因为 `RuntimeService` 拥有单一共享的 `AgentLoop`。
/// 如果允许项目级覆盖这些参数，配置文件会暗示一种运行时实际上无法安全执行的
/// 隔离语义——同一个进程无法同时以不同的并发上限运行两个会话。
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
/// 这些参数从 `~/.astrcode/config.json` 加载，控制运行时行为而非 Provider 连接细节。
///
/// # 为什么不在项目 overlay 中
///
/// 这些参数仅存在于用户级配置中，因为 `RuntimeService` 为整个进程维护一个共享的
/// `AgentLoop`。允许项目级覆盖会暗示这些设置可以按会话变化，但当前运行时架构
/// 无法安全地支持这种隔离。
///
/// # 字段设计
///
/// 所有字段均为 `Option<T>`，`None` 表示回退到环境变量或内置默认值。这样设计
/// 使得 `runtime` 块具有可扩展性——新增参数不需要立即在默认配置文件中设置值，
/// 避免了每次新增字段都导致默认配置文件需要更新的维护负担。
#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
#[serde(default)]
pub struct RuntimeConfig {
    /// 安全工具的最大并行执行数。
    ///
    /// `None` 表示回退到环境变量 `ASTRCODE_MAX_TOOL_CONCURRENCY`，再回退到内置默认值（10）。
    /// 保持 `Option` 类型是为了让 runtime 块具有可扩展性，不强制每个新字段都在写入默认
    /// 配置文件时立即变为 "已设置" 状态。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tool_concurrency: Option<usize>,
    /// 是否允许在模型步骤前自动执行上下文压缩。
    ///
    /// 启用后，当对话上下文接近模型上下文窗口限制时，运行时会自动压缩历史消息，
    /// 保留最近的用户回合和关键信息，释放 token 空间供后续对话使用。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_compact_enabled: Option<bool>,
    /// 触发上下文压缩的有效上下文窗口百分比阈值。
    ///
    /// 当已使用的上下文窗口达到此百分比时，压缩机制会被触发。例如 `90` 表示
    /// 使用率达到 90% 时开始压缩。有效范围 1-100。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compact_threshold_percent: Option<u8>,
    /// 单个工具结果可发送给模型的最大字节数。
    ///
    /// 用于防止过大的工具输出（如长文件内容、大量日志）消耗过多上下文窗口。
    /// 超出此限制的工具结果会被截断。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_result_max_bytes: Option<usize>,
    /// 上下文压缩时保留的最近用户回合数。
    ///
    /// 压缩过程中，最近的 N 个用户回合会原样保留不被压缩，确保模型仍能
    /// 看到最近的对话上下文。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compact_keep_recent_turns: Option<u8>,
    /// 自动续调的默认 token 预算。
    ///
    /// 设置为 0 时禁用自动续调功能。非零值表示每次初始回合后，模型可以继续
    /// 消耗最多此数量的 token 进行后续回复。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_token_budget: Option<u64>,
    /// 触发另一次续调所需的最小助手回复 token 增量。
    ///
    /// 如果上一次续调回复的 token 增量小于此值，说明模型已经接近完成，
    /// 继续续调的边际收益递减，此时停止续调。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continuation_min_delta_tokens: Option<usize>,
    /// 初始回合后允许的最大自动续调次数。
    ///
    /// 限制续调次数防止模型陷入无限循环或过度消耗 token 预算。
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
            profiles: default_profiles(),
        }
    }
}

/// LLM Provider 配置档。
///
/// 每个 Profile 定义了一个完整的 LLM 服务连接配置，包括 API 端点、认证密钥、
/// 可用模型列表和单次请求的最大 token 数。
///
/// # API Key 解析
///
/// `api_key` 字段支持三种格式（详见 [`Profile::resolve_api_key`]）：
/// - `literal:<value>`：字面值
/// - `env:<NAME>`：强制从环境变量读取
/// - 裸值：兼容旧版，尝试环境变量后回退为字面值
///
/// # Provider 类型
///
/// `provider_kind` 决定 API 协议格式：
/// - `"openai-compatible"`：使用 OpenAI Chat Completions API 格式
/// - `"anthropic"`：使用 Anthropic Messages API 格式
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
    pub models: Vec<String>,
    #[serde(default = "default_profile_max_tokens")]
    pub max_tokens: u32,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            name: "deepseek".to_string(),
            provider_kind: PROVIDER_KIND_OPENAI.to_string(),
            base_url: "https://api.deepseek.com".to_string(),
            api_key: Some(env_reference(DEEPSEEK_API_KEY_ENV)),
            models: vec!["deepseek-chat".to_string(), "deepseek-reasoner".to_string()],
            max_tokens: 8096,
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
            .field("max_tokens", &self.max_tokens)
            .finish()
    }
}

/// 连接测试的结果。
///
/// 由 `test_connection` 函数返回，用于前端展示 Provider 连接状态。
/// 无论测试成功或失败都返回 `Ok(TestResult)`，HTTP 错误被封装在 `error` 字段中
/// 而非作为 `Result::Err` 传播，这样调用方可以统一处理成功和失败两种情况。
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

// ─────────────────────────────────────────────────────────────────────────────
// Serde default factories (must be in the same module as the types they serve)
//
// 使用独立函数而非 `#[serde(default)]` + `Default` trait 的原因：
// JSON 配置文件可能只有部分字段（用户手动编辑时删掉了某些键），每个字段需要
// 自己的特定默认值（如 active_profile 默认 "deepseek" 而非空字符串），而
// `#[serde(default)]` 只能调用类型的 `Default::default()`，无法为单个字段
// 定制不同的默认值。
// ─────────────────────────────────────────────────────────────────────────────

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
    default_profiles()
}

fn default_config_runtime() -> RuntimeConfig {
    RuntimeConfig::default()
}

fn default_profiles() -> Vec<Profile> {
    vec![
        Profile {
            name: "deepseek".to_string(),
            provider_kind: PROVIDER_KIND_OPENAI.to_string(),
            base_url: "https://api.deepseek.com".to_string(),
            api_key: Some(env_reference(DEEPSEEK_API_KEY_ENV)),
            models: vec!["deepseek-chat".to_string(), "deepseek-reasoner".to_string()],
            max_tokens: 8096,
        },
        Profile {
            name: "anthropic".to_string(),
            provider_kind: PROVIDER_KIND_ANTHROPIC.to_string(),
            base_url: String::new(),
            api_key: Some(env_reference(ANTHROPIC_API_KEY_ENV)),
            models: vec![
                "claude-sonnet-4-5-20251001".to_string(),
                "claude-opus-4-5".to_string(),
            ],
            max_tokens: 8096,
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

fn default_profile_models() -> Vec<String> {
    vec!["deepseek-chat".to_string(), "deepseek-reasoner".to_string()]
}

fn default_profile_max_tokens() -> u32 {
    8096
}
