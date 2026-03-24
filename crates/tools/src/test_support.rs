use std::path::PathBuf;

use astrcode_core::{CancelToken, ToolContext};

#[allow(dead_code)]
pub fn test_tool_context() -> ToolContext {
    test_tool_context_for(std::env::temp_dir())
}

pub fn test_tool_context_for(path: impl Into<PathBuf>) -> ToolContext {
    let cwd = path.into();
    ToolContext {
        session_id: "session-test".to_string(),
        working_dir: cwd,
        cancel: CancelToken::new(),
        max_output_size: astrcode_core::DEFAULT_MAX_OUTPUT_SIZE,
    }
}
