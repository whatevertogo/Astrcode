//! Home directory resolution for Astrcode.
//!
//! Provides a single canonical function for resolving the application home directory,
//! shared across all crates in the workspace.

use std::path::PathBuf;

use crate::{AstrError, Result};

/// Resolves the home directory for Astrcode storage.
///
/// Resolution order (unified for both test and non-test builds):
/// 1. `ASTRCODE_TEST_HOME` — test isolation (checked first so tests don't leak to real home)
/// 2. `ASTRCODE_HOME_DIR` — production override
/// 3. `dirs::home_dir()` — system default
pub fn resolve_home_dir() -> Result<PathBuf> {
    const TEST_HOME_OVERRIDE_ENV: &str = "ASTRCODE_TEST_HOME";
    const APP_HOME_OVERRIDE_ENV: &str = "ASTRCODE_HOME_DIR";

    // 1. Check ASTRCODE_TEST_HOME first (test isolation)
    if let Some(home) = std::env::var_os(TEST_HOME_OVERRIDE_ENV) {
        if !home.is_empty() {
            return Ok(PathBuf::from(home));
        }
    }

    // 2. Check ASTRCODE_HOME_DIR (production override)
    if let Some(home) = std::env::var_os(APP_HOME_OVERRIDE_ENV) {
        if !home.is_empty() {
            return Ok(PathBuf::from(home));
        }
    }

    // 3. Fall back to system home directory
    dirs::home_dir().ok_or(AstrError::HomeDirectoryNotFound)
}
