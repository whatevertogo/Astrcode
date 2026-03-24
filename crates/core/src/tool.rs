use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::Value;

use crate::{CancelToken, Result, ToolDefinition, ToolExecutionResult};

pub type SessionId = String;

/// Default maximum output size for tool execution (1 MB)
pub const DEFAULT_MAX_OUTPUT_SIZE: usize = 1024 * 1024;

#[derive(Clone, Debug)]
pub struct ToolContext {
    pub session_id: SessionId,
    pub working_dir: PathBuf,
    pub cancel: CancelToken,
    /// Maximum output size in bytes. Defaults to 1MB.
    pub max_output_size: usize,
}

impl ToolContext {
    pub fn new(session_id: SessionId, working_dir: PathBuf, cancel: CancelToken) -> Self {
        Self {
            session_id,
            working_dir,
            cancel,
            max_output_size: DEFAULT_MAX_OUTPUT_SIZE,
        }
    }

    pub fn with_max_output_size(mut self, max_output_size: usize) -> Self {
        self.max_output_size = max_output_size;
        self
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn definition(&self) -> ToolDefinition;

    async fn execute(
        &self,
        tool_call_id: String,
        input: Value,
        ctx: &ToolContext,
    ) -> Result<ToolExecutionResult>;
}
