use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::Value;

use crate::{CancelToken, Result, ToolDefinition, ToolExecutionResult};

pub type SessionId = String;

#[derive(Clone, Debug)]
pub struct ToolContext {
    pub session_id: SessionId,
    pub working_dir: PathBuf,
    pub cancel: CancelToken,
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
