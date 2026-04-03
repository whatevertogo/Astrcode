//! 配置保存工具。
//!
//! 本模块提供配置持久化功能，使用原子写入策略确保配置文件的完整性。
//!
//! # 写入策略
//!
//! 1. 序列化配置为格式化的 JSON
//! 2. 写入临时文件（`.json.tmp`）并 fsync
//! 3. 原子重命名覆盖目标文件
//!
//! Windows 平台上 `std::fs::rename` 在目标已存在时返回 `AlreadyExists`，
//! 因此采用三步替换：原文件 → `.bak` → 临时文件 → 目标文件，失败时从 `.bak` 恢复。
//! 详见 [`crate::loader::write_json_atomic`]。

use std::{fs, path::Path};

use astrcode_core::{AstrError, Result};

use crate::{loader::write_json_atomic, types::Config, validation::normalize_config};

/// 保存配置到默认路径。
///
/// 等价于 `save_config_to_path(&config_path()?, config)`。
/// 保存前会执行规范化（填充缺失字段、验证合法性）。
pub fn save_config(config: &Config) -> Result<()> {
    let path = crate::loader::config_path()?;
    save_config_to_path(&path, config)
}

/// 保存配置到指定路径。
///
/// 保存前执行规范化确保配置合法，使用原子写入策略防止写入过程中断导致文件损坏。
/// 会自动创建父目录（若不存在）。
pub fn save_config_to_path(path: &Path, config: &Config) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        AstrError::Internal(format!("config path has no parent: {}", path.display()))
    })?;
    fs::create_dir_all(parent).map_err(|e| {
        AstrError::io(
            format!("failed to create config directory for {}", parent.display()),
            e,
        )
    })?;

    let normalized = normalize_config(config.clone())?;
    write_json_atomic(path, &normalized)
}
