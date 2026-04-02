//! API key resolution logic for Profile.

use astrcode_core::{AstrError, Result};

use crate::env::resolve_env_value;
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

        let resolved = resolve_env_value(&val).map_err(|error| match error {
            // Preserve profile context here so callers keep seeing the same actionable error.
            AstrError::Validation(message) => {
                AstrError::Validation(format!("profile '{}' 的 apiKey {}", self.name, message))
            }
            other => other,
        })?;
        if resolved.is_empty() {
            return Err(AstrError::MissingApiKey(format!(
                "profile '{}' 的 apiKey 不能为空",
                self.name
            )));
        }

        Ok(resolved)
    }
}
