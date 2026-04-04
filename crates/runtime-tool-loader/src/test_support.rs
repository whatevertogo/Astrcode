use std::path::{Path, PathBuf};

use astrcode_core::{CancelToken, ToolContext};

pub fn test_tool_context_for(path: impl Into<PathBuf>) -> ToolContext {
    let cwd = path.into();
    ToolContext::new("session-test".to_string(), cwd, CancelToken::new())
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
