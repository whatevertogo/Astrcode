use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow};
use astrcode_core::env::ASTRCODE_HOME_DIR_ENV;

pub fn resolve_home_dir() -> Result<PathBuf> {
    // Keep the desktop shell aligned with the workspace env catalog in
    // `runtime-config/constants.rs`, which re-exports this core constant.
    if let Some(home) = std::env::var_os(ASTRCODE_HOME_DIR_ENV) {
        if !home.is_empty() {
            return Ok(PathBuf::from(home));
        }
    }
    dirs::home_dir().ok_or_else(|| anyhow!("unable to resolve home directory"))
}

pub fn default_config_path() -> Result<PathBuf> {
    Ok(resolve_home_dir()?.join(".astrcode").join("config.json"))
}

/// 获取 Astrcode 根目录 (~/.astrcode/)
pub fn astrcode_root_dir() -> Result<PathBuf> {
    Ok(resolve_home_dir()?.join(".astrcode"))
}

pub fn runtime_dir() -> Result<PathBuf> {
    Ok(astrcode_root_dir()?
        .join("runtime")
        .join(desktop_runtime_scope()?))
}

/// 运行期 sidecar 会先复制到用户目录下的独立副本，再从该副本启动，
/// 这样构建产物可以被后续编译安全覆盖，不会被运行中的 Windows 进程锁住。
pub fn runtime_sidecar_dir() -> Result<PathBuf> {
    Ok(runtime_dir()?.join("sidecars"))
}

pub fn desktop_instance_lock_path() -> Result<PathBuf> {
    Ok(runtime_dir()?.join("desktop-instance.lock"))
}

pub fn desktop_instance_info_path() -> Result<PathBuf> {
    Ok(runtime_dir()?.join("desktop-instance.json"))
}

fn desktop_runtime_scope() -> Result<String> {
    let current_exe =
        std::env::current_exe().context("failed to resolve current desktop executable path")?;
    Ok(desktop_runtime_scope_for_path(&current_exe))
}

fn desktop_runtime_scope_for_path(path: &Path) -> String {
    let normalized = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let normalized_text = normalized.to_string_lossy().replace('\\', "/");
    let channel = infer_desktop_runtime_channel(&normalized_text);

    let mut hasher = DefaultHasher::new();
    normalized_text.hash(&mut hasher);
    let fingerprint = hasher.finish();

    // Why: debug/release/已安装包共用 `~/.astrcode/runtime` 会让单实例锁互相抢占，
    // 最终出现“明明启动了新包，却只是把旧窗口拉到前台”的错觉。
    // 这里把运行时目录按可执行文件路径做稳定分桶，既保留单安装单实例，
    // 也避免开发构建和安装包彼此劫持。
    format!("{channel}-{fingerprint:016x}")
}

fn infer_desktop_runtime_channel(normalized_exe_path: &str) -> &'static str {
    let lower = normalized_exe_path.to_ascii_lowercase();
    if lower.contains("/target/debug/") {
        "desktop-debug"
    } else if lower.contains("/target/release/") {
        "desktop-release"
    } else {
        "desktop-app"
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{desktop_runtime_scope_for_path, infer_desktop_runtime_channel};

    #[test]
    fn runtime_scope_distinguishes_debug_release_and_packaged_channels() {
        assert_eq!(
            infer_desktop_runtime_channel("d:/repo/target/debug/astrcode.exe"),
            "desktop-debug"
        );
        assert_eq!(
            infer_desktop_runtime_channel("d:/repo/target/release/astrcode.exe"),
            "desktop-release"
        );
        assert_eq!(
            infer_desktop_runtime_channel(
                "c:/users/demo/appdata/local/programs/astrcode/astrcode.exe"
            ),
            "desktop-app"
        );
    }

    #[test]
    fn runtime_scope_hashes_executable_path() {
        let debug_scope =
            desktop_runtime_scope_for_path(Path::new(r"D:\repo\target\debug\astrcode.exe"));
        let release_scope =
            desktop_runtime_scope_for_path(Path::new(r"D:\repo\target\release\astrcode.exe"));

        assert_ne!(debug_scope, release_scope);
        assert!(debug_scope.starts_with("desktop-debug-"));
        assert!(release_scope.starts_with("desktop-release-"));
    }
}
