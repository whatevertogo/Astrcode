//! 工具结果磁盘持久化基础设施。
//!
//! 提供工具结果落盘的核心函数，供工具执行侧（runtime-tool-loader）和
//! 管线聚合预算层（runtime-agent-loop）共享。
//!
//! # 两层接口
//!
//! - [`persist_tool_result`]：无条件落盘（不管内容大小），供管线聚合预算强制调用
//! - [`maybe_persist_tool_result`]：条件落盘（超过阈值时才落盘），供工具执行侧调用
//!
//! # 降级策略
//!
//! 磁盘写入失败时降级为截断预览，不 panic、不返回错误。
//! 这保证了即使文件系统不可用，工具结果仍然能以截断形式传递给 LLM。

use std::path::Path;

/// 工具结果存盘目录名（相对于 session 目录）。
pub const TOOL_RESULTS_DIR: &str = "tool-results";

/// 默认预览截断大小（字节）。
pub const TOOL_RESULT_PREVIEW_LIMIT: usize = 2 * 1024;

/// 默认内联阈值（字节）。
///
/// 工具结果超过此大小时触发落盘。可通过 `CapabilityDescriptor.max_result_inline_size`
/// 覆盖为 per-tool 值。
pub const DEFAULT_TOOL_RESULT_INLINE_LIMIT: usize = 32 * 1024;

/// 无条件将工具结果持久化到磁盘。
///
/// 不管内容大小，一律写入 `session_dir/tool-results/<id>.txt`，
/// 返回 `<persisted-output>` 格式的引用 + 预览。
/// 写入失败时降级为截断预览。
///
/// 供管线聚合预算层调用：当聚合预算超限时，选中的结果不管多大都需要落盘。
pub fn persist_tool_result(session_dir: &Path, tool_call_id: &str, content: &str) -> String {
    write_to_disk(session_dir, tool_call_id, content)
}

/// 条件持久化：仅当 content 大小超过 `inline_limit` 时落盘。
///
/// `inline_limit` 由调用方传入：
/// - 工具执行侧：从 `ToolContext::resolved_inline_limit()` 获取
/// - 其他场景：使用 `DEFAULT_TOOL_RESULT_INLINE_LIMIT`
pub fn maybe_persist_tool_result(
    session_dir: &Path,
    tool_call_id: &str,
    content: &str,
    inline_limit: usize,
) -> String {
    if content.len() <= inline_limit {
        return content.to_string();
    }
    write_to_disk(session_dir, tool_call_id, content)
}

/// 检测内容是否已被持久化（包含 `<persisted-output>` 标签）。
pub fn is_persisted_output(content: &str) -> bool {
    content.contains("<persisted-output>")
}

/// 实际写磁盘操作。
///
/// 包含完整的降级链路：
/// 1. `create_dir_all` 失败 → 截断预览
/// 2. `fs::write` 失败 → 截断预览
/// 3. 成功 → 生成 `<persisted-output>` 引用 + 预览
fn write_to_disk(session_dir: &Path, tool_call_id: &str, content: &str) -> String {
    let content_bytes = content.len();
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

/// 生成 `<persisted-output>` 格式的引用 + 预览。
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

/// 截断内容并附加通知。
fn truncate_with_notice(content: &str) -> String {
    let limit = TOOL_RESULT_PREVIEW_LIMIT.min(content.len());
    let truncated_at = content.floor_char_boundary(limit);
    let prefix = &content[..truncated_at];
    format!(
        "{prefix}\n\n... [output truncated after {} bytes; use offset/limit parameters or \
         readFile with persisted path for full content]",
        DEFAULT_TOOL_RESULT_INLINE_LIMIT
    )
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn persist_tool_result_writes_file_and_returns_reference() {
        let dir = tempfile::tempdir().expect("tempdir");
        let content = "x".repeat(100);
        let result = persist_tool_result(dir.path(), "call-abc123", &content);

        assert!(result.contains("<persisted-output>"));
        assert!(result.contains("tool-results/call-abc123.txt"));
        assert!(result.contains("100 bytes"));

        let file_path = dir.path().join("tool-results/call-abc123.txt");
        assert!(file_path.exists());
        assert_eq!(fs::read_to_string(&file_path).unwrap(), content);
    }

    #[test]
    fn maybe_persist_skips_when_below_limit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let content = "small".to_string();
        let result = maybe_persist_tool_result(dir.path(), "call-1", &content, 1024);

        assert_eq!(result, "small");
        assert!(!dir.path().join("tool-results/call-1.txt").exists());
    }

    #[test]
    fn maybe_persist_persists_when_above_limit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let content = "x".repeat(100);
        let result = maybe_persist_tool_result(dir.path(), "call-1", &content, 50);

        assert!(result.contains("<persisted-output>"));
        assert!(dir.path().join("tool-results/call-1.txt").exists());
    }

    #[test]
    fn is_persisted_output_detects_tag() {
        assert!(is_persisted_output(
            "<persisted-output>\nsome content\n</persisted-output>"
        ));
        assert!(!is_persisted_output("normal tool output"));
    }

    #[test]
    fn degrade_on_write_failure() {
        // Windows 上某些路径不会失败，所以只在实际降级时断言
        let content = "x".repeat(100);
        let result = persist_tool_result(Path::new("/nonexistent/path"), "call-1", &content);
        // 降级为截断预览或成功写入（取决于平台）
        assert!(result.contains("[output truncated") || result.contains("<persisted-output>"));
    }

    #[test]
    fn sanitizes_tool_call_id() {
        let dir = tempfile::tempdir().expect("tempdir");
        let content = "x".repeat(100);
        let _ = persist_tool_result(dir.path(), "call/../../../etc/passwd", &content);

        // 不应创建路径穿越目录
        assert!(!dir.path().join("etc").exists());
        // safe_id 只保留字母数字和 -_，过滤掉 / 和 .
        let file = dir.path().join("tool-results/calletcpasswd.txt");
        assert!(file.exists());
    }
}
