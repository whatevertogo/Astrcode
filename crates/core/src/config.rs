//! 运行时共享配置模型。
//!
//! 该模块只承载跨层共享的数据结构，不包含文件 IO、路径解析或环境变量读取流程。

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::env::{ANTHROPIC_API_KEY_ENV, ASTRCODE_MAX_TOOL_CONCURRENCY_ENV, DEEPSEEK_API_KEY_ENV};

const CURRENT_CONFIG_VERSION: &str = "1";
const PROVIDER_KIND_OPENAI: &str = "openai-compatible";
const PROVIDER_KIND_ANTHROPIC: &str = "anthropic";
const DEFAULT_OPENAI_CONTEXT_LIMIT: usize = 128_000;
const ENV_REFERENCE_PREFIX: &str = "env:";

/// 默认受控子会话最大深度。
pub const DEFAULT_MAX_SUBRUN_DEPTH: usize = 2;
/// 默认单轮最多新建的子代理数量。
pub const DEFAULT_MAX_SPAWN_PER_TURN: usize = 6;

/// 默认最大安全工具并发数。
pub const DEFAULT_MAX_TOOL_CONCURRENCY: usize = 10;
pub const DEFAULT_AUTO_COMPACT_ENABLED: bool = true;
pub const DEFAULT_COMPACT_THRESHOLD_PERCENT: u8 = 90;
pub const DEFAULT_TOOL_RESULT_MAX_BYTES: usize = 100_000;
pub const DEFAULT_COMPACT_KEEP_RECENT_TURNS: u8 = 4;
pub const DEFAULT_MAX_STEPS: usize = 50;
pub const DEFAULT_LLM_CONNECT_TIMEOUT_SECS: u64 = 10;
pub const DEFAULT_LLM_READ_TIMEOUT_SECS: u64 = 90;
pub const DEFAULT_LLM_MAX_RETRIES: u32 = 2;
pub const DEFAULT_LLM_RETRY_BASE_DELAY_MS: u64 = 250;
pub const DEFAULT_MAX_REACTIVE_COMPACT_ATTEMPTS: u8 = 3;
pub const DEFAULT_MAX_OUTPUT_CONTINUATION_ATTEMPTS: u8 = 3;
pub const DEFAULT_MAX_CONTINUATIONS: u8 = 3;
pub const DEFAULT_SUMMARY_RESERVE_TOKENS: usize = 20_000;
pub const DEFAULT_MAX_TRACKED_FILES: usize = 10;
pub const DEFAULT_MAX_RECOVERED_FILES: usize = 5;
pub const DEFAULT_RECOVERY_TOKEN_BUDGET: usize = 50_000;
pub const DEFAULT_TOOL_RESULT_INLINE_LIMIT: usize = 32 * 1024;
pub const DEFAULT_TOOL_RESULT_PREVIEW_LIMIT: usize = 2 * 1024;
pub const DEFAULT_MAX_IMAGE_SIZE: usize = 20 * 1024 * 1024;
pub const DEFAULT_MAX_GREP_LINES: usize = 500;
pub const DEFAULT_SESSION_BROADCAST_CAPACITY: usize = 2048;
pub const DEFAULT_SESSION_RECENT_RECORD_LIMIT: usize = 4096;
pub const DEFAULT_MAX_CONCURRENT_BRANCH_DEPTH: usize = 3;
pub const DEFAULT_AGGREGATE_RESULT_BYTES_BUDGET: usize = 200_000;
pub const DEFAULT_MICRO_COMPACT_GAP_THRESHOLD_SECS: u64 = 3600;
pub const DEFAULT_MICRO_COMPACT_KEEP_RECENT_RESULTS: usize = 10;
pub const DEFAULT_MAX_CONCURRENT_AGENTS: usize = 8;
pub const DEFAULT_FINALIZED_AGENT_RETAIN_LIMIT: usize = 256;
pub const DEFAULT_INBOX_CAPACITY: usize = 1024;
pub const DEFAULT_PARENT_DELIVERY_CAPACITY: usize = 1024;
pub const DEFAULT_MAX_CONSECUTIVE_FAILURES: usize = 3;
pub const DEFAULT_RECOVERY_TRUNCATE_BYTES: usize = 30_000;
pub const DEFAULT_API_SESSION_TTL_HOURS: i64 = 8;

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
    pub max_steps: Option<usize>,

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
    pub max_continuations: Option<u8>,
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
    pub max_spawn_per_turn: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_concurrent: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finalized_retain_limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inbox_capacity: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_delivery_capacity: Option<usize>,
}

/// 已补齐默认值的 Agent 执行配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAgentConfig {
    pub max_subrun_depth: usize,
    pub max_spawn_per_turn: usize,
    pub max_concurrent: usize,
    pub finalized_retain_limit: usize,
    pub inbox_capacity: usize,
    pub parent_delivery_capacity: usize,
}

