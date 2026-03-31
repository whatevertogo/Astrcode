//! Configuration loading utilities.

use std::fs;
use std::path::{Path, PathBuf};

use astrcode_core::{AstrError, Result};

use crate::types::Config;
use crate::validation::normalize_config;

/// Returns the path to the config file.
pub fn config_path() -> Result<PathBuf> {
    let home = astrcode_core::home::resolve_home_dir()?;
    Ok(home.join(".astrcode").join("config.json"))
}

/// Loads the configuration from the default path.
pub fn load_config() -> Result<Config> {
    let path = config_path()?;
    load_config_from_path(&path)
}

/// Loads the configuration from a specific path.
pub fn load_config_from_path(path: &Path) -> Result<Config> {
    if !path.exists() {
        let parent = path.parent().ok_or_else(|| {
            AstrError::Internal(format!("config path has no parent: {}", path.display()))
        })?;
        fs::create_dir_all(parent).map_err(|e| {
            AstrError::io(
                format!("failed to create config directory for {}", parent.display()),
                e,
            )
        })?;

        let default_cfg = Config::default();
        write_json_atomic(path, &default_cfg).map_err(|e| {
            e.context(format!(
                "failed to initialize config file at {}",
                path.display()
            ))
        })?;

        println!("Config created at {}，请填写 apiKey", path.display());
        return normalize_config(default_cfg);
    }

    let raw = fs::read_to_string(path)
        .map_err(|e| AstrError::io(format!("failed to read config at {}", path.display()), e))?;
    let config = serde_json::from_str::<Config>(&raw).map_err(|e| {
        AstrError::parse(format!("failed to parse config at {}", path.display()), e)
    })?;
    normalize_config(config)
        .map_err(|e| e.context(format!("failed to validate config at {}", path.display())))
}

/// Writes JSON atomically via a temp file and rename.
pub(crate) fn write_json_atomic(path: &Path, config: &Config) -> Result<()> {
    use std::io::Write;

    let json = serde_json::to_vec_pretty(config)
        .map_err(|e| AstrError::parse("failed to serialize config", e))?;
    let tmp_path = path.with_extension("json.tmp");
    let mut tmp_file = fs::File::create(&tmp_path).map_err(|e| {
        AstrError::io(
            format!("failed to create temp config at {}", tmp_path.display()),
            e,
        )
    })?;
    tmp_file.write_all(&json).map_err(|e| {
        AstrError::io(
            format!("failed to write temp config at {}", tmp_path.display()),
            e,
        )
    })?;
    tmp_file.flush().map_err(|e| {
        AstrError::io(
            format!("failed to flush temp config at {}", tmp_path.display()),
            e,
        )
    })?;
    tmp_file.sync_all().map_err(|e| {
        AstrError::io(
            format!("failed to fsync temp config at {}", tmp_path.display()),
            e,
        )
    })?;
    drop(tmp_file);

    // On most platforms, `rename` will atomically replace the destination.
    // On Windows, `std::fs::rename` fails with `AlreadyExists` if the
    // destination exists, so swap via a backup path and try to roll back
    // if the second rename fails.
    if let Err(err) = fs::rename(&tmp_path, path) {
        #[cfg(windows)]
        {
            if err.kind() == std::io::ErrorKind::AlreadyExists {
                let backup_path = path.with_extension("json.bak");
                let _ = fs::remove_file(&backup_path);

                // Move old config out of the way before placing the new file.
                if let Err(backup_err) = fs::rename(path, &backup_path) {
                    let _ = fs::remove_file(&tmp_path);
                    return Err(AstrError::Internal(format!(
                        "failed to move existing config {} to backup {} before replace: {}",
                        path.display(),
                        backup_path.display(),
                        backup_err
                    )));
                }

                if let Err(rename_err) = fs::rename(&tmp_path, path) {
                    match fs::rename(&backup_path, path) {
                        Ok(()) => return Err(AstrError::Internal(format!(
                            "failed to replace config {} with temp file {}: {}; original config restored from backup {} (temp file kept for recovery)",
                            path.display(),
                            tmp_path.display(),
                            rename_err,
                            backup_path.display()
                        ))),
                        Err(restore_err) => return Err(AstrError::Internal(format!(
                            "failed to replace config {} with temp file {}: {}; also failed to restore backup {}: {} (temp file kept for recovery)",
                            path.display(),
                            tmp_path.display(),
                            rename_err,
                            backup_path.display(),
                            restore_err
                        ))),
                    }
                }

                let _ = fs::remove_file(&backup_path);
            } else {
                let _ = fs::remove_file(&tmp_path);
                return Err(AstrError::Internal(format!(
                    "failed to replace config {} with temp file {}: {}",
                    path.display(),
                    tmp_path.display(),
                    err
                )));
            }
        }
        #[cfg(not(windows))]
        {
            let _ = fs::remove_file(&tmp_path);
            return Err(AstrError::Internal(format!(
                "failed to replace config {} with temp file {}: {}",
                path.display(),
                tmp_path.display(),
                err
            )));
        }
    }
    Ok(())
}
