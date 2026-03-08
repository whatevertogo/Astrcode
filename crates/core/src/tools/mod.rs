use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::action::{ToolDefinition, ToolExecutionResult};

pub mod list_dir;
pub mod read_file;
pub mod registry;
pub mod shell;

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
