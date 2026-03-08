use anyhow::Result;
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::action::{LlmMessage, LlmResponse, ToolDefinition};

pub mod openai;

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(
        &self,
        messages: &[LlmMessage],
        tools: &[ToolDefinition],
        cancel: CancellationToken,
    ) -> Result<LlmResponse>;
}
