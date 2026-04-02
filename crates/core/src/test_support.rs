//! # 测试支持工具
//!
//! 提供测试隔离所需的环境管理工具。
//!
//! ## 核心组件
//!
//! - **TestEnvGuard**: RAII 风格的环境变量守卫，测试结束后自动恢复
//! - **env_lock**: 全局互斥锁，确保测试间环境变量操作串行化
//!
//! ## 为什么是 pub mod 而非 #[cfg(test)]？
//!
//! 其他 crate（runtime、runtime-config、runtime-prompt）的测试代码通过
//! `astrcode_core::test_support::TestEnvGuard` 导入此模块。
//! Rust 不支持跨 crate 的 `#[cfg(test)]` 导出，所以只能保持 pub。
//! tempfile 依赖也因此无法移到 `[dev-dependencies]`。
//!
//! ## 使用方式
//!
//! ```ignore
//! let guard = TestEnvGuard::new();
//! // 此时 ASTRCODE_TEST_HOME 指向临时目录
//! // 测试代码...
//! // guard drop 时自动恢复原始环境变量
//! ```

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

use tempfile::TempDir;

pub use crate::env::ASTRCODE_TEST_HOME_ENV as TEST_HOME_ENV;

/// 全局环境变量互斥锁
///
/// 确保多个测试不会并发修改环境变量导致竞态条件。
/// 使用 `OnceLock` 延迟初始化，避免静态初始化顺序问题。
pub fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// 获取当前测试 home 目录（如果已设置）
pub fn test_home_dir() -> Option<PathBuf> {
    std::env::var_os(TEST_HOME_ENV).map(PathBuf::from)
}

/// 测试环境守卫
///
/// RAII 风格的环境变量管理，确保测试结束后自动恢复原始环境。
///
/// ## 工作原理
///
/// 1. 获取全局互斥锁（防止并发测试干扰）
/// 2. 创建临时目录作为测试 home
/// 3. 保存原始 `HOME`/`USERPROFILE`/`ASTRCODE_TEST_HOME`
/// 4. 设置 `ASTRCODE_TEST_HOME` 指向临时目录
/// 5. Drop 时恢复所有原始环境变量和工作目录
///
/// ## 跨平台处理
///
/// - Windows: 设置 `USERPROFILE`，移除 `HOME`
/// - Unix: 设置 `HOME`，移除 `USERPROFILE`
///
/// 这样确保 `dirs::home_dir()` 在任何平台都返回临时目录。
pub struct TestEnvGuard {
    /// 全局互斥锁守卫（防止并发测试干扰）
    _lock: MutexGuard<'static, ()>,
    /// 临时 home 目录（drop 时自动删除）
    _temp_home: TempDir,
    /// 原始工作目录
    previous_dir: PathBuf,
    /// 原始 HOME 环境变量
    previous_home: Option<std::ffi::OsString>,
    /// 原始 USERPROFILE 环境变量
    previous_userprofile: Option<std::ffi::OsString>,
    /// 原始 ASTRCODE_TEST_HOME 环境变量
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
