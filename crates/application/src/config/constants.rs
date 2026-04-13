//! 运行时配置常量与解析函数。
//!
//! 集中管理配置相关的所有常量、默认值和解析函数，是 runtime 调参的唯一真实来源。
//!
//! # 设计原则
//!
//! - 类型定义在 `core::config`，默认值和解析逻辑在此模块
//! - 所有 `resolve_*` 函数从 `RuntimeConfig` / `AgentConfig` 的 Option 字段解析出有效值
//! - 解析优先级：用户配置 > 环境变量（仅限个别字段）> 内置默认值
//!
//! # URL 标准化
//!
//! `resolve_*_api_url` 系列函数处理 Provider 地址的多种写法（根地址、版本根、完整集合地址），
//! 确保运行时始终拿到可直接发请求的完整 URL。

use astrcode_core::{AgentConfig, RuntimeConfig};

// ============================================================
// Provider 标识符
// ============================================================

/// OpenAI 兼容协议 Provider 标识符。
///
/// 用于 `Profile.provider_kind` 字段，表示该 Provider 使用 OpenAI Chat Completions API 格式。
/// Deepseek 等兼容 OpenAI 接口的服务都使用此标识符。
pub const PROVIDER_KIND_OPENAI: &str = "openai-compatible";

/// Anthropic Provider 标识符。
///
/// 用于 `Profile.provider_kind` 字段，表示该 Provider 使用 Anthropic Messages API 格式。
/// 与 OpenAI 兼容协议不同，Anthropic 使用专用请求头；同时允许通过 `baseUrl`
/// 覆盖默认官方地址，接入自定义 Anthropic 兼容网关。
pub const PROVIDER_KIND_ANTHROPIC: &str = "anthropic";

// ============================================================
// 值前缀
// ============================================================

/// 环境变量引用前缀。
///
/// 配置值以 `env:` 开头时，表示该值必须从指定名称的环境变量中读取。
pub const ENV_REFERENCE_PREFIX: &str = "env:";

/// 字面值前缀。
///
/// 配置值以 `literal:` 开头时，表示该值应直接作为字面值使用，跳过环境变量解析。
pub const LITERAL_VALUE_PREFIX: &str = "literal:";

// ============================================================
// 环境变量分组
// ============================================================

pub use astrcode_core::env::{
    ANTHROPIC_API_KEY_ENV, ASTRCODE_HOME_DIR_ENV, ASTRCODE_MAX_TOOL_CONCURRENCY_ENV,
    ASTRCODE_PLUGIN_DIRS_ENV, ASTRCODE_TEST_HOME_ENV, ASTRCODE_TOOL_INLINE_LIMIT_PREFIX,
    ASTRCODE_TOOL_RESULT_INLINE_LIMIT_ENV, DEEPSEEK_API_KEY_ENV, TAURI_ENV_TARGET_TRIPLE_ENV,
};

/// 影响 Astrcode 本地存储路径的环境变量。
pub const HOME_ENV_VARS: &[&str] = &[ASTRCODE_HOME_DIR_ENV, ASTRCODE_TEST_HOME_ENV];

/// 影响运行时插件发现的环境变量。
pub const PLUGIN_ENV_VARS: &[&str] = &[ASTRCODE_PLUGIN_DIRS_ENV];

/// 内置 Provider 默认配置使用的 API key 环境变量。
pub const PROVIDER_API_KEY_ENV_VARS: &[&str] = &[DEEPSEEK_API_KEY_ENV, ANTHROPIC_API_KEY_ENV];

/// Tauri sidecar 构建管道所需的环境变量。
pub const BUILD_ENV_VARS: &[&str] = &[TAURI_ENV_TARGET_TRIPLE_ENV];

/// 调优运行时执行行为的环境变量。
pub const RUNTIME_ENV_VARS: &[&str] = &[
    ASTRCODE_MAX_TOOL_CONCURRENCY_ENV,
    ASTRCODE_TOOL_RESULT_INLINE_LIMIT_ENV,
];

/// 所有 Astrcode 定义的环境变量。
///
/// 新增环境变量时必须同步更新此数组和对应的分组数组。
pub const ALL_ASTRCODE_ENV_VARS: &[&str] = &[
    ASTRCODE_HOME_DIR_ENV,
    ASTRCODE_TEST_HOME_ENV,
    ASTRCODE_PLUGIN_DIRS_ENV,
    DEEPSEEK_API_KEY_ENV,
    ANTHROPIC_API_KEY_ENV,
    TAURI_ENV_TARGET_TRIPLE_ENV,
    ASTRCODE_MAX_TOOL_CONCURRENCY_ENV,
    ASTRCODE_TOOL_RESULT_INLINE_LIMIT_ENV,
];

