//! 配置常量、环境变量分组与 URL 标准化辅助。
//!
//! runtime 默认值与解析逻辑已经下沉到 `core::config`，本模块只保留
//! application 层仍然拥有的环境变量、Provider 约定与 URL 规范化辅助。
//!
//! # 设计原则
//!
//! - 类型定义与 runtime 默认值统一收敛到 `core::config`
//! - application 只保留自身拥有的环境变量分组与 URL 辅助
//!
//! # URL 标准化
//!
//! `resolve_*_api_url` 系列函数处理 Provider 地址的多种写法（根地址、版本根、完整集合地址），
//! 确保运行时始终拿到可直接发请求的完整 URL。

pub use astrcode_core::config::DEFAULT_MAX_SUBRUN_DEPTH;

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

pub use astrcode_core::config::{
    DEFAULT_AGGREGATE_RESULT_BYTES_BUDGET, DEFAULT_API_SESSION_TTL_HOURS,
    DEFAULT_AUTO_COMPACT_ENABLED, DEFAULT_COMPACT_KEEP_RECENT_TURNS,
    DEFAULT_COMPACT_THRESHOLD_PERCENT, DEFAULT_FINALIZED_AGENT_RETAIN_LIMIT,
    DEFAULT_INBOX_CAPACITY, DEFAULT_LLM_CONNECT_TIMEOUT_SECS, DEFAULT_LLM_MAX_RETRIES,
    DEFAULT_LLM_READ_TIMEOUT_SECS, DEFAULT_LLM_RETRY_BASE_DELAY_MS, DEFAULT_MAX_CONCURRENT_AGENTS,
    DEFAULT_MAX_CONCURRENT_BRANCH_DEPTH, DEFAULT_MAX_CONSECUTIVE_FAILURES, DEFAULT_MAX_GREP_LINES,
    DEFAULT_MAX_IMAGE_SIZE, DEFAULT_MAX_OUTPUT_CONTINUATION_ATTEMPTS,
    DEFAULT_MAX_REACTIVE_COMPACT_ATTEMPTS, DEFAULT_MAX_RECOVERED_FILES, DEFAULT_MAX_STEPS,
    DEFAULT_MAX_TOOL_CONCURRENCY, DEFAULT_MAX_TRACKED_FILES,
    DEFAULT_MICRO_COMPACT_GAP_THRESHOLD_SECS, DEFAULT_MICRO_COMPACT_KEEP_RECENT_RESULTS,
    DEFAULT_PARENT_DELIVERY_CAPACITY, DEFAULT_RECOVERY_TOKEN_BUDGET,
    DEFAULT_RECOVERY_TRUNCATE_BYTES, DEFAULT_SESSION_BROADCAST_CAPACITY,
    DEFAULT_SESSION_RECENT_RECORD_LIMIT, DEFAULT_SUMMARY_RESERVE_TOKENS,
    DEFAULT_TOOL_RESULT_INLINE_LIMIT, DEFAULT_TOOL_RESULT_MAX_BYTES,
    DEFAULT_TOOL_RESULT_PREVIEW_LIMIT,
};

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

#[cfg(test)]
mod tests {
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
}
