use std::path::{Path, PathBuf};

use astrcode_core::{CancelToken, ToolContext};

pub fn test_tool_context_for(path: impl Into<PathBuf>) -> ToolContext {
    let cwd = path.into();
    // 工具测试里凡是会把中间结果持久化到 session 目录的实现，都应留在当前 tempdir
    // 内部，避免污染开发者真实的 `~/.astrcode/projects/...`。
    let session_storage_root = cwd.join(".astrcode-test-state");
    ToolContext::new("session-test".to_string(), cwd, CancelToken::new())
        .with_session_storage_root(session_storage_root)
        // 测试上下文使用 100KB 内联阈值，与 readFile 的 max_result_inline_size(100_000) 对齐。
        // 避免工具测试中 readFile 二次持久化已持久化的 grep 结果。
        .with_resolved_inline_limit(100_000)
}

pub fn canonical_tool_path(path: impl AsRef<Path>) -> PathBuf {
    let canonical =
        std::fs::canonicalize(path.as_ref()).unwrap_or_else(|_| path.as_ref().to_path_buf());

    // Tests should compare against the same path spelling that tools expose in metadata.
    // Windows may surface either 8.3 short names or long names depending on how TempDir was
    // created, so we normalize away verbatim prefixes and trust canonicalize's stable spelling.
    #[cfg(windows)]
    {
        if let Some(rendered) = canonical.to_str() {
            if let Some(stripped) = rendered.strip_prefix(r"\\?\UNC\") {
                return PathBuf::from(format!(r"\\{}", stripped));
            }
            if let Some(stripped) = rendered.strip_prefix(r"\\?\") {
                return PathBuf::from(stripped);
            }
        }
    }

    canonical
}
