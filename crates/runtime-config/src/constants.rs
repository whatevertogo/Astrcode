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