// ============================================================
// API URL 常量
// ============================================================

/// Anthropic Messages API endpoint URL。
pub const ANTHROPIC_MESSAGES_API_URL: &str = "https://api.anthropic.com/v1/messages";

/// Anthropic Models API endpoint URL。
///
/// 用于按模型 ID 拉取权威的上下文窗口和最大输出 token 元数据。
pub const ANTHROPIC_MODELS_API_URL: &str = "https://api.anthropic.com/v1/models";

/// Anthropic API version。
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// OpenAI-compatible 模型的保守默认上下文窗口。
///
/// 用于默认生成的 OpenAI-compatible profile，避免首次创建配置文件时出现空 limits。
pub const DEFAULT_OPENAI_CONTEXT_LIMIT: usize = 128_000;

// ============================================================
// 配置 schema 版本
// ============================================================

/// 配置 schema 的当前版本号。
///
/// 加载配置时空白的 version 字段会被迁移到此值，不支持的版本号会导致加载失败。
pub const CURRENT_CONFIG_VERSION: &str = "1";

// ============================================================
// 工具并发与压缩默认值
// ============================================================

/// 默认最大安全工具并发数。
///
/// 当 `runtime.maxToolConcurrency` 和环境变量都未设置时使用此值。
pub const DEFAULT_MAX_TOOL_CONCURRENCY: usize = 10;

/// 默认自动上下文压缩开关。
pub const DEFAULT_AUTO_COMPACT_ENABLED: bool = true;

/// 默认上下文压缩触发阈值（百分比）。
pub const DEFAULT_COMPACT_THRESHOLD_PERCENT: u8 = 90;

/// 默认单个工具结果的请求字节预算。
pub const DEFAULT_TOOL_RESULT_MAX_BYTES: usize = 100_000;

/// 默认上下文压缩时保留的最近用户回合数。
pub const DEFAULT_COMPACT_KEEP_RECENT_TURNS: u8 = 4;

// ============================================================
// LLM 客户端配置默认值
// ============================================================

/// 默认 LLM 连接超时（秒）。
pub const DEFAULT_LLM_CONNECT_TIMEOUT_SECS: u64 = 10;

/// 默认 LLM 读取超时（秒）。
pub const DEFAULT_LLM_READ_TIMEOUT_SECS: u64 = 90;

/// 默认 LLM 请求最大重试次数。
pub const DEFAULT_LLM_MAX_RETRIES: u32 = 2;

/// 默认 LLM 重试基础延迟（毫秒）。
pub const DEFAULT_LLM_RETRY_BASE_DELAY_MS: u64 = 250;

// ============================================================
// Agent 循环配置默认值
// ============================================================

/// 默认响应式压缩最大重试次数。
pub const DEFAULT_MAX_REACTIVE_COMPACT_ATTEMPTS: u8 = 3;

/// 默认输出续调最大尝试次数。
pub const DEFAULT_MAX_OUTPUT_CONTINUATION_ATTEMPTS: u8 = 3;

/// 默认摘要保留 token 数。
pub const DEFAULT_SUMMARY_RESERVE_TOKENS: usize = 20_000;

/// 默认最大跟踪文件数。
pub const DEFAULT_MAX_TRACKED_FILES: usize = 10;

/// 默认压缩恢复最大文件数。
pub const DEFAULT_MAX_RECOVERED_FILES: usize = 5;

/// 默认恢复 token 预算。
pub const DEFAULT_RECOVERY_TOKEN_BUDGET: usize = 50_000;

// ============================================================
// 工具限制配置默认值
// ============================================================

/// 默认工具结果内联阈值（字节）。
pub const DEFAULT_TOOL_RESULT_INLINE_LIMIT: usize = 32 * 1024;

/// 默认工具结果预览限制（字节）。
pub const DEFAULT_TOOL_RESULT_PREVIEW_LIMIT: usize = 2 * 1024;

/// 默认最大图片文件大小（字节）。
pub const DEFAULT_MAX_IMAGE_SIZE: usize = 20 * 1024 * 1024;

/// 默认 grep 最大显示行数。
pub const DEFAULT_MAX_GREP_LINES: usize = 500;

// ============================================================
// 会话配置默认值
// ============================================================

