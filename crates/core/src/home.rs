//! Home directory resolution for Astrcode.
//!
//! Provides a single canonical function for resolving the application home directory,
//! shared across all crates in the workspace.

use std::path::PathBuf;

use crate::{AstrError, Result};

/// 环境变量名，用于覆盖 Astrcode 的生产 home 目录
pub const ASTRCODE_HOME_DIR_ENV: &str = "ASTRCODE_HOME_DIR";

/// 环境变量名，用于覆盖 Astrcode 的测试 home 目录（仅用于测试隔离）
pub const ASTRCODE_TEST_HOME_ENV: &str = "ASTRCODE_TEST_HOME";

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
