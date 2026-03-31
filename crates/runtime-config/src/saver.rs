//! Configuration saving utilities.

use std::fs;
use std::path::Path;

use astrcode_core::{AstrError, Result};

use crate::loader::write_json_atomic;
use crate::types::Config;
use crate::validation::normalize_config;

/// Saves the configuration to the default path.
pub fn save_config(config: &Config) -> Result<()> {
    let path = crate::loader::config_path()?;
    save_config_to_path(&path, config)
}

/// Saves the configuration to a specific path.
pub fn save_config_to_path(path: &Path, config: &Config) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        AstrError::Internal(format!("config path has no parent: {}", path.display()))
    })?;
    fs::create_dir_all(parent).map_err(|e| {
        AstrError::io(
            format!("failed to create config directory for {}", parent.display()),
            e,
        )
    })?;

    let normalized = normalize_config(config.clone())?;
    write_json_atomic(path, &normalized)
}
