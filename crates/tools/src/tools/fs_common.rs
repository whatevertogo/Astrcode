use std::fs;
use std::path::{Component, Path, PathBuf};

use astrcode_core::{AstrError, CancelToken, Result, ToolContext};
use serde::Serialize;
use serde_json::{json, Value};

// Metadata conventions:
// - Path fields are returned as absolute path strings.
// - count/bytes/truncated/skipped_files are provided when they apply.
// - metadata is the machine-readable contract; output is display text only.
// - Structured machine data should not be embedded into output strings.

pub fn check_cancel(cancel: &CancelToken, _tool_name: &str) -> Result<()> {
    if cancel.is_cancelled() {
        return Err(AstrError::Cancelled);
    }
    Ok(())
}

pub fn resolve_path(ctx: &ToolContext, path: &Path) -> Result<PathBuf> {
    let working_dir = canonicalize_path(
        ctx.working_dir(),
        &format!(
            "failed to canonicalize working directory '{}'",
            ctx.working_dir().display()
        ),
    )?;
    let base = if path.is_absolute() {
        path.to_path_buf()
    } else {
        working_dir.join(path)
    };

    let resolved = resolve_for_boundary_check(&normalize_lexically(&base))?;
    if is_path_within_root(&resolved, &working_dir) {
        return Ok(resolved);
    }

    Err(AstrError::Validation(format!(
        "path '{}' escapes working directory '{}'",
        path.display(),
        working_dir.display()
    )))
}

pub async fn read_utf8_file(path: &Path) -> Result<String> {
    fs::read_to_string(path)
        .map_err(|e| AstrError::io(format!("failed reading file '{}'", path.display()), e))
}

pub async fn write_text_file(path: &Path, content: &str, create_dirs: bool) -> Result<usize> {
    if create_dirs {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                AstrError::io(
                    format!("failed creating parent directory '{}'", parent.display()),
                    e,
                )
            })?;
        }
    }

    fs::write(path, content.as_bytes())
        .map_err(|e| AstrError::io(format!("failed writing file '{}'", path.display()), e))?;

    Ok(content.len())
}

pub fn json_output<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(|e| AstrError::parse("failed to serialize output", e))
}

pub struct TextChangeReport {
    pub summary: String,
    pub metadata: Value,
}

pub fn build_text_change_report(
    path: &Path,
    change_type: &'static str,
    before: Option<&str>,
    after: &str,
) -> TextChangeReport {
    let diff = build_unified_diff(path, before.unwrap_or(""), after, before.is_none());
    let summary = if diff.has_changes {
        format!(
            "{change_type} {} (+{} -{})",
            path.display(),
            diff.added_lines,
            diff.removed_lines
        )
    } else {
        format!("{change_type} {} (no content changes)", path.display())
    };

    TextChangeReport {
        summary,
        metadata: json!({
            "path": path.to_string_lossy(),
            "changeType": change_type,
            "diff": {
                "patch": diff.patch,
                "addedLines": diff.added_lines,
                "removedLines": diff.removed_lines,
                "hasChanges": diff.has_changes,
                "truncated": diff.truncated,
            }
        }),
    }
}

struct UnifiedDiffReport {
    patch: String,
    added_lines: usize,
    removed_lines: usize,
    has_changes: bool,
    truncated: bool,
}