/// 默认会话广播容量。
pub const DEFAULT_SESSION_BROADCAST_CAPACITY: usize = 2048;

/// 默认会话最近记录限制。
pub const DEFAULT_SESSION_RECENT_RECORD_LIMIT: usize = 4096;

/// 默认最大并发分支深度。
pub const DEFAULT_MAX_CONCURRENT_BRANCH_DEPTH: usize = 3;

// ============================================================
// 服务器认证配置默认值
// ============================================================

/// 默认 API 会话有效期（小时）。
pub const DEFAULT_API_SESSION_TTL_HOURS: i64 = 8;

// ============================================================
// 上下文管线持久化与微压缩默认值
// ============================================================

/// 默认聚合预算（字节）。
pub const DEFAULT_AGGREGATE_RESULT_BYTES_BUDGET: usize = 200_000;

/// 默认微压缩空闲阈值（秒）。
pub const DEFAULT_MICRO_COMPACT_GAP_THRESHOLD_SECS: u64 = 3600;

/// 默认微压缩保留最近工具结果数。
pub const DEFAULT_MICRO_COMPACT_KEEP_RECENT_RESULTS: usize = 10;

// ============================================================
// 多 Agent 控制默认值
// ============================================================

/// 默认 Agent 嵌套深度上限。
pub const DEFAULT_MAX_AGENT_DEPTH: usize = 3;

/// 默认受控子会话最大深度。
///
/// 默认值刻意更保守，防止子 agent 再起子 agent 过早把事件流和 UI 复杂度推高。
pub const DEFAULT_MAX_SUBRUN_DEPTH: usize = 1;

/// 默认并发子 Agent 数上限。
pub const DEFAULT_MAX_CONCURRENT_AGENTS: usize = 8;

/// 默认终态 Agent 保留数。
pub const DEFAULT_FINALIZED_AGENT_RETAIN_LIMIT: usize = 256;

/// 默认单个 agent 收件箱容量上限。
pub const DEFAULT_INBOX_CAPACITY: usize = 1024;

/// 默认单个会话的父级交付队列容量上限。
pub const DEFAULT_PARENT_DELIVERY_CAPACITY: usize = 1024;

// ============================================================
// 压缩恢复与熔断配置默认值
// ============================================================

/// 默认熔断阈值：连续压缩失败次数达到此值后暂停自动压缩。
pub const DEFAULT_MAX_CONSECUTIVE_FAILURES: usize = 3;

/// 默认压缩恢复文件内容的截断字节数。
pub const DEFAULT_RECOVERY_TRUNCATE_BYTES: usize = 30_000;

// ============================================================
// URL 标准化辅助函数
// ============================================================

/// 判断路径段是否像显式 API 版本号。
///
/// OpenAI 兼容网关并不都使用 `/v1`，一些第三方会暴露 `/v4`、`/v1beta` 等版本根。
/// 只要段名以 `v` 开头且紧跟数字，就认为它是一个显式版本段。
fn looks_like_api_version_segment(segment: &str) -> bool {
    let mut chars = segment.chars();
    matches!(chars.next(), Some('v' | 'V'))
        && matches!(chars.next(), Some(ch) if ch.is_ascii_digit())
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
}

/// 将已包含显式版本段的 OpenAI 兼容地址标准化到目标集合路径。
fn normalize_openai_versioned_base_url(trimmed: &str, collection_suffix: &str) -> Option<String> {
    let segments = trimmed.split('/').collect::<Vec<_>>();
    let version_index = segments
        .iter()
        .rposition(|segment| looks_like_api_version_segment(segment))?;
    let prefix = segments[..=version_index].join("/");
    Some(format!("{prefix}/{collection_suffix}"))
}

fn split_url_query(url: &str) -> (&str, Option<&str>) {
    match url.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (url, None),
    }
}

fn join_url_query(path: String, query: Option<&str>) -> String {
    match query {
        Some(query) if !query.is_empty() => format!("{path}?{query}"),
        _ => path,
    }
}

