//! Runtime configuration constants.
//!
//! Astrcode-specific environment variable names are grouped here by domain so
//! configuration-related code and docs have a single categorized index.

use crate::types::RuntimeConfig;

/// OpenAI-compatible provider kind identifier.
pub const PROVIDER_KIND_OPENAI: &str = "openai-compatible";

/// Anthropic provider kind identifier.
pub const PROVIDER_KIND_ANTHROPIC: &str = "anthropic";

/// Prefix for values that must be read from an environment variable.
pub const ENV_REFERENCE_PREFIX: &str = "env:";

/// Prefix for values that must stay literal and skip env resolution.
pub const LITERAL_VALUE_PREFIX: &str = "literal:";

pub use astrcode_core::env::{
    ANTHROPIC_API_KEY_ENV, ASTRCODE_HOME_DIR_ENV, ASTRCODE_MAX_TOOL_CONCURRENCY_ENV,
    ASTRCODE_PLUGIN_DIRS_ENV, ASTRCODE_TEST_HOME_ENV, DEEPSEEK_API_KEY_ENV,
    TAURI_ENV_TARGET_TRIPLE_ENV,
};

/// Environment variables that affect where Astrcode stores local state.
pub const HOME_ENV_VARS: &[&str] = &[ASTRCODE_HOME_DIR_ENV, ASTRCODE_TEST_HOME_ENV];

/// Environment variables that influence runtime plugin discovery.
pub const PLUGIN_ENV_VARS: &[&str] = &[ASTRCODE_PLUGIN_DIRS_ENV];

/// Environment variables used by the built-in provider defaults.
pub const PROVIDER_API_KEY_ENV_VARS: &[&str] = &[DEEPSEEK_API_KEY_ENV, ANTHROPIC_API_KEY_ENV];

/// Environment variables required by the Tauri sidecar build pipeline.
pub const BUILD_ENV_VARS: &[&str] = &[TAURI_ENV_TARGET_TRIPLE_ENV];

/// Environment variables that tune runtime execution behavior.
pub const RUNTIME_ENV_VARS: &[&str] = &[ASTRCODE_MAX_TOOL_CONCURRENCY_ENV];

/// All Astrcode-defined environment variables, grouped above by responsibility.
pub const ALL_ASTRCODE_ENV_VARS: &[&str] = &[
    ASTRCODE_HOME_DIR_ENV,
    ASTRCODE_TEST_HOME_ENV,
    ASTRCODE_PLUGIN_DIRS_ENV,
    DEEPSEEK_API_KEY_ENV,
    ANTHROPIC_API_KEY_ENV,
    TAURI_ENV_TARGET_TRIPLE_ENV,
    ASTRCODE_MAX_TOOL_CONCURRENCY_ENV,
];

/// Anthropic Messages API endpoint URL.
pub const ANTHROPIC_MESSAGES_API_URL: &str = "https://api.anthropic.com/v1/messages";

/// Anthropic API version.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Current configuration schema version.
pub const CURRENT_CONFIG_VERSION: &str = "1";

/// Default maximum number of concurrency-safe tools that may execute in parallel.
pub const DEFAULT_MAX_TOOL_CONCURRENCY: usize = 10;
/// Default auto-compact toggle.
pub const DEFAULT_AUTO_COMPACT_ENABLED: bool = true;
/// Default percentage of effective context window used before compaction starts.
pub const DEFAULT_COMPACT_THRESHOLD_PERCENT: u8 = 90;
/// Default per-tool-result request budget in bytes.
pub const DEFAULT_TOOL_RESULT_MAX_BYTES: usize = 100_000;
/// Default number of recent user turns kept verbatim during compaction.
pub const DEFAULT_COMPACT_KEEP_RECENT_TURNS: u8 = 4;
/// Default token budget. Zero disables auto-continue.
pub const DEFAULT_TOKEN_BUDGET: u64 = 0;
/// Default diminishing-returns threshold for auto-continue.
pub const DEFAULT_CONTINUATION_MIN_DELTA_TOKENS: usize = 500;
/// Default maximum number of continuation turns.
pub const DEFAULT_MAX_CONTINUATIONS: u8 = 3;

/// Returns the maximum number of concurrency-safe tools from process env/defaults.
///
/// This helper is intentionally env-only so low-level callers that have not
/// loaded `config.json` yet can still honor the OS override. Higher-level
/// runtime services should prefer [`resolve_max_tool_concurrency`] so the
/// user-configured `runtime.maxToolConcurrency` block stays authoritative.
pub fn max_tool_concurrency() -> usize {
    std::env::var(ASTRCODE_MAX_TOOL_CONCURRENCY_ENV)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_MAX_TOOL_CONCURRENCY)
        .max(1)
}

/// Resolves the effective tool parallelism cap for a loaded runtime config.
///
/// Resolution order is:
/// 1. `config.runtime.maxToolConcurrency`
/// 2. `ASTRCODE_MAX_TOOL_CONCURRENCY`
/// 3. built-in default
///
/// This keeps runtime tuning centralized in `config.json` without breaking
/// existing env-based deployments and tests.
pub fn resolve_max_tool_concurrency(runtime: &RuntimeConfig) -> usize {
    runtime
        .max_tool_concurrency
        .unwrap_or_else(max_tool_concurrency)
        .max(1)
}

pub fn resolve_auto_compact_enabled(runtime: &RuntimeConfig) -> bool {
    runtime
        .auto_compact_enabled
        .unwrap_or(DEFAULT_AUTO_COMPACT_ENABLED)
}

pub fn resolve_compact_threshold_percent(runtime: &RuntimeConfig) -> u8 {
    runtime
        .compact_threshold_percent
        .unwrap_or(DEFAULT_COMPACT_THRESHOLD_PERCENT)
        .clamp(1, 100)
}

pub fn resolve_tool_result_max_bytes(runtime: &RuntimeConfig) -> usize {
    runtime
        .tool_result_max_bytes
        .unwrap_or(DEFAULT_TOOL_RESULT_MAX_BYTES)
        .max(1)
}

pub fn resolve_compact_keep_recent_turns(runtime: &RuntimeConfig) -> u8 {
    runtime
        .compact_keep_recent_turns
        .unwrap_or(DEFAULT_COMPACT_KEEP_RECENT_TURNS)
        .max(1)
}

pub fn resolve_default_token_budget(runtime: &RuntimeConfig) -> u64 {
    runtime.default_token_budget.unwrap_or(DEFAULT_TOKEN_BUDGET)
}

pub fn resolve_continuation_min_delta_tokens(runtime: &RuntimeConfig) -> usize {
    runtime
        .continuation_min_delta_tokens
        .unwrap_or(DEFAULT_CONTINUATION_MIN_DELTA_TOKENS)
        .max(1)
}

pub fn resolve_max_continuations(runtime: &RuntimeConfig) -> u8 {
    runtime
        .max_continuations
        .unwrap_or(DEFAULT_MAX_CONTINUATIONS)
        .max(1)
}