/// 已补齐默认值的 Runtime 执行配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRuntimeConfig {
    pub max_tool_concurrency: usize,
    pub auto_compact_enabled: bool,
    pub compact_threshold_percent: u8,
    pub tool_result_max_bytes: usize,
    pub compact_keep_recent_turns: u8,
    pub agent: ResolvedAgentConfig,
    pub max_consecutive_failures: usize,
    pub recovery_truncate_bytes: usize,
    pub max_steps: usize,
    pub llm_connect_timeout_secs: u64,
    pub llm_read_timeout_secs: u64,
    pub llm_max_retries: u32,
    pub llm_retry_base_delay_ms: u64,
    pub max_reactive_compact_attempts: u8,
    pub max_output_continuation_attempts: u8,
    pub max_continuations: u8,
    pub summary_reserve_tokens: usize,
    pub max_tracked_files: usize,
    pub max_recovered_files: usize,
    pub recovery_token_budget: usize,
    pub tool_result_inline_limit: usize,
    pub tool_result_preview_limit: usize,
    pub max_image_size: usize,
    pub max_grep_lines: usize,
    pub session_broadcast_capacity: usize,
    pub session_recent_record_limit: usize,
    pub max_concurrent_branch_depth: usize,
    pub aggregate_result_bytes_budget: usize,
    pub micro_compact_gap_threshold_secs: u64,
    pub micro_compact_keep_recent_results: usize,
    pub api_session_ttl_hours: i64,
}

impl fmt::Debug for AgentConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentConfig")
            .field("max_subrun_depth", &self.max_subrun_depth)
            .field("max_spawn_per_turn", &self.max_spawn_per_turn)
            .field("max_concurrent", &self.max_concurrent)
            .field("finalized_retain_limit", &self.finalized_retain_limit)
            .field("inbox_capacity", &self.inbox_capacity)
            .field("parent_delivery_capacity", &self.parent_delivery_capacity)
            .finish()
    }
}

impl Default for ResolvedAgentConfig {
    fn default() -> Self {
        Self {
            max_subrun_depth: DEFAULT_MAX_SUBRUN_DEPTH,
            max_spawn_per_turn: DEFAULT_MAX_SPAWN_PER_TURN,
            max_concurrent: DEFAULT_MAX_CONCURRENT_AGENTS,
            finalized_retain_limit: DEFAULT_FINALIZED_AGENT_RETAIN_LIMIT,
            inbox_capacity: DEFAULT_INBOX_CAPACITY,
            parent_delivery_capacity: DEFAULT_PARENT_DELIVERY_CAPACITY,
        }
    }
}

