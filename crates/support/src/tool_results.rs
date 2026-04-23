//! 工具结果磁盘持久化。

use std::path::{Path, PathBuf};

use astrcode_core::tool_result_persist::{
    PersistedToolOutput, PersistedToolResult, TOOL_RESULT_PREVIEW_LIMIT, TOOL_RESULTS_DIR,
};

pub fn persist_tool_result(
    session_dir: &Path,
    tool_call_id: &str,
    content: &str,
) -> PersistedToolResult {
    write_to_disk(session_dir, tool_call_id, content)
}

pub fn maybe_persist_tool_result(
    session_dir: &Path,
    tool_call_id: &str,
    content: &str,
    inline_limit: usize,
) -> PersistedToolResult {
    if content.len() <= inline_limit {
        return PersistedToolResult {
            output: content.to_string(),
            persisted: None,
        };
    }
    write_to_disk(session_dir, tool_call_id, content)
}

fn write_to_disk(session_dir: &Path, tool_call_id: &str, content: &str) -> PersistedToolResult {
    let content_bytes = content.len();
    let results_dir = session_dir.join(TOOL_RESULTS_DIR);

    if std::fs::create_dir_all(&results_dir).is_err() {
        log::warn!(
            "tool-result: failed to create dir '{}', falling back to truncation",
            results_dir.display()
        );
        return PersistedToolResult {
            output: truncate_with_notice(content),
            persisted: None,
        };
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
        return PersistedToolResult {
            output: truncate_with_notice(content),
            persisted: None,
        };
    }

    let relative_path = path
        .strip_prefix(session_dir)
        .unwrap_or(&path)
        .to_string_lossy()
        .replace('\\', "/");
    let persisted = PersistedToolOutput {
        storage_kind: "toolResult".to_string(),
        absolute_path: normalize_absolute_path(&path),
        relative_path,
        total_bytes: content_bytes as u64,
        preview_text: build_preview_text(content),
        preview_bytes: TOOL_RESULT_PREVIEW_LIMIT.min(content.len()) as u64,
    };

    PersistedToolResult {
        output: format_persisted_output(&persisted),
        persisted: Some(persisted),
    }
}

fn format_persisted_output(persisted: &PersistedToolOutput) -> String {
    format!(
        "<persisted-output>\nLarge tool output was saved to a file instead of being \
         inlined.\nPath: {}\nBytes: {}\nRead the file with `readFile`.\nIf you only need a \
         section, read a smaller chunk instead of the whole file.\nStart from the first chunk \
         when you do not yet know the right section.\nSuggested first read: {{ path: {:?}, \
         charOffset: 0, maxChars: 20000 }}\n</persisted-output>",
        persisted.absolute_path, persisted.total_bytes, persisted.absolute_path
    )
}

fn build_preview_text(content: &str) -> String {
    let preview_limit = TOOL_RESULT_PREVIEW_LIMIT.min(content.len());
    let truncated_at = content.floor_char_boundary(preview_limit);
    content[..truncated_at].to_string()
}

fn normalize_absolute_path(path: &Path) -> String {
    normalize_verbatim_path(path.to_path_buf())
        .to_string_lossy()
        .to_string()
}

fn normalize_verbatim_path(path: PathBuf) -> PathBuf {
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

fn truncate_with_notice(content: &str) -> String {
    let limit = TOOL_RESULT_PREVIEW_LIMIT.min(content.len());
    let truncated_at = content.floor_char_boundary(limit);
    let prefix = &content[..truncated_at];
    format!(
        "{prefix}\n\n... [output truncated to {limit} bytes because persisted storage is \
         unavailable; use offset/limit parameters or rerun with a narrower scope for full content]"
    )
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use super::*;

    #[test]
    fn persist_tool_result_writes_file_and_returns_reference() {
        let dir = tempfile::tempdir().expect("tempdir");
        let content = "x".repeat(100);
        let result = persist_tool_result(dir.path(), "call-abc123", &content);

        assert!(result.output.contains("<persisted-output>"));
        assert!(result.output.contains("Large tool output was saved"));
        let persisted = result.persisted.expect("persisted metadata should exist");
        assert!(result.output.contains(&persisted.absolute_path));
        assert!(result.output.contains("Bytes: 100"));
        assert_eq!(persisted.relative_path, "tool-results/call-abc123.txt");
        assert_eq!(persisted.total_bytes, 100);
        assert_eq!(persisted.preview_text, content);
        assert_eq!(persisted.preview_bytes, 100);

        let file_path = dir.path().join("tool-results/call-abc123.txt");
        assert!(file_path.exists());
        assert_eq!(
            fs::read_to_string(&file_path).expect("persisted file should be readable"),
            content
        );
    }

    #[test]
    fn maybe_persist_skips_when_below_limit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let content = "small".to_string();
        let result = maybe_persist_tool_result(dir.path(), "call-1", &content, 1024);

        assert_eq!(result.output, "small");
        assert!(result.persisted.is_none());
        assert!(!dir.path().join("tool-results/call-1.txt").exists());
    }

    #[test]
    fn maybe_persist_persists_when_above_limit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let content = "x".repeat(100);
        let result = maybe_persist_tool_result(dir.path(), "call-1", &content, 50);

        assert!(result.output.contains("<persisted-output>"));
        assert!(result.persisted.is_some());
        assert!(dir.path().join("tool-results/call-1.txt").exists());
    }

    #[test]
    fn degrade_on_write_failure() {
        let content = "x".repeat(100);
        let result = persist_tool_result(Path::new("/nonexistent/path"), "call-1", &content);
        assert!(
            result.output.contains("[output truncated")
                || result.output.contains("<persisted-output>")
        );
    }

    #[test]
    fn sanitizes_tool_call_id() {
        let dir = tempfile::tempdir().expect("tempdir");
        let content = "x".repeat(100);
        let _ = persist_tool_result(dir.path(), "call/../../../etc/passwd", &content);

        assert!(!dir.path().join("etc").exists());
        let file = dir.path().join("tool-results/calletcpasswd.txt");
        assert!(file.exists());
    }
}