/// 手工实现的 unified diff 生成器。
///
/// ## 为什么不用外部 diff 库
///
/// 工具系统需要生成 unified diff 格式的输出给前端渲染，但只需要单 hunk（所有变更
/// 集中在一个区域），不需要标准 diff 的多 hunk 分割。因此使用简化算法：
///
/// ## 算法
///
/// 1. **前后缀匹配**: 从文件开头和结尾分别找公共行（prefix_len / suffix_len），
///    中间的就是变更区域。这是最简单的 diff 策略——不处理交叉插入/删除。
/// 2. **上下文行**: 变更区域前后各取最多 3 行作为上下文（context_start =
///    prefix_len - 3），保证 diff 可读性。
/// 3. **Hunk header 计算**: `@@ -{start},{count} +{start},{count} @@`
///    - start 使用 1-based 行号：如果有删除行则 `context_start + 1`，否则 `context_start`
///      （纯新增文件时 start 从 0 开始无意义，+1 使其从第 1 行开始）
///    - count = hunk_end - context_start（包含上下文行和变更行）
/// 4. **截断**: 超过 240 行的 diff 截断，避免前端渲染性能问题。
fn build_unified_diff(path: &Path, before: &str, after: &str, created: bool) -> UnifiedDiffReport {
    let before_lines = text_lines(before);
    let after_lines = text_lines(after);

    let mut prefix_len = 0usize;
    while prefix_len < before_lines.len()
        && prefix_len < after_lines.len()
        && before_lines[prefix_len] == after_lines[prefix_len]
    {
        prefix_len += 1;
    }

    let mut suffix_len = 0usize;
    while suffix_len < before_lines.len().saturating_sub(prefix_len)
        && suffix_len < after_lines.len().saturating_sub(prefix_len)
        && before_lines[before_lines.len() - 1 - suffix_len]
            == after_lines[after_lines.len() - 1 - suffix_len]
    {
        suffix_len += 1;
    }

    let before_change_end = before_lines.len().saturating_sub(suffix_len);
    let after_change_end = after_lines.len().saturating_sub(suffix_len);
    let has_changes = prefix_len != before_lines.len() || prefix_len != after_lines.len();
    let removed_lines = before_change_end.saturating_sub(prefix_len);
    let added_lines = after_change_end.saturating_sub(prefix_len);

    let display_path = path.display().to_string().replace('\\', "/");
    let before_label = if created {
        "/dev/null".to_string()
    } else {
        format!("a/{display_path}")
    };
    let after_label = format!("b/{display_path}");
    let mut lines = vec![format!("--- {before_label}"), format!("+++ {after_label}")];

    if !has_changes {
        lines.push(format!("@@ -1,0 +1,0 @@ {}", "no changes"));
        return UnifiedDiffReport {
            patch: lines.join("\n"),
            added_lines,
            removed_lines,
            has_changes,
            truncated: false,
        };
    }

    let context_start = prefix_len.saturating_sub(3);
    let before_hunk_end = (before_change_end + 3).min(before_lines.len());
    let after_hunk_end = (after_change_end + 3).min(after_lines.len());
    let before_hunk_start = if removed_lines == 0 {
        context_start
    } else {
        context_start + 1
    };
    let after_hunk_start = if added_lines == 0 {
        context_start
    } else {
        context_start + 1
    };

    lines.push(format!(
        "@@ -{},{} +{},{} @@",
        before_hunk_start,
        before_hunk_end.saturating_sub(context_start),
        after_hunk_start,
        after_hunk_end.saturating_sub(context_start)
    ));

    for line in &before_lines[context_start..prefix_len] {
        lines.push(format!(" {}", line));
    }

    for line in &before_lines[prefix_len..before_change_end] {
        lines.push(format!("-{}", line));
    }

    for line in &after_lines[prefix_len..after_change_end] {
        lines.push(format!("+{}", line));
    }

    for line in &before_lines[before_change_end..before_hunk_end] {
        lines.push(format!(" {}", line));
    }

    const MAX_PATCH_LINES: usize = 240;
    let truncated = lines.len() > MAX_PATCH_LINES;
    if truncated {
        lines.truncate(MAX_PATCH_LINES);
        lines.push("... diff truncated ...".to_string());
    }

    UnifiedDiffReport {
        patch: lines.join("\n"),
        added_lines,
        removed_lines,
        has_changes,
        truncated,
    }
}

fn text_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }

    let mut lines: Vec<&str> = text.lines().collect();
    if text.ends_with('\n') {
        lines.push("");
    }
    lines
}

fn normalize_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                let popped = normalized.pop();
                if !popped {
                    normalized.push(component.as_os_str());
                }
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }

    normalized
}

/// 解析路径到绝对形式，用于沙箱边界检查。
///
/// **为什么不能用 `fs::canonicalize` 直接处理**: `canonicalize` 要求路径在磁盘上存在，
/// 但 `writeFile` 和 `editFile` 经常操作尚不存在的文件/目录。此函数从路径的尾部向上
/// 找到第一个存在的祖先，对其 `canonicalize` 获取真实绝对路径，再拼回缺失的部分。
/// 例如 `/home/user/project/new_dir/new_file.txt` 中 `new_dir/` 不存在时，
/// 先 canonicalize `/home/user/project/`，再追加 `new_dir/new_file.txt`。
fn resolve_for_boundary_check(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        return canonicalize_path(
            path,
            &format!("failed to canonicalize path '{}'", path.display()),
        );
    }

    let mut missing_components = Vec::new();
    let mut current = path;
    while !current.exists() {
        let Some(name) = current.file_name() else {
            return Err(AstrError::Validation(format!(
                "path '{}' cannot be resolved under the working directory",
                path.display()
            )));
        };
        let Some(parent) = current.parent() else {
            return Err(AstrError::Validation(format!(
                "path '{}' cannot be resolved under the working directory",
                path.display()
            )));
        };
        missing_components.push(name.to_os_string());
        current = parent;
    }

    let mut resolved_parent = canonicalize_path(
        current,
        &format!("failed to canonicalize path '{}'", current.display()),
    )?;
    for component in missing_components.iter().rev() {
        resolved_parent.push(component);
    }

    Ok(normalize_lexically(&resolved_parent))
}

fn canonicalize_path(path: &Path, context: &str) -> Result<PathBuf> {
    fs::canonicalize(path)
        .map(normalize_absolute_path)
        .map_err(|e| AstrError::io(context.to_string(), e))
}

