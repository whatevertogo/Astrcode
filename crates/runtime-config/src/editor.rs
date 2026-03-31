//! Utilities for opening the configuration file in the system's default editor.

use std::path::Path;
use std::process::Command;

use astrcode_core::{AstrError, Result};

use crate::loader::config_path;
use crate::loader::load_config;

/// Opens the configuration file in the system's default editor.
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

/// Represents a platform-specific command to open a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OpenCommand {
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
}

/// Returns the appropriate command to open a file on the given OS.
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
