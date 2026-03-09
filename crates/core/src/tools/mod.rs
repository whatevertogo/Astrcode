// Tool error semantics:
//
// Err(anyhow::Error) - execution-level failure:
//   - argument parsing failure
//   - IO errors
//   - regex compilation failure
//   - cancellation
//
// ToolExecutionResult { ok: false } - tool-level refusal:
//   - safety policy rejection
//   - query condition not met but the system is healthy
//   - future destructive operations needing confirmation
//
// Rule of thumb:
//   - "the system failed" -> Err
//   - "the tool chose not to do it" -> ok: false
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::action::{ToolDefinition, ToolExecutionResult};

pub mod edit_file;
pub mod find_files;
pub mod fs_common;
pub mod grep;
pub mod list_dir;
pub mod read_file;
pub mod registry;
pub mod shell;
pub mod write_file;

#[async_trait]
pub trait Tool: Send + Sync {
    fn definition(&self) -> ToolDefinition;

    async fn execute(
        &self,
        tool_call_id: String,
        args: serde_json::Value,
        cancel: CancellationToken,
    ) -> Result<ToolExecutionResult>;
}

pub type DynTool = Arc<dyn Tool>;
