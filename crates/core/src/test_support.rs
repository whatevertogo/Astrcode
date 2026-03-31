use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

use tempfile::TempDir;

pub const TEST_HOME_ENV: &str = "ASTRCODE_TEST_HOME";

pub fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub fn test_home_dir() -> Option<PathBuf> {
    std::env::var_os(TEST_HOME_ENV).map(PathBuf::from)
}

pub struct TestEnvGuard {
    _lock: MutexGuard<'static, ()>,
    _temp_home: TempDir,
    previous_dir: PathBuf,
    previous_home: Option<std::ffi::OsString>,
    previous_userprofile: Option<std::ffi::OsString>,
    previous_test_home: Option<std::ffi::OsString>,
}

impl Default for TestEnvGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl TestEnvGuard {
    pub fn new() -> Self {
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

    pub fn home_dir(&self) -> &Path {
        self._temp_home.path()
    }

    pub fn set_current_dir<P: AsRef<Path>>(&self, path: P) {
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
