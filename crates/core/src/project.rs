//! # 项目目录解析
//!
//! 负责将工作目录映射到 `~/.astrcode/projects/<slug>` 下的持久化目录。
//!
//! ## 设计动机
//!
//! 不同操作系统的路径格式差异很大（Windows 盘符、UNC 路径、Unix 绝对路径等），
//! 此模块提供统一的 kebab-case slug 生成策略，确保：
//! 1. 同一项目不同大小写路径映射到同一目录（Windows 下转小写）
//! 2. 目录名保持人类可读性
//! 3. 超长路径通过稳定 hash 截断，避免文件系统限制

use std::path::{Component, Path, PathBuf, Prefix};

use uuid::Uuid;

use crate::{home::resolve_home_dir, Result};

/// 项目目录名称的最大长度限制。
///
/// 超过此长度的路径名会被截断并追加稳定 hash。
const MAX_PROJECT_DIR_NAME_LEN: usize = 96;

/// 返回 `~/.astrcode` 根目录。
///
/// 所有用户级和项目级持久化数据都应从这里派生，避免各 crate 自己拼装路径。
pub fn astrcode_dir() -> Result<PathBuf> {
    Ok(resolve_home_dir()?.join(".astrcode"))
}

/// 返回项目级持久化根目录 `~/.astrcode/projects`。
pub fn projects_dir() -> Result<PathBuf> {
    Ok(astrcode_dir()?.join("projects"))
}

/// 返回工作目录对应的项目目录名称。
///
/// ## 生成策略
///
/// 1. 规范化路径（Windows 转小写，确保同一项目不同大小写路径映射到同一目录）
/// 2. 转换为 kebab-case slug
/// 3. 如果超过 `MAX_PROJECT_DIR_NAME_LEN`，截断并追加稳定 hash
///
/// ## 设计动机
///
/// 目录名优先保持人类可读，例如 `D:\project1` 会映射为 `D-project1`；
/// 当路径过长时，再追加稳定 hash 截断，既保留可读性，也避免路径无限增长。
pub fn project_dir_name(working_dir: &Path) -> String {
    let canonical =
        std::fs::canonicalize(working_dir).unwrap_or_else(|_| working_dir.to_path_buf());
    let normalized = normalize_project_identity(&canonical);
    let mut slug = components_to_slug(&normalized);
    if slug.is_empty() {
        slug = "default-project".to_string();
    }

    if slug.len() <= MAX_PROJECT_DIR_NAME_LEN {
        return slug;
    }

    let stable_hash = stable_project_hash(&normalized);
    let keep_len = MAX_PROJECT_DIR_NAME_LEN.saturating_sub(stable_hash.len() + 1);
    slug.truncate(keep_len);
    slug = slug.trim_end_matches(['-', '.', ' ']).to_string();
    if slug.is_empty() {
        stable_hash
    } else {
        format!("{slug}-{stable_hash}")
    }
}

/// 返回工作目录的项目级持久化目录 `~/.astrcode/projects/<project>`。
pub fn project_dir(working_dir: &Path) -> Result<PathBuf> {
    Ok(projects_dir()?.join(project_dir_name(working_dir)))
}

/// 规范化项目标识路径
///
/// Windows 下将路径转为小写，确保 `D:\Project` 和 `D:\project` 映射到同一项目目录。
/// Unix 下保持原样（文件系统区分大小写）。
fn normalize_project_identity(path: &Path) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(path.to_string_lossy().to_ascii_lowercase())
    } else {
        path.to_path_buf()
    }
}

/// 将路径组件转换为 kebab-case slug
///
/// ## 处理规则
///
/// - **Prefix**（Windows 盘符）: `D:` → `D`，UNC 路径 → `server-share`
/// - **RootDir**: 根目录 → `root`（Windows 下后续会移除）
/// - **Normal**: 清理非法字符，连续非法字符合并为单个 `-`
/// - **CurDir/ParentDir**: 忽略（`.` 和 `..`）
fn components_to_slug(path: &Path) -> String {
    let mut segments = Vec::new();

    for component in path.components() {
        match component {
            Component::Prefix(prefix) => {
                if let Some(segment) = prefix_to_segment(prefix.kind()) {
                    segments.push(segment);
                }
            }
            Component::RootDir => {
                if segments.is_empty() {
                    segments.push("root".to_string());
                }
            }
            Component::Normal(segment) => {
                let sanitized = sanitize_component(&segment.to_string_lossy());
                if !sanitized.is_empty() {
                    segments.push(sanitized);
                }
            }
            Component::CurDir | Component::ParentDir => {}
        }
    }

    // Windows 下移除开头的 "root"（因为盘符已经提供了足够的标识）
    if cfg!(windows) && segments.first().is_some_and(|segment| segment == "root") {
        segments.remove(0);
    }

    segments.join("-")
}

fn prefix_to_segment(prefix: Prefix<'_>) -> Option<String> {
    match prefix {
        Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
            Some((letter as char).to_ascii_uppercase().to_string())
        }
        Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
            let server = sanitize_component(&server.to_string_lossy());
            let share = sanitize_component(&share.to_string_lossy());
            let joined = [server, share]
                .into_iter()
                .filter(|segment| !segment.is_empty())
                .collect::<Vec<_>>()
                .join("-");
            if joined.is_empty() {
                None
            } else {
                Some(joined)
            }
        }
        Prefix::DeviceNS(device) => {
            let device = sanitize_component(&device.to_string_lossy());
            if device.is_empty() {
                None
            } else {
                Some(device)
            }
        }
        Prefix::Verbatim(value) => {
            let value = sanitize_component(&value.to_string_lossy());
            if value.is_empty() {
                None
            } else {
                Some(value)
            }
        }
    }
}

fn sanitize_component(value: &str) -> String {
    let mut sanitized = String::with_capacity(value.len());
    let mut last_was_separator = false;

    for ch in value.chars() {
        let is_valid = ch.is_alphanumeric() || ch == '-' || ch == '_' || ch == '.';
        if is_valid {
            sanitized.push(ch);
            last_was_separator = false;
            continue;
        }

        if !last_was_separator {
            sanitized.push('-');
            last_was_separator = true;
        }
    }

    sanitized
        .trim_matches(['-', '.', ' '])
        .chars()
        .collect::<String>()
}

fn stable_project_hash(path: &Path) -> String {
    let source = path.to_string_lossy();
    Uuid::new_v5(&Uuid::NAMESPACE_URL, source.as_bytes())
        .simple()
        .to_string()[..8]
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_dir_name_normalizes_invalid_filename_characters() {
        let path = Path::new("demo folder/feature:alpha");
        assert_eq!(project_dir_name(path), "demo-folder-feature-alpha");
    }

    #[test]
    fn project_dir_name_truncates_with_stable_hash_when_needed() {
        let long = format!("root/{}", "very-long-segment-".repeat(16));
        let name = project_dir_name(Path::new(&long));
        assert!(name.len() <= MAX_PROJECT_DIR_NAME_LEN);
        assert!(
            name.chars().rev().take(8).all(|ch| ch.is_ascii_hexdigit()),
            "truncated project dirs should end with a short stable hash"
        );
    }
}
