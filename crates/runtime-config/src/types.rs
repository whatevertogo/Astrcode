//! Configuration data types: Config, Profile, and TestResult.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::constants::{CURRENT_CONFIG_VERSION, PROVIDER_KIND_ANTHROPIC, PROVIDER_KIND_OPENAI};

/// Top-level application configuration.
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
    #[serde(default = "default_config_profiles")]
    pub profiles: Vec<Profile>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CURRENT_CONFIG_VERSION.to_string(),
            active_profile: "deepseek".to_string(),
            active_model: "deepseek-chat".to_string(),
            profiles: default_profiles(),
        }
    }
}

/// LLM provider profile configuration.
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
            api_key: Some("env:DEEPSEEK_API_KEY".to_string()),
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
            .field("profiles", &self.profiles)
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

/// Result of a connection test.
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

fn default_profiles() -> Vec<Profile> {
    vec![
        Profile {
            name: "deepseek".to_string(),
            provider_kind: PROVIDER_KIND_OPENAI.to_string(),
            base_url: "https://api.deepseek.com".to_string(),
            api_key: Some("env:DEEPSEEK_API_KEY".to_string()),
            models: vec!["deepseek-chat".to_string(), "deepseek-reasoner".to_string()],
            max_tokens: 8096,
        },
        Profile {
            name: "anthropic".to_string(),
            provider_kind: PROVIDER_KIND_ANTHROPIC.to_string(),
            base_url: String::new(),
            api_key: Some("env:ANTHROPIC_API_KEY".to_string()),
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
    Some("env:DEEPSEEK_API_KEY".to_string())
}

fn default_profile_models() -> Vec<String> {
    vec!["deepseek-chat".to_string(), "deepseek-reasoner".to_string()]
}

fn default_profile_max_tokens() -> u32 {
    8096
}
