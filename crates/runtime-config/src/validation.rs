//! Configuration normalization, migration, and validation.

use std::collections::HashSet;

use astrcode_core::{AstrError, Result};

use crate::constants::{CURRENT_CONFIG_VERSION, PROVIDER_KIND_ANTHROPIC, PROVIDER_KIND_OPENAI};
use crate::types::Config;

/// Normalizes and validates the configuration.
pub fn normalize_config(mut config: Config) -> Result<Config> {
    migrate_config(&mut config)?;
    validate_config(&config)?;
    Ok(config)
}

/// Migrates the configuration to the current version.
fn migrate_config(config: &mut Config) -> Result<()> {
    if config.version.trim().is_empty() {
        config.version = CURRENT_CONFIG_VERSION.to_string();
    }

    match config.version.as_str() {
        CURRENT_CONFIG_VERSION => {}
        other => {
            return Err(AstrError::Validation(format!(
                "unsupported config version: {}",
                other
            )))
        }
    }

    if config.active_profile.trim().is_empty() {
        config.active_profile = Config::default().active_profile;
    }

    if config.active_model.trim().is_empty() {
        config.active_model = Config::default().active_model;
    }

    Ok(())
}

/// Validates the configuration for correctness.
pub fn validate_config(config: &Config) -> Result<()> {
    if config.profiles.is_empty() {
        return Err(AstrError::Validation(
            "config must contain at least one profile".to_string(),
        ));
    }

    let mut seen_names = HashSet::new();
    for profile in &config.profiles {
        if profile.name.trim().is_empty() {
            return Err(AstrError::Validation(
                "profile name cannot be empty".to_string(),
            ));
        }
        if !seen_names.insert(profile.name.as_str()) {
            return Err(AstrError::Validation(format!(
                "duplicate profile name: {}",
                profile.name
            )));
        }
        if profile.models.is_empty() {
            return Err(AstrError::Validation(format!(
                "profile '{}' must contain at least one model",
                profile.name
            )));
        }
        if profile.max_tokens == 0 {
            return Err(AstrError::Validation(format!(
                "profile '{}' max_tokens must be greater than 0",
                profile.name
            )));
        }
        match profile.provider_kind.as_str() {
            PROVIDER_KIND_OPENAI => {
                if profile.base_url.trim().is_empty() {
                    return Err(AstrError::Validation(format!(
                        "profile '{}' base_url cannot be empty",
                        profile.name
                    )));
                }
            }
            PROVIDER_KIND_ANTHROPIC => {}
            other => {
                return Err(AstrError::Validation(format!(
                    "profile '{}' has unsupported provider_kind '{}'",
                    profile.name, other
                )))
            }
        }
    }

    let active_profile = config
        .profiles
        .iter()
        .find(|profile| profile.name == config.active_profile)
        .ok_or_else(|| {
            AstrError::Validation(format!(
                "active_profile '{}' not found",
                config.active_profile
            ))
        })?;
    if !active_profile
        .models
        .iter()
        .any(|model| model == &config.active_model)
    {
        return Err(AstrError::Validation(format!(
            "active_model '{}' is not configured under profile '{}'",
            config.active_model, config.active_profile
        )));
    }

    Ok(())
}
