//! # Home 目录解析
//!
//! 提供 Astrcode 应用主目录的统一解析入口，供整个 workspace 的所有 crate 共享使用。
//!
//! ## 解析优先级
//!
//! 1. `ASTRCODE_TEST_HOME` — 测试隔离环境变量，优先级最高
//! 2. `ASTRCODE_HOME_DIR` — 生产环境覆盖变量
//! 3. 系统默认 home 目录下的 `.astrcode` 文件夹

use std::path::PathBuf;

pub use crate::env::{ASTRCODE_HOME_DIR_ENV, ASTRCODE_TEST_HOME_ENV};
use crate::{AstrError, Result};

/// Resolves the home directory for Astrcode storage.
///
/// Resolution order (unified for both test and non-test builds):
/// 1. `ASTRCODE_TEST_HOME` — test isolation (checked first so tests don't leak to real home)
/// 2. `ASTRCODE_HOME_DIR` — production override
/// 3. `dirs::home_dir()` — system default
pub fn resolve_home_dir() -> Result<PathBuf> {
    // 1. Check ASTRCODE_TEST_HOME first (test isolation)
    if let Some(home) = std::env::var_os(ASTRCODE_TEST_HOME_ENV) {
        if !home.is_empty() {
            return Ok(PathBuf::from(home));
        }
    }

    // 2. Check ASTRCODE_HOME_DIR (production override)
    if let Some(home) = std::env::var_os(ASTRCODE_HOME_DIR_ENV) {
        if !home.is_empty() {
            return Ok(PathBuf::from(home));
        }
    }

    // 3. Fall back to system home directory
    dirs::home_dir().ok_or(AstrError::HomeDirectoryNotFound)
}
