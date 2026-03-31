//! API key resolution logic for Profile.

use astrcode_core::{AstrError, Result};

use crate::types::Profile;

impl Profile {
    /// Resolves the API key from the profile configuration.
    ///
    /// Supports three formats:
    /// - `literal:<value>`: Returns `<value>` directly.
    /// - `env:<name>`: Reads the environment variable `<name>`.
    /// - Plain value: If it looks like an env var name (uppercase + digits + underscores, contains `_`),
    ///   attempts to read it as an env var; falls back to treating it as a literal.
    pub fn resolve_api_key(&self) -> Result<String> {
        let val = match &self.api_key {
            None => {
                return Err(AstrError::MissingApiKey(format!(
                    "profile '{}' 未配置 apiKey",
                    self.name
                )))
            }
            Some(s) => s.trim().to_string(),
        };

        if val.is_empty() {
            return Err(AstrError::MissingApiKey(format!(
                "profile '{}' 的 apiKey 不能为空",
                self.name
            )));
        }

        if let Some(raw) = val.strip_prefix("literal:") {
            let literal = raw.trim().to_string();
            if literal.is_empty() {
                return Err(AstrError::MissingApiKey(format!(
                    "profile '{}' 的 apiKey 不能为空",
                    self.name
                )));
            }
            return Ok(literal);
        }

        if let Some(raw) = val.strip_prefix("env:") {
            let env_name = raw.trim();
            if !is_env_var_name(env_name) {
                return Err(AstrError::Validation(format!(
                    "profile '{}' 的 apiKey env 引用 '{}' 非法",
                    self.name, env_name
                )));
            }
            return std::env::var(env_name)
                .map_err(|_| AstrError::EnvVarNotFound(format!("环境变量 {} 未设置", env_name)));
        }

        if is_env_var_name(&val) {
            if let Ok(resolved) = std::env::var(&val) {
                return Ok(resolved);
            }
        }

        Ok(val)
    }
}

/// Checks if a value looks like an environment variable name
/// (uppercase letters, digits, underscores, and contains at least one underscore).
fn is_env_var_name(value: &str) -> bool {
    value
        .chars()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && value.contains('_')
}
