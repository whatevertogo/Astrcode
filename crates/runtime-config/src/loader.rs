//! 配置加载工具。
//!
//! 本模块负责从文件系统读取和初始化 Astrcode 配置。
//!
//! # 加载流程
//!
//! 1. [`config_path`] 确定默认配置文件路径（`~/.astrcode/config.json`）
//! 2. [`load_config_from_path`] 读取 JSON 并执行规范化/验证
//! 3. 若文件不存在，创建包含默认值的配置文件并输出提示到 stdout
//! 4. [`load_resolved_config`] 可选地加载项目 overlay 并合并到用户配置
//!
//! # 首次启动行为
//!
//! `load_config()` 是此 crate 中唯一会向 stdout 打印的函数，仅在配置文件不存在时
//! 触发一次，用于引导用户填写 API key。

use std::fs;
use std::path::{Path, PathBuf};

use astrcode_core::project::project_dir;
use astrcode_core::{AstrError, Result};

use crate::types::{Config, ConfigOverlay};
use crate::validation::normalize_config;

/// 返回默认配置文件路径。
///
/// 路径为 `~/.astrcode/config.json`，其中 `~` 通过 `astrcode_core::home::resolve_home_dir()` 解析。
/// 可通过 `ASTRCODE_HOME_DIR_ENV` 环境变量覆盖 home 目录。
pub fn config_path() -> Result<PathBuf> {
    let home = astrcode_core::home::resolve_home_dir()?;
    Ok(home.join(".astrcode").join("config.json"))
}

/// 从默认路径加载配置。
///
/// 等价于 `load_config_from_path(&config_path()?)`。
pub fn load_config() -> Result<Config> {
    let path = config_path()?;
    load_config_from_path(&path)
}

/// 加载指定工作目录的有效配置（用户配置 + 可选项目 overlay）。
///
/// 加载流程：
/// 1. 加载用户级配置 `~/.astrcode/config.json`
/// 2. 若提供了工作目录，尝试加载项目 overlay `<project>/.astrcode/config.json`
/// 3. 合并 overlay（仅覆盖显式设置的字段）
/// 4. 执行规范化与验证
///
/// 这是 HTTP 请求入口应使用的函数，确保每个请求都获得考虑了项目上下文的完整配置。
pub fn load_resolved_config(working_dir: Option<&Path>) -> Result<Config> {
    let mut config = load_config()?;
    if let Some(working_dir) = working_dir {
        let project_path = project_overlay_path(working_dir)?;
        if let Some(overlay) = load_config_overlay_from_path(&project_path)? {
            // Overlay 使用 Option 字段，避免 project 文件的 serde 默认值误覆盖 user 配置。
            config = apply_overlay(config, overlay);
        }
    }
    normalize_config(config)
}

/// 从指定路径加载配置。
///
/// 若文件不存在，会自动创建父目录并写入默认配置。这是首次启动时初始化配置文件的入口。
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

        // 首次启动时输出提示到 stdout，引导用户填写 API key。
        // 这是 load_config 唯一的 stdout 副作用，仅在配置文件不存在时触发一次。
        println!("Config created at {}，请填写 apiKey", path.display());
        return normalize_config(default_cfg);
    }

    let config = read_json_from_path::<Config>(path)?;
    normalize_config(config)
        .map_err(|e| e.context(format!("failed to validate config at {}", path.display())))
}

/// 加载项目 overlay 配置（文件存在时）。
///
/// 若文件不存在返回 `None`，不报错。这使得项目 overlay 是完全可选的。
pub fn load_config_overlay_from_path(path: &Path) -> Result<Option<ConfigOverlay>> {
    if !path.exists() {
        return Ok(None);
    }
    read_json_from_path(path).map(Some)
}

/// 返回指定工作目录的项目 overlay 路径。
///
/// 路径为 `<project>/.astrcode/config.json`，其中 `<project>` 通过
/// `astrcode_core::project::project_dir(working_dir)` 解析。
pub fn project_overlay_path(working_dir: &Path) -> Result<PathBuf> {
    Ok(project_dir(working_dir)?.join("config.json"))
}

fn apply_overlay(mut base: Config, overlay: ConfigOverlay) -> Config {
    if let Some(active_profile) = overlay.active_profile {
        base.active_profile = active_profile;
    }
    if let Some(active_model) = overlay.active_model {
        base.active_model = active_model;
    }
    if let Some(profiles) = overlay.profiles {
        base.profiles = profiles;
    }
    base
}

fn read_json_from_path<T>(path: &Path) -> Result<T>
where
    T: serde::de::DeserializeOwned,
{
    let raw = fs::read_to_string(path)
        .map_err(|e| AstrError::io(format!("failed to read config at {}", path.display()), e))?;
    serde_json::from_str::<T>(&raw)
        .map_err(|e| AstrError::parse(format!("failed to parse config at {}", path.display()), e))
}

/// Writes JSON atomically via a temp file and rename.
///
/// 写入策略：先写临时文件 → fsync → 原子重命名覆盖目标。在大多数 Unix 上
/// `rename` 天然支持原子替换，但在 Windows 上 `std::fs::rename` 在目标已存在
/// 时返回 `AlreadyExists`，因此需要三步替换：
/// 1. 将原始文件重命名为 `.bak` 备份
/// 2. 将临时文件重命名为目标文件
/// 3. 若步骤 2 失败，从 `.bak` 恢复原始文件；临时文件故意保留以供手动恢复
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
