use std::path::PathBuf;

use astrcode_core::{CancelToken, ToolContext};

pub fn test_tool_context_for(path: impl Into<PathBuf>) -> ToolContext {
    let cwd = path.into();
    ToolContext::new("session-test".to_string(), cwd, CancelToken::new())
}