impl Default for ResolvedRuntimeConfig {
    fn default() -> Self {
        Self {
            max_tool_concurrency: max_tool_concurrency(),
            auto_compact_enabled: DEFAULT_AUTO_COMPACT_ENABLED,
            compact_threshold_percent: DEFAULT_COMPACT_THRESHOLD_PERCENT,
            tool_result_max_bytes: DEFAULT_TOOL_RESULT_MAX_BYTES,
            compact_keep_recent_turns: DEFAULT_COMPACT_KEEP_RECENT_TURNS,
            agent: ResolvedAgentConfig::default(),
            max_consecutive_failures: DEFAULT_MAX_CONSECUTIVE_FAILURES,
            recovery_truncate_bytes: DEFAULT_RECOVERY_TRUNCATE_BYTES,
            max_steps: DEFAULT_MAX_STEPS,
            llm_connect_timeout_secs: DEFAULT_LLM_CONNECT_TIMEOUT_SECS,
            llm_read_timeout_secs: DEFAULT_LLM_READ_TIMEOUT_SECS,
            llm_max_retries: DEFAULT_LLM_MAX_RETRIES,
            llm_retry_base_delay_ms: DEFAULT_LLM_RETRY_BASE_DELAY_MS,
            max_reactive_compact_attempts: DEFAULT_MAX_REACTIVE_COMPACT_ATTEMPTS,
            max_output_continuation_attempts: DEFAULT_MAX_OUTPUT_CONTINUATION_ATTEMPTS,
            max_continuations: DEFAULT_MAX_CONTINUATIONS,
            summary_reserve_tokens: DEFAULT_SUMMARY_RESERVE_TOKENS,
            max_tracked_files: DEFAULT_MAX_TRACKED_FILES,
            max_recovered_files: DEFAULT_MAX_RECOVERED_FILES,
            recovery_token_budget: DEFAULT_RECOVERY_TOKEN_BUDGET,
            tool_result_inline_limit: DEFAULT_TOOL_RESULT_INLINE_LIMIT,
            tool_result_preview_limit: DEFAULT_TOOL_RESULT_PREVIEW_LIMIT,
            max_image_size: DEFAULT_MAX_IMAGE_SIZE,
            max_grep_lines: DEFAULT_MAX_GREP_LINES,
            session_broadcast_capacity: DEFAULT_SESSION_BROADCAST_CAPACITY,
            session_recent_record_limit: DEFAULT_SESSION_RECENT_RECORD_LIMIT,
            max_concurrent_branch_depth: DEFAULT_MAX_CONCURRENT_BRANCH_DEPTH,
            aggregate_result_bytes_budget: DEFAULT_AGGREGATE_RESULT_BYTES_BUDGET,
            micro_compact_gap_threshold_secs: DEFAULT_MICRO_COMPACT_GAP_THRESHOLD_SECS,
            micro_compact_keep_recent_results: DEFAULT_MICRO_COMPACT_KEEP_RECENT_RESULTS,
            api_session_ttl_hours: DEFAULT_API_SESSION_TTL_HOURS,
        }
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
pub struct ModelSelection {
    pub profile_name: String,
    pub model: String,
    pub provider_kind: String,
}

impl ModelSelection {
    pub fn new(
        profile_name: impl Into<String>,
        model: impl Into<String>,
        provider_kind: impl Into<String>,
    ) -> Self {
        Self {
            profile_name: profile_name.into(),
            model: model.into(),
            provider_kind: provider_kind.into(),
        }
    }
}

pub type CurrentModelSelection = ModelSelection;

/// 扁平化的模型选项。
pub type ModelOption = ModelSelection;

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
            .field("max_steps", &self.max_steps)
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
            .field("max_continuations", &self.max_continuations)
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

/// 从进程环境变量/默认值获取最大安全工具并发数。
pub fn max_tool_concurrency() -> usize {
    std::env::var(ASTRCODE_MAX_TOOL_CONCURRENCY_ENV)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_MAX_TOOL_CONCURRENCY)
        .max(1)
}

pub fn resolve_agent_config(agent: Option<&AgentConfig>) -> ResolvedAgentConfig {
    let defaults = ResolvedAgentConfig::default();
    ResolvedAgentConfig {
        max_subrun_depth: agent
            .and_then(|value| value.max_subrun_depth)
            .unwrap_or(defaults.max_subrun_depth)
            .max(1),
        max_spawn_per_turn: agent
            .and_then(|value| value.max_spawn_per_turn)
            .unwrap_or(defaults.max_spawn_per_turn)
            .max(1),
        max_concurrent: agent
            .and_then(|value| value.max_concurrent)
            .unwrap_or(defaults.max_concurrent)
            .max(1),
        finalized_retain_limit: agent
            .and_then(|value| value.finalized_retain_limit)
            .unwrap_or(defaults.finalized_retain_limit),
        inbox_capacity: agent
            .and_then(|value| value.inbox_capacity)
            .unwrap_or(defaults.inbox_capacity)
            .max(1),
        parent_delivery_capacity: agent
            .and_then(|value| value.parent_delivery_capacity)
            .unwrap_or(defaults.parent_delivery_capacity)
            .max(1),
    }
}

pub fn resolve_runtime_config(runtime: &RuntimeConfig) -> ResolvedRuntimeConfig {
    let defaults = ResolvedRuntimeConfig::default();
    ResolvedRuntimeConfig {
        max_tool_concurrency: runtime
            .max_tool_concurrency
            .unwrap_or(defaults.max_tool_concurrency)
            .max(1),
        auto_compact_enabled: runtime
            .auto_compact_enabled
            .unwrap_or(defaults.auto_compact_enabled),
        compact_threshold_percent: runtime
            .compact_threshold_percent
            .unwrap_or(defaults.compact_threshold_percent)
            .clamp(1, 100),
        tool_result_max_bytes: runtime
            .tool_result_max_bytes
            .unwrap_or(defaults.tool_result_max_bytes)
            .max(1),
        compact_keep_recent_turns: runtime
            .compact_keep_recent_turns
            .unwrap_or(defaults.compact_keep_recent_turns)
            .max(1),
        agent: resolve_agent_config(runtime.agent.as_ref()),
        max_consecutive_failures: runtime
            .max_consecutive_failures
            .unwrap_or(defaults.max_consecutive_failures)
            .max(1),
        recovery_truncate_bytes: runtime
            .recovery_truncate_bytes
            .unwrap_or(defaults.recovery_truncate_bytes)
            .max(1024),
        max_steps: runtime.max_steps.unwrap_or(defaults.max_steps).max(1),
        llm_connect_timeout_secs: runtime
            .llm_connect_timeout_secs
            .unwrap_or(defaults.llm_connect_timeout_secs)
            .max(1),
        llm_read_timeout_secs: runtime
            .llm_read_timeout_secs
            .unwrap_or(defaults.llm_read_timeout_secs)
            .max(1),
        llm_max_retries: runtime.llm_max_retries.unwrap_or(defaults.llm_max_retries),
        llm_retry_base_delay_ms: runtime
            .llm_retry_base_delay_ms
            .unwrap_or(defaults.llm_retry_base_delay_ms)
            .max(1),
        max_reactive_compact_attempts: runtime
            .max_reactive_compact_attempts
            .unwrap_or(defaults.max_reactive_compact_attempts)
            .max(1),
        max_output_continuation_attempts: runtime
            .max_output_continuation_attempts
            .unwrap_or(defaults.max_output_continuation_attempts)
            .max(1),
        max_continuations: runtime
            .max_continuations
            .unwrap_or(defaults.max_continuations)
            .max(1),
        summary_reserve_tokens: runtime
            .summary_reserve_tokens
            .unwrap_or(defaults.summary_reserve_tokens),
        max_tracked_files: runtime
            .max_tracked_files
            .unwrap_or(defaults.max_tracked_files)
            .max(1),
        max_recovered_files: runtime
            .max_recovered_files
            .unwrap_or(defaults.max_recovered_files),
        recovery_token_budget: runtime
            .recovery_token_budget
            .unwrap_or(defaults.recovery_token_budget),
        tool_result_inline_limit: runtime
            .tool_result_inline_limit
            .unwrap_or(defaults.tool_result_inline_limit)
            .max(1024),
        tool_result_preview_limit: runtime
            .tool_result_preview_limit
            .unwrap_or(defaults.tool_result_preview_limit)
            .max(256),
        max_image_size: runtime
            .max_image_size
            .unwrap_or(defaults.max_image_size)
            .max(1024 * 1024),
        max_grep_lines: runtime
            .max_grep_lines
            .unwrap_or(defaults.max_grep_lines)
            .max(10),
        session_broadcast_capacity: runtime
            .session_broadcast_capacity
            .unwrap_or(defaults.session_broadcast_capacity)
            .max(64),
        session_recent_record_limit: runtime
            .session_recent_record_limit
            .unwrap_or(defaults.session_recent_record_limit)
            .max(128),
        max_concurrent_branch_depth: runtime
            .max_concurrent_branch_depth
            .unwrap_or(defaults.max_concurrent_branch_depth)
            .max(1),
        aggregate_result_bytes_budget: runtime
            .aggregate_result_bytes_budget
            .unwrap_or(defaults.aggregate_result_bytes_budget)
            .max(1024),
        micro_compact_gap_threshold_secs: runtime
            .micro_compact_gap_threshold_secs
            .unwrap_or(defaults.micro_compact_gap_threshold_secs)
            .max(60),
        micro_compact_keep_recent_results: runtime
            .micro_compact_keep_recent_results
            .unwrap_or(defaults.micro_compact_keep_recent_results),
        api_session_ttl_hours: runtime
            .api_session_ttl_hours
            .unwrap_or(defaults.api_session_ttl_hours)
            .max(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_runtime_config_uses_defaults_for_empty_runtime() {
        let resolved = resolve_runtime_config(&RuntimeConfig::default());

        assert_eq!(resolved.max_tool_concurrency, max_tool_concurrency());
        assert_eq!(resolved.max_steps, DEFAULT_MAX_STEPS);
        assert_eq!(resolved.agent.max_subrun_depth, DEFAULT_MAX_SUBRUN_DEPTH);
        assert_eq!(
            resolved.agent.max_spawn_per_turn,
            DEFAULT_MAX_SPAWN_PER_TURN
        );
        assert_eq!(
            resolved.tool_result_inline_limit,
            DEFAULT_TOOL_RESULT_INLINE_LIMIT
        );
    }

    #[test]
    fn resolved_runtime_config_honors_runtime_overrides() {
        let resolved = resolve_runtime_config(&RuntimeConfig {
            max_tool_concurrency: Some(16),
            max_steps: Some(12),
            llm_read_timeout_secs: Some(120),
            agent: Some(AgentConfig {
                max_subrun_depth: Some(5),
                max_spawn_per_turn: Some(2),
                ..AgentConfig::default()
            }),
            ..RuntimeConfig::default()
        });

        assert_eq!(resolved.max_tool_concurrency, 16);
        assert_eq!(resolved.max_steps, 12);
        assert_eq!(resolved.llm_read_timeout_secs, 120);
        assert_eq!(resolved.agent.max_subrun_depth, 5);
        assert_eq!(resolved.agent.max_spawn_per_turn, 2);
    }
}
