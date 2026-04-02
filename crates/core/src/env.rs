//! Workspace-defined environment variable names.
//!
//! These constants are the lowest-level source of truth for Astrcode-specific
//! environment variables so foundational crates do not need to depend on
//! higher-level configuration crates just to read process environment.

/// Overrides the Astrcode home directory in normal runtime execution.
pub const ASTRCODE_HOME_DIR_ENV: &str = "ASTRCODE_HOME_DIR";

/// Overrides the Astrcode home directory for test isolation.
pub const ASTRCODE_TEST_HOME_ENV: &str = "ASTRCODE_TEST_HOME";

/// Adds extra plugin discovery roots, separated using OS-specific path rules.
pub const ASTRCODE_PLUGIN_DIRS_ENV: &str = "ASTRCODE_PLUGIN_DIRS";

/// Supplies the Tauri target triple used when preparing the sidecar binary.
pub const TAURI_ENV_TARGET_TRIPLE_ENV: &str = "TAURI_ENV_TARGET_TRIPLE";

/// Default DeepSeek API key environment variable name.
pub const DEEPSEEK_API_KEY_ENV: &str = "DEEPSEEK_API_KEY";

/// Default Anthropic API key environment variable name.
pub const ANTHROPIC_API_KEY_ENV: &str = "ANTHROPIC_API_KEY";

/// Maximum number of concurrency-safe tools that may execute in parallel within a single step.
pub const ASTRCODE_MAX_TOOL_CONCURRENCY_ENV: &str = "ASTRCODE_MAX_TOOL_CONCURRENCY";
