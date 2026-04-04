//! # 文件系统公共工具
//!
//! 提供所有文件工具共享的基础设施：
//!
//! - **路径沙箱**: `resolve_path` 确保所有路径操作不逃逸工作目录
//! - **取消检查**: `check_cancel` 在长操作的关键节点检查用户取消
//! - **文件 I/O**: `read_utf8_file` / `write_text_file` 统一 UTF-8 读写
//! - **Diff 生成**: `build_text_change_report` 手工实现 unified diff
//! - **JSON 序列化**: `json_output` 统一工具输出的 JSON 编码
//!
//! ## Metadata 约定
//!
//! - 路径字段统一返回绝对路径字符串
//! - `count`/`bytes`/`truncated`/`skipped_files` 在适用时提供
//! - `metadata` 是机器可读的契约；`output` 仅供展示
//! - 结构化机器数据不应嵌入到 `output` 字符串中

use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use astrcode_core::{AstrError, CancelToken, Result, ToolContext};
use serde::Serialize;
use serde_json::{Value, json};

/// 检查取消标记，如果已取消则返回 `AstrError::Cancelled`。
///
/// 在长操作（遍历目录、逐行搜索、大文件读取）的关键节点调用，
/// 确保用户取消能快速响应。
pub fn check_cancel(cancel: &CancelToken) -> Result<()> {
    if cancel.is_cancelled() {
        return Err(AstrError::Cancelled);
    }
    Ok(())
}

/// 将路径解析为工作目录内的绝对路径，拒绝逃逸路径。
///
/// **为什么使用 `resolve_for_boundary_check` 而非 `fs::canonicalize`**:
/// canonicalize 要求路径在磁盘上存在，但 writeFile/editFile 经常操作
/// 尚不存在的文件。resolve_for_boundary_check 从路径尾部向上找到第一个
/// 存在的祖先进行 canonicalize，再拼回缺失部分。
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

/// 读取文件内容为 UTF-8 字符串。
///
/// 文件包含无效 UTF-8 时返回错误。
pub async fn read_utf8_file(path: &Path) -> Result<String> {
    fs::read_to_string(path)
        .map_err(|e| AstrError::io(format!("failed reading file '{}'", path.display()), e))
}

/// 将文本内容写入文件。
///
/// `create_dirs` 为 true 时自动创建缺失的父目录。
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

/// 将值序列化为 JSON 字符串，用于工具的结构化输出。
pub fn json_output<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(|e| AstrError::parse("failed to serialize output", e))
}

/// 文本变更报告，由 `build_text_change_report` 生成。
pub struct TextChangeReport {
    pub summary: String,
    pub metadata: Value,
}

/// 构建文本变更报告，包含 unified diff 和变更统计。
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

/// 简化的 unified diff 生成器（单 hunk）。
///
/// **为什么不使用外部 diff 库**：前端渲染只需要单 hunk 集中展示变更，
/// 不需要标准 diff 的多 hunk 分割。此算法通过前后缀匹配找到变更区域，
/// 加上最多 3 行上下文保证可读性。
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

    // 变更区域前后各取最多3行上下文，保证diff可读性
    let context_start = prefix_len.saturating_sub(3);
    let before_hunk_end = (before_change_end + 3).min(before_lines.len());
    let after_hunk_end = (after_change_end + 3).min(after_lines.len());
    // hunk header起始行使用1-based：有修改行时加1使上下文区域对齐
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
            Component::CurDir => {},
            Component::ParentDir => {
                let popped = normalized.pop();
                if !popped {
                    normalized.push(component.as_os_str());
                }
            },
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            },
        }
    }

    normalized
}

/// 解析路径到绝对形式，用于沙箱边界检查。
///
/// 当路径尾部组件尚不存在时（如 writeFile 创建新文件），
/// 向上找到第一个存在的祖先 canonicalize 后拼回缺失部分。
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

/// 移除 Windows `fs::canonicalize` 返回的 `\\?\` 前缀。
///
/// Windows canonicalize 返回 `\\?\` 开头的路径，移除后更友好。
/// 注意不要在此函数后使用 `starts_with` 做沙箱检查（已改用词法归一化）。
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

