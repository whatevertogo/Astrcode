//! Provider kind constants and configuration version constants.

/// OpenAI-compatible provider kind identifier.
pub const PROVIDER_KIND_OPENAI: &str = "openai-compatible";

/// Anthropic provider kind identifier.
pub const PROVIDER_KIND_ANTHROPIC: &str = "anthropic";

/// Anthropic Messages API endpoint URL.
pub const ANTHROPIC_MESSAGES_API_URL: &str = "https://api.anthropic.com/v1/messages";

/// Anthropic API version.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Current configuration schema version.
pub const CURRENT_CONFIG_VERSION: &str = "1";
