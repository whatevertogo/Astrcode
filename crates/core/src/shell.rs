//! # Shell 检测与解析
//!
//! 自动检测当前平台的默认 Shell，并支持用户指定的 Shell 覆盖。
//!
//! ## 支持的 Shell 类型
//!
//! - **PowerShell**: `pwsh` / `powershell`
//! - **Cmd**: `cmd`
//! - **Posix**: `bash` / `zsh` / `sh`
//! - **Wsl**: Windows WSL bash
//!
//! ## 检测策略（Windows 优先级）
//!
//! 1. `$env:SHELL` 环境变量（支持 Git Bash / WSL 环境检测）
//! 2. Git Bash 磁盘路径探测
//! 3. `wsl.exe` / `wsl` 命令探测
//! 4. `pwsh` / `powershell` 兜底

#[cfg(windows)]
use std::path::PathBuf;
use std::{env, path::Path, process::Command, sync::OnceLock};

use crate::{AstrError, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellFamily {
    PowerShell,
    Cmd,
    Posix,
    Wsl,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedShell {
    pub program: String,
    pub family: ShellFamily,
    pub label: String,
}

pub fn resolve_shell(shell_override: Option<&str>) -> Result<ResolvedShell> {
    match shell_override {
        Some(program) => resolve_shell_override(program),
        None => Ok(resolve_default_shell().clone()),
    }
}

pub fn default_shell_label() -> String {
    resolve_default_shell().label.clone()
}

pub fn detect_shell_family(shell: &str) -> Option<ShellFamily> {
    let file_name = Path::new(shell)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(shell);
    let normalized = file_name.trim_end_matches(".exe").to_ascii_lowercase();

    match normalized.as_str() {
        "pwsh" | "powershell" => Some(ShellFamily::PowerShell),
        "cmd" => Some(ShellFamily::Cmd),
        "sh" | "bash" | "zsh" => Some(ShellFamily::Posix),
        "wsl" => Some(ShellFamily::Wsl),
        _ => None,
    }
}

fn resolve_shell_override(program: &str) -> Result<ResolvedShell> {
    let family = detect_shell_family(program).ok_or_else(|| unsupported_shell_error(program))?;
    Ok(ResolvedShell {
        label: shell_label(program, family),
        family,
        program: program.to_string(),
    })
}

fn resolve_default_shell() -> &'static ResolvedShell {
    static SHELL: OnceLock<ResolvedShell> = OnceLock::new();
    SHELL.get_or_init(resolve_default_shell_uncached)
}

#[cfg(windows)]
fn resolve_default_shell_uncached() -> ResolvedShell {
    if let Some(shell) = resolve_windows_env_shell() {
        return shell;
    }

    if let Some(shell) = resolve_windows_git_bash_fallback() {
        return shell;
    }

    if command_exists("wsl.exe") {
        return ResolvedShell {
            program: "wsl.exe".to_string(),
            family: ShellFamily::Wsl,
            label: "wsl-bash".to_string(),
        };
    }

    if command_exists("wsl") {
        return ResolvedShell {
            program: "wsl".to_string(),
            family: ShellFamily::Wsl,
            label: "wsl-bash".to_string(),
        };
    }

    if command_exists("pwsh") {
        return ResolvedShell {
            program: "pwsh".to_string(),
            family: ShellFamily::PowerShell,
            label: "pwsh".to_string(),
        };
    }

    ResolvedShell {
        program: "powershell".to_string(),
        family: ShellFamily::PowerShell,
        label: "powershell".to_string(),
    }
}

#[cfg(not(windows))]
fn resolve_default_shell_uncached() -> ResolvedShell {
    if let Some(shell_env) = env::var_os("SHELL")
        .and_then(|value| value.into_string().ok())
        .and_then(resolve_unix_env_shell)
    {
        return shell_env;
    }

    if path_exists(Path::new("/bin/bash")) {
        return ResolvedShell {
            program: "/bin/bash".to_string(),
            family: ShellFamily::Posix,
            label: "bash".to_string(),
        };
    }

    if command_exists("bash") {
        return ResolvedShell {
            program: "bash".to_string(),
            family: ShellFamily::Posix,
            label: "bash".to_string(),
        };
    }

    if path_exists(Path::new("/bin/sh")) {
        return ResolvedShell {
            program: "/bin/sh".to_string(),
            family: ShellFamily::Posix,
            label: "sh".to_string(),
        };
    }

    ResolvedShell {
        program: "sh".to_string(),
        family: ShellFamily::Posix,
        label: "sh".to_string(),
    }
}

#[cfg(windows)]
fn resolve_windows_env_shell() -> Option<ResolvedShell> {
    let shell_env = env::var_os("SHELL")
        .and_then(|value| value.into_string().ok())
        .filter(|value| !value.trim().is_empty());

    if looks_like_windows_git_bash_env() {
        if let Some(program) = shell_env.as_deref().and_then(resolve_windows_posix_program) {
            return Some(ResolvedShell {
                label: "git-bash".to_string(),
                family: ShellFamily::Posix,
                program,
            });
        }

        if command_exists("bash") {
            return Some(ResolvedShell {
                program: "bash".to_string(),
                family: ShellFamily::Posix,
                label: "git-bash".to_string(),
            });
        }
    }

    if looks_like_windows_wsl_env() {
        if command_exists("wsl.exe") {
            return Some(ResolvedShell {
                program: "wsl.exe".to_string(),
                family: ShellFamily::Wsl,
                label: "wsl-bash".to_string(),
            });
        }
        if command_exists("wsl") {
            return Some(ResolvedShell {
                program: "wsl".to_string(),
                family: ShellFamily::Wsl,
                label: "wsl-bash".to_string(),
            });
        }
    }

    None
}

#[cfg(windows)]
fn resolve_windows_git_bash_fallback() -> Option<ResolvedShell> {
    for candidate in windows_git_bash_candidates() {
        if path_exists(&candidate) {
            return Some(ResolvedShell {
                program: candidate.to_string_lossy().into_owned(),
                family: ShellFamily::Posix,
                label: "git-bash".to_string(),
            });
        }
    }

    None
}

#[cfg(not(windows))]
fn resolve_unix_env_shell(shell_env: String) -> Option<ResolvedShell> {
    let family = detect_shell_family(&shell_env)?;
    let label = shell_label(&shell_env, family);
    if is_shell_program_usable(&shell_env) {
        return Some(ResolvedShell {
            program: shell_env,
            family,
            label,
        });
    }

    None
}

#[cfg(windows)]
fn resolve_windows_posix_program(shell_env: &str) -> Option<String> {
    if !matches!(detect_shell_family(shell_env), Some(ShellFamily::Posix)) {
        return None;
    }

    if is_windows_native_path(shell_env) && path_exists(Path::new(shell_env)) {
        return Some(shell_env.to_string());
    }

    if command_exists("bash") {
        return Some("bash".to_string());
    }

    None
}

fn shell_label(program: &str, family: ShellFamily) -> String {
    let file_name = Path::new(program)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(program);
    let normalized = file_name.trim_end_matches(".exe").to_ascii_lowercase();

    match family {
        ShellFamily::Wsl => "wsl-bash".to_string(),
        ShellFamily::PowerShell => match normalized.as_str() {
            "pwsh" => "pwsh".to_string(),
            _ => "powershell".to_string(),
        },
        ShellFamily::Cmd => "cmd".to_string(),
        ShellFamily::Posix => {
            #[cfg(windows)]
            {
                match normalized.as_str() {
                    "zsh" => "zsh".to_string(),
                    _ => "git-bash".to_string(),
                }
            }

            #[cfg(not(windows))]
            {
                match normalized.as_str() {
                    "bash" => "bash".to_string(),
                    "zsh" => "zsh".to_string(),
                    _ => "sh".to_string(),
                }
            }
        },
    }
}

fn unsupported_shell_error(shell: &str) -> AstrError {
    AstrError::Validation(format!(
        "unsupported shell override '{}'; supported families are pwsh/powershell, cmd, wsl, and \
         sh/bash/zsh",
        shell
    ))
}

#[cfg(not(windows))]
fn is_shell_program_usable(program: &str) -> bool {
    let path = Path::new(program);
    if path.components().count() > 1 || path.is_absolute() {
        return path_exists(path);
    }

    command_exists(program)
}

fn command_exists(program: &str) -> bool {
    Command::new(program)
        .arg(version_probe_arg(program))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

fn version_probe_arg(program: &str) -> &'static str {
    match detect_shell_family(program) {
        Some(ShellFamily::Cmd) => "/?",
        Some(ShellFamily::PowerShell) => "-Version",
        Some(ShellFamily::Wsl) | Some(ShellFamily::Posix) | None => "--version",
    }
}

fn path_exists(path: &Path) -> bool {
    path.is_file()
}

#[cfg(windows)]
fn looks_like_windows_git_bash_env() -> bool {
    ["MSYSTEM", "MINGW_PREFIX", "MSYSTEM_CHOST", "CHERE_INVOKING"]
        .into_iter()
        .any(has_non_empty_env)
        || env_contains("OSTYPE", "msys")
        || env_contains("OSTYPE", "cygwin")
}

#[cfg(windows)]
fn looks_like_windows_wsl_env() -> bool {
    ["WSL_DISTRO_NAME", "WSL_INTEROP"]
        .into_iter()
        .any(has_non_empty_env)
}

#[cfg(windows)]
fn windows_git_bash_candidates() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for key in ["ProgramFiles", "ProgramFiles(x86)", "LocalAppData"] {
        if let Some(root) = env::var_os(key) {
            roots.push(PathBuf::from(root));
        }
    }

    let mut candidates = Vec::new();
    for root in roots {
        if root
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("Programs"))
        {
            candidates.push(root.join("Git").join("bin").join("bash.exe"));
            candidates.push(root.join("Git").join("usr").join("bin").join("bash.exe"));
            continue;
        }

        candidates.push(root.join("Git").join("bin").join("bash.exe"));
        candidates.push(root.join("Git").join("usr").join("bin").join("bash.exe"));
        candidates.push(
            root.join("Programs")
                .join("Git")
                .join("bin")
                .join("bash.exe"),
        );
        candidates.push(
            root.join("Programs")
                .join("Git")
                .join("usr")
                .join("bin")
                .join("bash.exe"),
        );
    }

    candidates
}

