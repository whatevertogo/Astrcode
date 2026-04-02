//! Shared environment value parsing for runtime configuration fields.

use astrcode_core::{AstrError, Result};

use crate::constants::{ENV_REFERENCE_PREFIX, LITERAL_VALUE_PREFIX};

/// Parsed configuration value shape before env lookup is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParsedEnvValue<'a> {
    /// Treat the raw value as-is.
    Literal(&'a str),
    /// Require the value to be loaded from the named environment variable.
    ExplicitEnv(&'a str),
    /// Legacy bare env-like names are resolved when present but stay literal otherwise.
    OptionalEnv(&'a str),
}

/// Parses a config value into either a literal or an env-backed reference.
///
/// Centralizing this logic keeps `runtime-config` consistent about which strings
/// are allowed to escape config files and which strings require process env.
pub fn parse_env_value(raw: &str) -> Result<ParsedEnvValue<'_>> {
    let trimmed = raw.trim();

    if let Some(literal) = trimmed.strip_prefix(LITERAL_VALUE_PREFIX) {
        return Ok(ParsedEnvValue::Literal(literal.trim()));
    }

    if let Some(env_name) = trimmed.strip_prefix(ENV_REFERENCE_PREFIX) {
        let env_name = env_name.trim();
        if !is_env_var_name(env_name) {
            return Err(AstrError::Validation(format!(
                "env 引用 '{}' 非法",
                env_name
            )));
        }
        return Ok(ParsedEnvValue::ExplicitEnv(env_name));
    }

    if is_env_var_name(trimmed) {
        return Ok(ParsedEnvValue::OptionalEnv(trimmed));
    }

    Ok(ParsedEnvValue::Literal(trimmed))
}

/// Resolves a parsed config value to the effective runtime value.
///
/// We only materialize env values in memory so secrets can come from the
/// process environment without being rewritten back into `config.json`.
pub fn resolve_env_value(raw: &str) -> Result<String> {
    match parse_env_value(raw)? {
        ParsedEnvValue::Literal(value) => Ok(value.to_string()),
        ParsedEnvValue::ExplicitEnv(env_name) => std::env::var(env_name)
            .map_err(|_| AstrError::EnvVarNotFound(format!("环境变量 {} 未设置", env_name))),
        ParsedEnvValue::OptionalEnv(env_name) => {
            Ok(std::env::var(env_name).unwrap_or_else(|_| env_name.to_string()))
        }
    }
}

/// Builds the serialized `env:<NAME>` reference used by default config values.
pub fn env_reference(env_name: &str) -> String {
    format!("{ENV_REFERENCE_PREFIX}{env_name}")
}

/// Checks if a value looks like an environment variable name
/// (uppercase letters, digits, underscores, and contains at least one underscore).
pub fn is_env_var_name(value: &str) -> bool {
    value
        .chars()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && value.contains('_')
}
