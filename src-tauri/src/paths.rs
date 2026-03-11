use std::path::PathBuf;

pub fn resolve_home_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("ASTRCODE_HOME_DIR") {
        if !home.is_empty() {
            return PathBuf::from(home);
        }
    }
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

pub fn default_config_path() -> PathBuf {
    resolve_home_dir().join(".astrcode").join("config.json")
}