fn resolve_anthropic_api_collection_url(
    base_url: &str,
    collection: &'static str,
    default_url: &'static str,
) -> String {
    let (path, query) = split_url_query(base_url.trim());
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return default_url.to_string();
    }

    let this_collection_suffix = format!("/{collection}");
    if trimmed.ends_with(&this_collection_suffix) {
        return join_url_query(trimmed.to_string(), query);
    }

    // 兄弟集合互换：messages ↔ models
    let sibling_collection = if collection == "messages" {
        "models"
    } else {
        "messages"
    };
    let sibling_suffix = format!("/{sibling_collection}");
    if trimmed.ends_with(&sibling_suffix) {
        return join_url_query(
            format!(
                "{}/{}",
                trimmed.trim_end_matches(&sibling_suffix),
                collection
            ),
            query,
        );
    }

    if trimmed.ends_with("/v1") {
        return join_url_query(format!("{trimmed}/{collection}"), query);
    }

    if let Some((prefix, _tail)) = trimmed.rsplit_once("/v1/") {
        // 只要已经落在 `/v1/<something>` 形态，就把尾集合标准化成目标集合
        return join_url_query(format!("{prefix}/v1/{collection}"), query);
    }

    join_url_query(format!("{trimmed}/v1/{collection}"), query)
}

/// 解析 Anthropic Messages API 地址。
///
/// 兼容三种写法：
/// - 空字符串：回退到官方默认地址
/// - API 根地址：如 `https://gateway.example.com/anthropic`
/// - 完整集合地址：如 `https://gateway.example.com/anthropic/v1/messages`
pub fn resolve_anthropic_messages_api_url(base_url: &str) -> String {
    resolve_anthropic_api_collection_url(base_url, "messages", ANTHROPIC_MESSAGES_API_URL)
}

/// 解析 Anthropic Models API 地址。
///
/// 与 [`resolve_anthropic_messages_api_url`] 使用同一规则，确保消息和模型探测落在同一条链路。
pub fn resolve_anthropic_models_api_url(base_url: &str) -> String {
    resolve_anthropic_api_collection_url(base_url, "models", ANTHROPIC_MODELS_API_URL)
}

/// 解析 OpenAI Chat Completions API 地址。
///
/// 兼容四种写法：
/// - API 根地址：如 `https://api.deepseek.com`
/// - 版本根地址：如 `https://api.openai.com/v1`
/// - 完整集合地址：如 `https://api.openai.com/v1/chat/completions`
/// - 第三方版本根地址：如 `https://api.z.ai/api/coding/paas/v4`
pub fn resolve_openai_chat_completions_api_url(base_url: &str) -> String {
    let (path, query) = split_url_query(base_url.trim());
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return String::new();
    }

    let normalized = if trimmed.ends_with("/chat/completions") {
        trimmed.to_string()
    } else if trimmed.ends_with("/chat") {
        format!("{trimmed}/completions")
    } else if let Some(versioned_url) =
        normalize_openai_versioned_base_url(trimmed, "chat/completions")
    {
        versioned_url
    } else {
        format!("{trimmed}/v1/chat/completions")
    };

    join_url_query(normalized, query)
}

// ============================================================
// 从环境变量/默认值获取配置（无 RuntimeConfig 依赖的场景）
// ============================================================

/// 从进程环境变量/默认值获取最大安全工具并发数。
///
/// 此函数仅读取环境变量，适用于尚未加载 `config.json` 的底层调用方。
pub fn max_tool_concurrency() -> usize {
    std::env::var(ASTRCODE_MAX_TOOL_CONCURRENCY_ENV)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_MAX_TOOL_CONCURRENCY)
        .max(1)
}

// ============================================================
// RuntimeConfig 解析函数
// ============================================================

/// 解析已加载运行时配置的有效工具并发上限。
///
/// 解析优先级：
/// 1. `config.runtime.maxToolConcurrency`（用户配置最高优先级）
/// 2. `ASTRCODE_MAX_TOOL_CONCURRENCY` 环境变量
/// 3. 内置默认值
pub fn resolve_max_tool_concurrency(runtime: &RuntimeConfig) -> usize {
    runtime
        .max_tool_concurrency
        .unwrap_or_else(max_tool_concurrency)
        .max(1)
}

/// 解析自动压缩开关。
pub fn resolve_auto_compact_enabled(runtime: &RuntimeConfig) -> bool {
    runtime
        .auto_compact_enabled
        .unwrap_or(DEFAULT_AUTO_COMPACT_ENABLED)
}

/// 解析压缩触发阈值百分比。
pub fn resolve_compact_threshold_percent(runtime: &RuntimeConfig) -> u8 {
    runtime
        .compact_threshold_percent
        .unwrap_or(DEFAULT_COMPACT_THRESHOLD_PERCENT)
        .clamp(1, 100)
}

