use anyhow::{anyhow, Result};
use astrcode_core::env::ASTRCODE_HOME_DIR_ENV;
use std::path::PathBuf;

pub fn resolve_home_dir() -> Result<PathBuf> {
    // Keep the desktop shell aligned with the workspace env catalog in
    // `runtime-config/constants.rs`, which re-exports this core constant.
    if let Some(home) = std::env::var_os(ASTRCODE_HOME_DIR_ENV) {
        if !home.is_empty() {
            return Ok(PathBuf::from(home));
        }
    }
    dirs::home_dir().ok_or_else(|| anyhow!("unable to resolve home directory"))
}

pub fn default_config_path() -> Result<PathBuf> {
    Ok(resolve_home_dir()?.join(".astrcode").join("config.json"))
}

/// 获取 Astrcode 根目录 (~/.astrcode/)
pub fn astrcode_root_dir() -> Result<PathBuf> {
    Ok(resolve_home_dir()?.join(".astrcode"))
}
