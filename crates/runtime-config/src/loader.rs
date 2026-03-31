//! Configuration loading utilities.

use std::fs;
use std::path::{Path, PathBuf};

use astrcode_core::{AstrError, Result};

use crate::types::Config;
use crate::validation::normalize_config;

/// Returns the path to the config file.
pub fn config_path() -> Result<PathBuf> {
    let home = resolve_home_dir()?;
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
        return Ok(normalize_config(default_cfg)?);
    }

    let raw = fs::read_to_string(path)
        .map_err(|e| AstrError::io(format!("failed to read config at {}", path.display()), e))?;
    let config = serde_json::from_str::<Config>(&raw).map_err(|e| {
        AstrError::parse(format!("failed to parse config at {}", path.display()), e)
    })?;
    normalize_config(config)
        .map_err(|e| e.context(format!("failed to validate config at {}", path.display())))
}

/// Resolves the home directory for config storage.
///
/// In test mode, uses `ASTRCODE_TEST_HOME` if set.
/// In production, checks `ASTRCODE_HOME_DIR` env override, then falls back to `dirs::home_dir()`.
fn resolve_home_dir() -> Result<PathBuf> {
    #[cfg(test)]
    if let Some(home) = test_support::test_home_dir() {
        return Ok(home);
    }

    #[cfg(test)]
    {
        #[allow(clippy::needless_return)]
        return Err(AstrError::Internal(format!(
            "{} must be set before tests call config_path()",
            test_support::TEST_HOME_ENV
        )));
    }

    #[cfg(not(test))]
    {
        const APP_HOME_OVERRIDE_ENV: &str = "ASTRCODE_HOME_DIR";
        const TEST_HOME_OVERRIDE_ENV: &str = "ASTRCODE_TEST_HOME";

        if let Some(home) = std::env::var_os(APP_HOME_OVERRIDE_ENV) {
            if !home.is_empty() {
                return Ok(PathBuf::from(home));
            }
        }

        // Also check test home override in non-test builds, since dependent
        // crates may be exercised by integration tests that set this variable.
        if let Some(home) = std::env::var_os(TEST_HOME_OVERRIDE_ENV) {
            if !home.is_empty() {
                return Ok(PathBuf::from(home));
            }
        }

        dirs::home_dir().ok_or(AstrError::HomeDirectoryNotFound)
    }
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

/// Test support utilities for isolated home directory testing.
#[cfg(test)]
pub(crate) mod test_support {
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, MutexGuard, OnceLock};

    use tempfile::TempDir;

    pub(crate) const TEST_HOME_ENV: &str = "ASTRCODE_TEST_HOME";

    pub(crate) fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    pub(crate) fn test_home_dir() -> Option<PathBuf> {
        std::env::var_os(TEST_HOME_ENV).map(PathBuf::from)
    }

    pub(crate) struct TestEnvGuard {
        _lock: MutexGuard<'static, ()>,
        _temp_home: TempDir,
        previous_dir: PathBuf,
        previous_home: Option<std::ffi::OsString>,
        previous_userprofile: Option<std::ffi::OsString>,
        previous_test_home: Option<std::ffi::OsString>,
    }

    impl TestEnvGuard {
        pub(crate) fn new() -> Self {
            let lock = env_lock().lock().expect("env lock should be acquired");
            let temp_home = tempfile::tempdir().expect("tempdir should be created");
            let previous_dir = std::env::current_dir().expect("cwd should resolve");
            let previous_home = std::env::var_os("HOME");
            let previous_userprofile = std::env::var_os("USERPROFILE");
            let previous_test_home = std::env::var_os(TEST_HOME_ENV);

            std::env::set_var(TEST_HOME_ENV, temp_home.path());
            #[cfg(windows)]
            {
                std::env::set_var("USERPROFILE", temp_home.path());
                std::env::remove_var("HOME");
            }
            #[cfg(not(windows))]
            {
                std::env::set_var("HOME", temp_home.path());
                std::env::remove_var("USERPROFILE");
            }

            Self {
                _lock: lock,
                _temp_home: temp_home,
                previous_dir,
                previous_home,
                previous_userprofile,
                previous_test_home,
            }
        }

        pub(crate) fn home_dir(&self) -> &Path {
            self._temp_home.path()
        }

        #[allow(dead_code)]
        pub(crate) fn set_current_dir<P: AsRef<Path>>(&self, path: P) {
            std::env::set_current_dir(path).expect("set cwd should work");
        }
    }

    impl Drop for TestEnvGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.previous_dir);

            match &self.previous_home {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
            match &self.previous_userprofile {
                Some(value) => std::env::set_var("USERPROFILE", value),
                None => std::env::remove_var("USERPROFILE"),
            }
            match &self.previous_test_home {
                Some(value) => std::env::set_var(TEST_HOME_ENV, value),
                None => std::env::remove_var(TEST_HOME_ENV),
            }
        }
    }
}