/// 解析单个工具结果的字节预算。
pub fn resolve_tool_result_max_bytes(runtime: &RuntimeConfig) -> usize {
    runtime
        .tool_result_max_bytes
        .unwrap_or(DEFAULT_TOOL_RESULT_MAX_BYTES)
        .max(1)
}

/// 解析压缩时保留的最近回合数。
pub fn resolve_compact_keep_recent_turns(runtime: &RuntimeConfig) -> u8 {
    runtime
        .compact_keep_recent_turns
        .unwrap_or(DEFAULT_COMPACT_KEEP_RECENT_TURNS)
        .max(1)
}

// ============================================================
// LLM 客户端配置解析
// ============================================================

/// 解析 LLM 连接超时（秒）。
pub fn resolve_llm_connect_timeout_secs(runtime: &RuntimeConfig) -> u64 {
    runtime
        .llm_connect_timeout_secs
        .unwrap_or(DEFAULT_LLM_CONNECT_TIMEOUT_SECS)
        .max(1)
}

/// 解析 LLM 读取超时（秒）。
pub fn resolve_llm_read_timeout_secs(runtime: &RuntimeConfig) -> u64 {
    runtime
        .llm_read_timeout_secs
        .unwrap_or(DEFAULT_LLM_READ_TIMEOUT_SECS)
        .max(1)
}

/// 解析 LLM 最大重试次数。
pub fn resolve_llm_max_retries(runtime: &RuntimeConfig) -> u32 {
    runtime.llm_max_retries.unwrap_or(DEFAULT_LLM_MAX_RETRIES)
}

// ============================================================
// Agent 循环配置解析
// ============================================================

/// 解析响应式压缩最大重试次数。
pub fn resolve_max_reactive_compact_attempts(runtime: &RuntimeConfig) -> u8 {
    runtime
        .max_reactive_compact_attempts
        .unwrap_or(DEFAULT_MAX_REACTIVE_COMPACT_ATTEMPTS)
        .max(1)
}

/// 解析输出续调最大尝试次数。
pub fn resolve_max_output_continuation_attempts(runtime: &RuntimeConfig) -> u8 {
    runtime
        .max_output_continuation_attempts
        .unwrap_or(DEFAULT_MAX_OUTPUT_CONTINUATION_ATTEMPTS)
        .max(1)
}

/// 解析摘要保留 token 数。
pub fn resolve_summary_reserve_tokens(runtime: &RuntimeConfig) -> usize {
    runtime
        .summary_reserve_tokens
        .unwrap_or(DEFAULT_SUMMARY_RESERVE_TOKENS)
}

/// 解析最大跟踪文件数。
pub fn resolve_max_tracked_files(runtime: &RuntimeConfig) -> usize {
    runtime
        .max_tracked_files
        .unwrap_or(DEFAULT_MAX_TRACKED_FILES)
        .max(1)
}

/// 解析压缩恢复最大文件数。
pub fn resolve_max_recovered_files(runtime: &RuntimeConfig) -> usize {
    runtime
        .max_recovered_files
        .unwrap_or(DEFAULT_MAX_RECOVERED_FILES)
}

/// 解析恢复 token 预算。
pub fn resolve_recovery_token_budget(runtime: &RuntimeConfig) -> usize {
    runtime
        .recovery_token_budget
        .unwrap_or(DEFAULT_RECOVERY_TOKEN_BUDGET)
}

// ============================================================
// 工具限制配置解析
// ============================================================

/// 解析工具结果内联阈值（字节）。
pub fn resolve_tool_result_inline_limit(runtime: &RuntimeConfig) -> usize {
    runtime
        .tool_result_inline_limit
        .unwrap_or(DEFAULT_TOOL_RESULT_INLINE_LIMIT)
        .max(1024)
}

/// 解析工具结果预览限制（字节）。
pub fn resolve_tool_result_preview_limit(runtime: &RuntimeConfig) -> usize {
    runtime
        .tool_result_preview_limit
        .unwrap_or(DEFAULT_TOOL_RESULT_PREVIEW_LIMIT)
        .max(256)
}

/// 解析最大图片文件大小（字节）。
pub fn resolve_max_image_size(runtime: &RuntimeConfig) -> usize {
    runtime
        .max_image_size
        .unwrap_or(DEFAULT_MAX_IMAGE_SIZE)
        .max(1024 * 1024)
}

/// 解析 grep 最大显示行数。
pub fn resolve_max_grep_lines(runtime: &RuntimeConfig) -> usize {
    runtime
        .max_grep_lines
        .unwrap_or(DEFAULT_MAX_GREP_LINES)
        .max(10)
}

