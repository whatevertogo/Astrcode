use anyhow::{anyhow, Result};
use std::path::PathBuf;

pub fn resolve_home_dir() -> Result<PathBuf> {
    if let Some(home) = std::env::var_os("ASTRCODE_HOME_DIR") {
        if !home.is_empty() {
            return Ok(PathBuf::from(home));
        }
    }
    dirs::home_dir().ok_or_else(|| anyhow!("unable to resolve home directory"))
}

pub fn default_config_path() -> Result<PathBuf> {
    Ok(resolve_home_dir()?.join(".astrcode").join("config.json"))
}
