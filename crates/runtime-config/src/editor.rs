//! 在系统默认编辑器中打开配置文件的工具。
//!
//! 本模块提供跨平台的文件打开功能，支持 Windows（`cmd /c start`）、
/// macOS（`open`）和 Linux（`xdg-open`）。
use std::path::Path;
use std::process::Command;

use astrcode_core::{AstrError, Result};

use crate::loader::config_path;
use crate::loader::load_config;

/// 在系统默认编辑器中打开配置文件。
///
/// 先执行一次 `load_config()` 确保配置文件已初始化（首次启动时会自动创建），
/// 然后使用平台相关的命令打开文件。
pub fn open_config_in_editor() -> Result<()> {
    let _ = load_config()?;
    let path = config_path()?;
    let open_command = platform_open_command(std::env::consts::OS, &path)?;
    Command::new(&open_command.program)
        .args(&open_command.args)
        .spawn()
        .map_err(|e| {
            AstrError::io(
                format!("failed to open config in editor: {}", path.display()),
                e,
            )
        })?;
    Ok(())
}

/// 平台特定的文件打开命令。
///
/// 封装了程序名和参数列表，用于跨平台打开配置文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OpenCommand {
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
}

/// 返回指定操作系统上打开文件的合适命令。
///
/// 支持的操作系统：
/// - `windows`：`cmd /c start "" <path>`
/// - `macos`：`open <path>`
/// - `linux`：`xdg-open <path>`
///
/// 不支持的平台返回 `UnsupportedPlatform` 错误。
pub(crate) fn platform_open_command(os: &str, path: &Path) -> Result<OpenCommand> {
    let rendered_path = path
        .to_str()
        .ok_or_else(|| {
            AstrError::Internal(format!(
                "config path is not valid utf-8: {}",
                path.display()
            ))
        })?
        .to_string();

    let command = match os {
        "windows" => OpenCommand {
            program: "cmd".to_string(),
            args: vec![
                "/c".to_string(),
                "start".to_string(),
                String::new(),
                rendered_path,
            ],
        },
        "macos" => OpenCommand {
            program: "open".to_string(),
            args: vec![rendered_path],
        },
        "linux" => OpenCommand {
            program: "xdg-open".to_string(),
            args: vec![rendered_path],
        },
        other => return Err(AstrError::UnsupportedPlatform(other.to_string())),
    };

    Ok(command)
}