// ============================================================
// 会话配置解析
// ============================================================

/// 解析会话广播容量。
pub fn resolve_session_broadcast_capacity(runtime: &RuntimeConfig) -> usize {
    runtime
        .session_broadcast_capacity
        .unwrap_or(DEFAULT_SESSION_BROADCAST_CAPACITY)
        .max(64)
}

/// 解析会话最近记录限制。
pub fn resolve_session_recent_record_limit(runtime: &RuntimeConfig) -> usize {
    runtime
        .session_recent_record_limit
        .unwrap_or(DEFAULT_SESSION_RECENT_RECORD_LIMIT)
        .max(128)
}

/// 解析最大并发分支深度。
pub fn resolve_max_concurrent_branch_depth(runtime: &RuntimeConfig) -> usize {
    runtime
        .max_concurrent_branch_depth
        .unwrap_or(DEFAULT_MAX_CONCURRENT_BRANCH_DEPTH)
        .max(1)
}

// ============================================================
// 服务器认证配置解析
// ============================================================

/// 解析 API 会话有效期（小时）。
pub fn resolve_api_session_ttl_hours(runtime: &RuntimeConfig) -> i64 {
    runtime
        .api_session_ttl_hours
        .unwrap_or(DEFAULT_API_SESSION_TTL_HOURS)
        .max(1)
}

// ============================================================
// 上下文管线持久化与微压缩配置解析
// ============================================================

/// 解析聚合预算（字节）。
pub fn resolve_aggregate_result_bytes_budget(runtime: &RuntimeConfig) -> usize {
    runtime
        .aggregate_result_bytes_budget
        .unwrap_or(DEFAULT_AGGREGATE_RESULT_BYTES_BUDGET)
        .max(1024)
}

/// 解析微压缩空闲阈值（秒）。
pub fn resolve_micro_compact_gap_threshold_secs(runtime: &RuntimeConfig) -> u64 {
    runtime
        .micro_compact_gap_threshold_secs
        .unwrap_or(DEFAULT_MICRO_COMPACT_GAP_THRESHOLD_SECS)
        .max(60)
}

/// 解析微压缩保留最近工具结果数。
pub fn resolve_micro_compact_keep_recent_results(runtime: &RuntimeConfig) -> usize {
    runtime
        .micro_compact_keep_recent_results
        .unwrap_or(DEFAULT_MICRO_COMPACT_KEEP_RECENT_RESULTS)
}

// ============================================================
// 多 Agent 控制配置解析
// ============================================================

/// 解析受控子会话最大深度。
pub fn resolve_agent_max_subrun_depth(agent: Option<&AgentConfig>) -> usize {
    agent
        .and_then(|a| a.max_subrun_depth)
        .unwrap_or(DEFAULT_MAX_SUBRUN_DEPTH)
        .max(1)
}

/// 解析并发子 Agent 数上限。
pub fn resolve_agent_max_concurrent(agent: Option<&AgentConfig>) -> usize {
    agent
        .and_then(|a| a.max_concurrent)
        .unwrap_or(DEFAULT_MAX_CONCURRENT_AGENTS)
        .max(1)
}

/// 解析终态 Agent 保留数。
pub fn resolve_agent_finalized_retain_limit(agent: Option<&AgentConfig>) -> usize {
    agent
        .and_then(|a| a.finalized_retain_limit)
        .unwrap_or(DEFAULT_FINALIZED_AGENT_RETAIN_LIMIT)
}

/// 解析单个 agent 收件箱容量上限。
pub fn resolve_agent_inbox_capacity(agent: Option<&AgentConfig>) -> usize {
    agent
        .and_then(|a| a.inbox_capacity)
        .unwrap_or(DEFAULT_INBOX_CAPACITY)
        .max(1)
}

/// 解析单个会话的父级交付队列容量上限。
pub fn resolve_agent_parent_delivery_capacity(agent: Option<&AgentConfig>) -> usize {
    agent
        .and_then(|a| a.parent_delivery_capacity)
        .unwrap_or(DEFAULT_PARENT_DELIVERY_CAPACITY)
        .max(1)
}

// ============================================================
// 压缩恢复与熔断配置解析
// ============================================================

/// 解析熔断阈值（连续压缩失败次数）。
pub fn resolve_max_consecutive_failures(runtime: &RuntimeConfig) -> usize {
    runtime
        .max_consecutive_failures
        .unwrap_or(DEFAULT_MAX_CONSECUTIVE_FAILURES)
        .max(1)
}