#[cfg(windows)]
fn is_windows_native_path(program: &str) -> bool {
    program.contains('\\')
        || Path::new(program).is_absolute()
        || program
            .as_bytes()
            .get(1)
            .is_some_and(|value| *value == b':')
}

#[cfg(windows)]
fn has_non_empty_env(key: &str) -> bool {
    env::var_os(key).is_some_and(|value| !value.is_empty())
}

#[cfg(windows)]
fn env_contains(key: &str, needle: &str) -> bool {
    env::var_os(key)
        .and_then(|value| value.into_string().ok())
        .is_some_and(|value| value.to_ascii_lowercase().contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_supported_shell_families() {
        assert_eq!(detect_shell_family("pwsh"), Some(ShellFamily::PowerShell));
        assert_eq!(
            detect_shell_family("powershell.exe"),
            Some(ShellFamily::PowerShell)
        );
        assert_eq!(detect_shell_family("cmd"), Some(ShellFamily::Cmd));
        assert_eq!(detect_shell_family("/bin/bash"), Some(ShellFamily::Posix));
        assert_eq!(detect_shell_family("wsl.exe"), Some(ShellFamily::Wsl));
    }

    #[test]
    fn rejects_unknown_shell_override() {
        let err = resolve_shell(Some("fish")).expect_err("fish should be rejected");
        assert!(matches!(err, AstrError::Validation(_)));
    }

    #[test]
    fn override_shell_uses_stable_display_label() {
        let shell = resolve_shell(Some("pwsh")).expect("pwsh should resolve");
        assert_eq!(shell.label, "pwsh");
        assert_eq!(shell.family, ShellFamily::PowerShell);
    }
}
