use std::path::PathBuf;

use astrcode_core::{CancelToken, ToolContext};

#[allow(dead_code)]
pub fn test_tool_context() -> ToolContext {
    test_tool_context_for(std::env::temp_dir())
}

pub fn test_tool_context_for(path: impl Into<PathBuf>) -> ToolContext {
    let cwd = path.into();
    ToolContext::new("session-test".to_string(), cwd, CancelToken::new())
}