/// 解析压缩恢复文件内容的截断字节数。
pub fn resolve_recovery_truncate_bytes(runtime: &RuntimeConfig) -> usize {
    runtime
        .recovery_truncate_bytes
        .unwrap_or(DEFAULT_RECOVERY_TRUNCATE_BYTES)
        .max(1024)
}

#[cfg(test)]
mod tests {
    use astrcode_core::RuntimeConfig;

    use super::*;

    #[test]
    fn anthropic_url_helpers_fall_back_to_official_defaults() {
        assert_eq!(
            resolve_anthropic_messages_api_url(""),
            ANTHROPIC_MESSAGES_API_URL
        );
        assert_eq!(
            resolve_anthropic_models_api_url(""),
            ANTHROPIC_MODELS_API_URL
        );
    }

    #[test]
    fn anthropic_url_helpers_expand_root_style_base_url() {
        let base_url = "https://gateway.example.com/anthropic";
        assert_eq!(
            resolve_anthropic_messages_api_url(base_url),
            "https://gateway.example.com/anthropic/v1/messages"
        );
        assert_eq!(
            resolve_anthropic_models_api_url(base_url),
            "https://gateway.example.com/anthropic/v1/models"
        );
    }

    #[test]
    fn anthropic_url_helpers_accept_full_collection_urls() {
        let messages_url = "https://gateway.example.com/anthropic/v1/messages";
        let models_url = "https://gateway.example.com/anthropic/v1/models";

        assert_eq!(
            resolve_anthropic_messages_api_url(messages_url),
            messages_url
        );
        assert_eq!(resolve_anthropic_models_api_url(messages_url), models_url);
        assert_eq!(resolve_anthropic_models_api_url(models_url), models_url);
        assert_eq!(resolve_anthropic_messages_api_url(models_url), messages_url);
    }

    #[test]
    fn anthropic_url_helpers_trim_whitespace_and_trailing_slashes() {
        let base_url = "  https://gateway.example.com/anthropic/v1/  ";
        assert_eq!(
            resolve_anthropic_messages_api_url(base_url),
            "https://gateway.example.com/anthropic/v1/messages"
        );
        assert_eq!(
            resolve_anthropic_models_api_url(base_url),
            "https://gateway.example.com/anthropic/v1/models"
        );
    }

    #[test]
    fn anthropic_url_helpers_expand_v1_base_without_collection() {
        let base_url = "https://gateway.example.com/anthropic/v1";
        assert_eq!(
            resolve_anthropic_messages_api_url(base_url),
            "https://gateway.example.com/anthropic/v1/messages"
        );
        assert_eq!(
            resolve_anthropic_models_api_url(base_url),
            "https://gateway.example.com/anthropic/v1/models"
        );
    }

    #[test]
    fn anthropic_url_helpers_replace_nonstandard_v1_tail() {
        let base_url = "https://gateway.example.com/anthropic/v1/messeges?foo=bar";
        assert_eq!(
            resolve_anthropic_messages_api_url(base_url),
            "https://gateway.example.com/anthropic/v1/messages?foo=bar"
        );
        assert_eq!(
            resolve_anthropic_models_api_url(base_url),
            "https://gateway.example.com/anthropic/v1/models?foo=bar"
        );
    }

