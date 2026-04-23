//! 宿主机路径解析。
//!
//! 负责 Astrcode home / projects 等宿主路径能力，避免 `core`
//! 继续持有 `dirs`、`canonicalize` 等 owner。

use std::path::{Path, PathBuf};

use astrcode_core::{AstrError, Result};
pub use astrcode_core::{
    env::{ASTRCODE_HOME_DIR_ENV, ASTRCODE_TEST_HOME_ENV},
    project::project_dir_name,
};

/// 解析 Astrcode 的宿主 home 目录。
///
/// 解析顺序：
/// 1. `ASTRCODE_TEST_HOME`
/// 2. `ASTRCODE_HOME_DIR`
/// 3. `dirs::home_dir()`
pub fn resolve_home_dir() -> Result<PathBuf> {
    if let Some(home) = std::env::var_os(ASTRCODE_TEST_HOME_ENV) {
        if !home.is_empty() {
            return Ok(PathBuf::from(home));
        }
    }

    if let Some(home) = std::env::var_os(ASTRCODE_HOME_DIR_ENV) {
        if !home.is_empty() {
            return Ok(PathBuf::from(home));
        }
    }

    dirs::home_dir().ok_or(AstrError::HomeDirectoryNotFound)
}

/// 返回 `~/.astrcode` 根目录。
pub fn astrcode_dir() -> Result<PathBuf> {
    Ok(resolve_home_dir()?.join(".astrcode"))
}

/// 返回 `~/.astrcode/projects` 根目录。
pub fn projects_dir() -> Result<PathBuf> {
    Ok(astrcode_dir()?.join("projects"))
}

/// 返回工作目录对应的项目持久化目录。
pub fn project_dir(working_dir: &Path) -> Result<PathBuf> {
    Ok(projects_dir()?.join(project_dir_name(working_dir)))
}

#[cfg(test)]
mod tests {
    use std::{
        ffi::OsString,
        sync::{Mutex, OnceLock},
    };

    use super::{
        ASTRCODE_HOME_DIR_ENV, ASTRCODE_TEST_HOME_ENV, astrcode_dir, project_dir, project_dir_name,
        projects_dir, resolve_home_dir,
    };

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    struct EnvVarGuard {
        key: &'static str,
        original: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: Option<OsString>) -> Self {
            let original = std::env::var_os(key);
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
            Self { key, original }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match self.original.take() {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    #[test]
    fn resolve_home_dir_prefers_test_home_env() {
        let _lock = env_lock().lock().expect("env lock poisoned");
        let _test_home = EnvVarGuard::set(
            ASTRCODE_TEST_HOME_ENV,
            Some(OsString::from("__astrcode_test_home__")),
        );
        let _home = EnvVarGuard::set(
            ASTRCODE_HOME_DIR_ENV,
            Some(OsString::from("__astrcode_home__")),
        );

        let home = resolve_home_dir().expect("resolve home dir should succeed");
        assert_eq!(home, std::path::PathBuf::from("__astrcode_test_home__"));
    }

    #[test]
    fn resolve_home_dir_uses_home_env_when_test_home_absent() {
        let _lock = env_lock().lock().expect("env lock poisoned");
        let _test_home = EnvVarGuard::set(ASTRCODE_TEST_HOME_ENV, None);
        let _home = EnvVarGuard::set(
            ASTRCODE_HOME_DIR_ENV,
            Some(OsString::from("__astrcode_home__")),
        );

        let home = resolve_home_dir().expect("resolve home dir should succeed");
        assert_eq!(home, std::path::PathBuf::from("__astrcode_home__"));
    }

    #[test]
    fn astrcode_and_project_paths_follow_home_resolution() {
        let _lock = env_lock().lock().expect("env lock poisoned");
        let temp_home = tempfile::tempdir().expect("temp home should be created");
        let home_root = temp_home.path().join("home-root");
        let _test_home = EnvVarGuard::set(
            ASTRCODE_TEST_HOME_ENV,
            Some(home_root.as_os_str().to_os_string()),
        );
        let _home = EnvVarGuard::set(ASTRCODE_HOME_DIR_ENV, None);
        let workspace = std::path::Path::new("workspace/demo");

        let resolved_astrcode = astrcode_dir().expect("astrcode dir should resolve");
        let resolved_projects = projects_dir().expect("projects dir should resolve");
        let resolved_project = project_dir(workspace).expect("project dir should resolve");

        assert_eq!(resolved_astrcode, home_root.join(".astrcode"));
        assert_eq!(resolved_projects, resolved_astrcode.join("projects"));
        assert_eq!(
            resolved_project,
            resolved_projects.join(project_dir_name(workspace))
        );
    }
}
