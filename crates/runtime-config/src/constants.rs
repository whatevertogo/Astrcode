//! Runtime configuration constants.
//!
//! Astrcode-specific environment variable names are grouped here by domain so
//! configuration-related code and docs have a single categorized index.

/// OpenAI-compatible provider kind identifier.
pub const PROVIDER_KIND_OPENAI: &str = "openai-compatible";

/// Anthropic provider kind identifier.
pub const PROVIDER_KIND_ANTHROPIC: &str = "anthropic";

/// Prefix for values that must be read from an environment variable.
pub const ENV_REFERENCE_PREFIX: &str = "env:";

/// Prefix for values that must stay literal and skip env resolution.
pub const LITERAL_VALUE_PREFIX: &str = "literal:";

pub use astrcode_core::env::{
    ANTHROPIC_API_KEY_ENV, ASTRCODE_HOME_DIR_ENV, ASTRCODE_PLUGIN_DIRS_ENV, ASTRCODE_TEST_HOME_ENV,
    DEEPSEEK_API_KEY_ENV, TAURI_ENV_TARGET_TRIPLE_ENV,
};

/// Environment variables that affect where Astrcode stores local state.
pub const HOME_ENV_VARS: &[&str] = &[ASTRCODE_HOME_DIR_ENV, ASTRCODE_TEST_HOME_ENV];

/// Environment variables that influence runtime plugin discovery.
pub const PLUGIN_ENV_VARS: &[&str] = &[ASTRCODE_PLUGIN_DIRS_ENV];

/// Environment variables used by the built-in provider defaults.
pub const PROVIDER_API_KEY_ENV_VARS: &[&str] = &[DEEPSEEK_API_KEY_ENV, ANTHROPIC_API_KEY_ENV];

/// Environment variables required by the Tauri sidecar build pipeline.
pub const BUILD_ENV_VARS: &[&str] = &[TAURI_ENV_TARGET_TRIPLE_ENV];

/// All Astrcode-defined environment variables, grouped above by responsibility.
pub const ALL_ASTRCODE_ENV_VARS: &[&str] = &[
    ASTRCODE_HOME_DIR_ENV,
    ASTRCODE_TEST_HOME_ENV,
    ASTRCODE_PLUGIN_DIRS_ENV,
    DEEPSEEK_API_KEY_ENV,
    ANTHROPIC_API_KEY_ENV,
    TAURI_ENV_TARGET_TRIPLE_ENV,
];

/// Anthropic Messages API endpoint URL.
pub const ANTHROPIC_MESSAGES_API_URL: &str = "https://api.anthropic.com/v1/messages";

/// Anthropic API version.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Current configuration schema version.
pub const CURRENT_CONFIG_VERSION: &str = "1";