    #[test]
    fn openai_url_helper_expands_root_style_base_url() {
        assert_eq!(
            resolve_openai_chat_completions_api_url("https://api.deepseek.com"),
            "https://api.deepseek.com/v1/chat/completions"
        );
        assert_eq!(
            resolve_openai_chat_completions_api_url("https://api.openai.com/v1"),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn openai_url_helper_preserves_non_v1_version_root_for_third_party_gateways() {
        assert_eq!(
            resolve_openai_chat_completions_api_url("https://api.z.ai/api/coding/paas/v4"),
            "https://api.z.ai/api/coding/paas/v4/chat/completions"
        );
    }

    #[test]
    fn openai_url_helper_normalizes_non_v1_versioned_endpoint_tails() {
        assert_eq!(
            resolve_openai_chat_completions_api_url(
                "https://api.z.ai/api/coding/paas/v4/chat/completion?foo=bar"
            ),
            "https://api.z.ai/api/coding/paas/v4/chat/completions?foo=bar"
        );
    }

    #[test]
    fn openai_url_helper_preserves_full_endpoint_and_query() {
        assert_eq!(
            resolve_openai_chat_completions_api_url(
                "https://gateway.example.com/openai/v1/chat/completions?foo=bar"
            ),
            "https://gateway.example.com/openai/v1/chat/completions?foo=bar"
        );
    }

    #[test]
    fn openai_url_helper_replaces_nonstandard_v1_tail() {
        assert_eq!(
            resolve_openai_chat_completions_api_url(
                "https://gateway.example.com/openai/v1/chat/completion"
            ),
            "https://gateway.example.com/openai/v1/chat/completions"
        );
    }

    #[test]
    fn resolve_compact_keep_recent_turns_uses_default_when_missing() {
        let runtime = RuntimeConfig::default();
        assert_eq!(
            resolve_compact_keep_recent_turns(&runtime),
            DEFAULT_COMPACT_KEEP_RECENT_TURNS
        );
    }

    #[test]
    fn resolve_compact_keep_recent_turns_honors_override() {
        let runtime = RuntimeConfig {
            compact_keep_recent_turns: Some(2),
            ..RuntimeConfig::default()
        };
        assert_eq!(resolve_compact_keep_recent_turns(&runtime), 2);
    }

    #[test]
    fn max_tool_concurrency_returns_default_when_env_unset() {
        std::env::remove_var(ASTRCODE_MAX_TOOL_CONCURRENCY_ENV);
        assert_eq!(max_tool_concurrency(), DEFAULT_MAX_TOOL_CONCURRENCY);
    }

    #[test]
    fn resolve_functions_use_defaults_for_empty_runtime() {
        let runtime = RuntimeConfig::default();
        assert_eq!(
            resolve_auto_compact_enabled(&runtime),
            DEFAULT_AUTO_COMPACT_ENABLED
        );
        assert_eq!(
            resolve_compact_threshold_percent(&runtime),
            DEFAULT_COMPACT_THRESHOLD_PERCENT
        );
        assert_eq!(
            resolve_tool_result_max_bytes(&runtime),
            DEFAULT_TOOL_RESULT_MAX_BYTES
        );
        assert_eq!(
            resolve_llm_connect_timeout_secs(&runtime),
            DEFAULT_LLM_CONNECT_TIMEOUT_SECS
        );
        assert_eq!(
            resolve_llm_read_timeout_secs(&runtime),
            DEFAULT_LLM_READ_TIMEOUT_SECS
        );
        assert_eq!(resolve_llm_max_retries(&runtime), DEFAULT_LLM_MAX_RETRIES);
        assert_eq!(
            resolve_session_broadcast_capacity(&runtime),
            DEFAULT_SESSION_BROADCAST_CAPACITY
        );
        assert_eq!(
            resolve_session_recent_record_limit(&runtime),
            DEFAULT_SESSION_RECENT_RECORD_LIMIT
        );
        assert_eq!(
            resolve_api_session_ttl_hours(&runtime),
            DEFAULT_API_SESSION_TTL_HOURS
        );
    }

    #[test]
    fn resolve_agent_config_uses_defaults_when_none() {
        assert_eq!(
            resolve_agent_max_subrun_depth(None),
            DEFAULT_MAX_SUBRUN_DEPTH
        );
        assert_eq!(
            resolve_agent_max_concurrent(None),
            DEFAULT_MAX_CONCURRENT_AGENTS
        );
        assert_eq!(
            resolve_agent_finalized_retain_limit(None),
            DEFAULT_FINALIZED_AGENT_RETAIN_LIMIT
        );
        assert_eq!(resolve_agent_inbox_capacity(None), DEFAULT_INBOX_CAPACITY);
        assert_eq!(
            resolve_agent_parent_delivery_capacity(None),
            DEFAULT_PARENT_DELIVERY_CAPACITY
        );
    }

    #[test]
    fn resolve_agent_config_honors_overrides() {
        use astrcode_core::AgentConfig;
        let agent = AgentConfig {
            max_subrun_depth: Some(5),
            max_concurrent: Some(3),
            finalized_retain_limit: Some(100),
            inbox_capacity: Some(512),
            parent_delivery_capacity: Some(256),
        };
        assert_eq!(resolve_agent_max_subrun_depth(Some(&agent)), 5);
        assert_eq!(resolve_agent_max_concurrent(Some(&agent)), 3);
        assert_eq!(resolve_agent_finalized_retain_limit(Some(&agent)), 100);
        assert_eq!(resolve_agent_inbox_capacity(Some(&agent)), 512);
        assert_eq!(resolve_agent_parent_delivery_capacity(Some(&agent)), 256);
    }
}