fn is_path_within_root(path: &Path, root: &Path) -> bool {
    let normalized_path = normalize_lexically(path);
    let normalized_root = normalize_lexically(root);
    normalized_path == normalized_root || normalized_path.starts_with(&normalized_root)
}

fn normalize_absolute_path(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(rendered) = path.to_str() {
            if let Some(stripped) = rendered.strip_prefix(r"\\?\UNC\") {
                return PathBuf::from(format!(r"\\{}", stripped));
            }
            if let Some(stripped) = rendered.strip_prefix(r"\\?\") {
                return PathBuf::from(stripped);
            }
        }
    }

    path
}

#[cfg(test)]
mod tests {
    use crate::test_support::test_tool_context_for;

    use super::*;

    #[test]
    fn check_cancel_returns_error_for_cancelled_token() {
        let ctx = test_tool_context_for(std::env::temp_dir());
        ctx.cancel().cancel();

        let err = check_cancel(ctx.cancel(), "grep").expect_err("cancelled token should fail");
        assert!(err.to_string().contains("cancelled"));
    }

    #[test]
    fn resolve_path_returns_absolute_normalized_path() {
        let cwd = std::env::current_dir().expect("cwd should resolve");
        let ctx = test_tool_context_for(cwd.clone());
        let resolved =
            resolve_path(&ctx, Path::new("./src/../Cargo.toml")).expect("path should resolve");

        assert!(resolved.is_absolute());
        assert_eq!(resolved, cwd.join("Cargo.toml"));
    }

    #[test]
    fn resolve_path_rejects_relative_escape_from_working_dir() {
        let parent = tempfile::tempdir().expect("tempdir should be created");
        let working_dir = parent.path().join("workspace");
        fs::create_dir_all(&working_dir).expect("workspace should be created");
        let ctx = test_tool_context_for(&working_dir);

        let err = resolve_path(&ctx, Path::new("../outside.txt"))
            .expect_err("escaping path should be rejected");

        assert!(matches!(err, AstrError::Validation(_)));
        assert!(err.to_string().contains("escapes working directory"));
    }

    #[test]
    fn resolve_path_rejects_absolute_path_outside_working_dir() {
        let working_dir = tempfile::tempdir().expect("tempdir should be created");
        let outside_dir = tempfile::tempdir().expect("tempdir should be created");
        let outside = outside_dir.path().join("outside.txt");
        fs::write(&outside, "outside").expect("outside file should be created");
        let ctx = test_tool_context_for(working_dir.path());

        let err =
            resolve_path(&ctx, &outside).expect_err("absolute path outside working dir fails");

        assert!(matches!(err, AstrError::Validation(_)));
        assert!(err.to_string().contains("escapes working directory"));
    }

    #[test]
    fn resolve_path_allows_absolute_path_inside_working_dir() {
        let working_dir = tempfile::tempdir().expect("tempdir should be created");
        let file = working_dir.path().join("notes.txt");
        fs::write(&file, "hello").expect("file should be created");
        let ctx = test_tool_context_for(working_dir.path());

        let resolved = resolve_path(&ctx, &file).expect("path should resolve");

        assert_eq!(resolved, file);
    }

    #[test]
    fn is_path_within_root_ignores_trailing_separators() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let root = temp.path().join("workspace");
        fs::create_dir_all(root.join("nested")).expect("workspace should be created");
        let root_with_separator =
            PathBuf::from(format!("{}{}", root.display(), std::path::MAIN_SEPARATOR));

        assert!(is_path_within_root(
            &root.join("nested"),
            &root_with_separator
        ));
    }

    #[tokio::test]
    async fn write_and_read_text_file_round_trip() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("note.txt");

        let written: usize = write_text_file(&file, "hello", false)
            .await
            .expect("write should succeed");
        let content = read_utf8_file(&file).await.expect("read should succeed");

        assert_eq!(written, 5);
        assert_eq!(content, "hello");
    }

    #[tokio::test]
    async fn write_text_file_creates_parent_directories() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("nested").join("dir").join("note.txt");

        write_text_file(&file, "hello", true)
            .await
            .expect("write should succeed");

        assert!(file.exists());
    }

    #[test]
    fn text_change_report_contains_patch_metadata() {
        let report = build_text_change_report(
            Path::new("src/demo.rs"),
            "updated",
            Some("old line\n"),
            "new line\n",
        );

        assert!(report.summary.contains("src/demo.rs"));
        assert_eq!(report.metadata["changeType"], json!("updated"));
        assert!(report.metadata["diff"]["patch"]
            .as_str()
            .expect("patch should exist")
            .contains("-old line"));
        assert!(report.metadata["diff"]["patch"]
            .as_str()
            .expect("patch should exist")
            .contains("+new line"));
    }
}