fn is_path_within_root(path: &Path, root: &Path) -> bool {
    let normalized_path = normalize_lexically(path);
    let normalized_root = normalize_lexically(root);
    normalized_path == normalized_root || normalized_path.starts_with(&normalized_root)
}

/// 工具输出 inline 阈值：序列化结果超过此字节数时触发存盘。
pub const TOOL_RESULT_INLINE_LIMIT: usize = 32 * 1024;

/// 工具结果存盘目录名（相对于 session 目录）。
pub const TOOL_RESULTS_DIR: &str = "tool-results";

/// 工具结果预览截断大小。
///
/// 超阈值被存盘时，返回此大小的预览供 LLM 快速了解内容。
pub const TOOL_RESULT_PREVIEW_LIMIT: usize = 2 * 1024;

/// 将大型工具结果存到磁盘并返回截断预览。
///
/// 超阈值时存到 `session_dir/tool-results/<id>.txt`，返回预览供 LLM 了解内容。
/// 存盘失败时降级为 `truncate_with_notice`。
/// `force_inline` 用于调试/测试模式跳过存盘。
pub fn maybe_persist_large_tool_result(
    session_dir: &std::path::Path,
    tool_call_id: &str,
    content: &str,
    force_inline: bool,
) -> String {
    let content_bytes = content.len();
    if content_bytes <= TOOL_RESULT_INLINE_LIMIT || force_inline {
        return content.to_string();
    }

    let results_dir = session_dir.join(TOOL_RESULTS_DIR);
    if std::fs::create_dir_all(&results_dir).is_err() {
        log::warn!(
            "tool-result: failed to create dir '{}', falling back to truncation",
            results_dir.display()
        );
        return truncate_with_notice(content);
    }

    let safe_id: String = tool_call_id
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .take(64)
        .collect();
    let path = results_dir.join(format!("{safe_id}.txt"));
    if std::fs::write(&path, content).is_err() {
        log::warn!(
            "tool-result: failed to write '{}', falling back to truncation",
            path.display()
        );
        return truncate_with_notice(content);
    }

    let relative_path = path
        .strip_prefix(session_dir)
        .unwrap_or(&path)
        .to_string_lossy()
        .replace('\\', "/");

    format_persisted_output(&relative_path, content_bytes, content)
}

/// 截断内容并附加通知。
fn truncate_with_notice(content: &str) -> String {
    let limit = TOOL_RESULT_PREVIEW_LIMIT.min(content.len());
    let truncated_at = content.floor_char_boundary(limit);
    let prefix = &content[..truncated_at];
    format!(
        "{prefix}\n\n... [output truncated after {} bytes; use offset/limit parameters or \
         readFile with persisted path for full content]",
        TOOL_RESULT_INLINE_LIMIT
    )
}

/// 构建 `<persisted-output>` 格式的截断预览。
///
/// 让 LLM 能在同 turn 中用 `readFile` 读取完整内容。
fn format_persisted_output(relative_path: &str, total_bytes: usize, content: &str) -> String {
    let preview_limit = TOOL_RESULT_PREVIEW_LIMIT.min(content.len());
    let truncated_at = content.floor_char_boundary(preview_limit);
    let preview = &content[..truncated_at];

    format!(
        "<persisted-output>\nOutput too large ({total_bytes} bytes). Full output saved to: \
         {relative_path}\n\nPreview (first {preview_limit} bytes):\n{preview}\n...\nUse \
         `readFile` with the persisted path to view the full content.\n</persisted-output>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{canonical_tool_path, test_tool_context_for};

    #[test]
    fn check_cancel_returns_error_for_cancelled_token() {
        let ctx = test_tool_context_for(std::env::temp_dir());
        ctx.cancel().cancel();

        let err = check_cancel(ctx.cancel()).expect_err("cancelled token should fail");
        assert!(err.to_string().contains("cancelled"));
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
    fn resolve_path_allows_absolute_path_inside_working_dir() {
        let working_dir = tempfile::tempdir().expect("tempdir should be created");
        let file = working_dir.path().join("notes.txt");
        fs::write(&file, "hello").expect("file should be created");
        let ctx = test_tool_context_for(working_dir.path());

        let resolved = resolve_path(&ctx, &file).expect("path should resolve");

        assert_eq!(resolved, canonical_tool_path(&file));
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
}
