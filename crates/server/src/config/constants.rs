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
#![allow(dead_code)]

// ============================================================
// Provider 标识符
// ============================================================

/// OpenAI 家族 Provider 标识符。
///
/// 用于 `Profile.provider_kind` 字段，表示该 Provider 使用 OpenAI 兼容协议，
/// 并可按 `apiMode` 切换 `responses` 或 `chat_completions`。
pub const PROVIDER_KIND_OPENAI: &str = "openai";

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
    ASTRCODE_HOME_DIR_ENV, ASTRCODE_MAX_TOOL_CONCURRENCY_ENV, ASTRCODE_PLUGIN_DIRS_ENV,
    ASTRCODE_TEST_HOME_ENV, ASTRCODE_TOOL_RESULT_INLINE_LIMIT_ENV, DEEPSEEK_API_KEY_ENV,
    OPENAI_API_KEY_ENV, TAURI_ENV_TARGET_TRIPLE_ENV,
};

/// 影响 Astrcode 本地存储路径的环境变量。
pub const HOME_ENV_VARS: &[&str] = &[ASTRCODE_HOME_DIR_ENV, ASTRCODE_TEST_HOME_ENV];

/// 影响运行时插件发现的环境变量。
pub const PLUGIN_ENV_VARS: &[&str] = &[ASTRCODE_PLUGIN_DIRS_ENV];

/// 内置 Provider 默认配置使用的 API key 环境变量。
pub const PROVIDER_API_KEY_ENV_VARS: &[&str] = &[DEEPSEEK_API_KEY_ENV, OPENAI_API_KEY_ENV];

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
    OPENAI_API_KEY_ENV,
    TAURI_ENV_TARGET_TRIPLE_ENV,
    ASTRCODE_MAX_TOOL_CONCURRENCY_ENV,
    ASTRCODE_TOOL_RESULT_INLINE_LIMIT_ENV,
];

// ============================================================
// API URL 常量
// ============================================================

/// OpenAI 官方 Chat Completions API endpoint URL。
pub const OPENAI_CHAT_COMPLETIONS_API_URL: &str = "https://api.openai.com/v1/chat/completions";

/// OpenAI 官方 Responses API endpoint URL。
pub const OPENAI_RESPONSES_API_URL: &str = "https://api.openai.com/v1/responses";

/// OpenAI 家族模型的保守默认上下文窗口。
///
/// 用于默认生成的 OpenAI profile，避免首次创建配置文件时出现空 limits。
pub const DEFAULT_OPENAI_CONTEXT_LIMIT: usize = 128_000;

// ============================================================
// 配置 schema 版本
// ============================================================

/// 配置 schema 的当前版本号。
///
/// 加载配置时空白的 version 字段会被迁移到此值，不支持的版本号会导致加载失败。
pub const CURRENT_CONFIG_VERSION: &str = "1";

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

fn replace_openai_collection_tail(trimmed: &str, collection_suffix: &str) -> Option<String> {
    const KNOWN_SUFFIXES: &[&str] = &[
        "/chat/completions",
        "/chat/completion",
        "/chat",
        "/responses",
        "/response",
    ];

    KNOWN_SUFFIXES.iter().find_map(|suffix| {
        trimmed
            .strip_suffix(suffix)
            .map(|prefix| format!("{prefix}/{collection_suffix}"))
    })
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
        return OPENAI_CHAT_COMPLETIONS_API_URL.to_string();
    }

    let normalized = if trimmed.ends_with("/chat/completions") {
        trimmed.to_string()
    } else if let Some(replaced) = replace_openai_collection_tail(trimmed, "chat/completions") {
        replaced
    } else if let Some(versioned_url) =
        normalize_openai_versioned_base_url(trimmed, "chat/completions")
    {
        versioned_url
    } else {
        format!("{trimmed}/v1/chat/completions")
    };

    join_url_query(normalized, query)
}

/// 解析 OpenAI Responses API 地址。
///
/// 兼容四种写法：
/// - API 根地址：如 `https://api.openai.com`
/// - 版本根地址：如 `https://api.openai.com/v1`
/// - 完整集合地址：如 `https://api.openai.com/v1/responses`
/// - 第三方版本根地址：如 `https://gateway.example.com/openai/v1`
pub fn resolve_openai_responses_api_url(base_url: &str) -> String {
    let (path, query) = split_url_query(base_url.trim());
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return OPENAI_RESPONSES_API_URL.to_string();
    }

    let normalized = if trimmed.ends_with("/responses") {
        trimmed.to_string()
    } else if let Some(replaced) = replace_openai_collection_tail(trimmed, "responses") {
        replaced
    } else if let Some(versioned_url) = normalize_openai_versioned_base_url(trimmed, "responses") {
        versioned_url
    } else {
        format!("{trimmed}/v1/responses")
    };

    join_url_query(normalized, query)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn responses_url_helper_falls_back_to_official_default() {
        assert_eq!(
            resolve_openai_responses_api_url(""),
            OPENAI_RESPONSES_API_URL
        );
    }

    #[test]
    fn responses_url_helper_expands_root_style_base_url() {
        assert_eq!(
            resolve_openai_responses_api_url("https://api.openai.com/v1"),
            "https://api.openai.com/v1/responses"
        );
    }

    #[test]
    fn responses_url_helper_replaces_chat_collection_tail() {
        assert_eq!(
            resolve_openai_responses_api_url(
                "https://gateway.example.com/openai/v1/chat/completions"
            ),
            "https://gateway.example.com/openai/v1/responses"
        );
    }
}
