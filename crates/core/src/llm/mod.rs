use std::sync::Arc;
use std::sync::Mutex;

use anyhow::Result;
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::action::{LlmMessage, LlmResponse, ToolDefinition};

pub mod openai;

/// Type alias for a thread-safe delta callback.
pub type DeltaCallback = Arc<Mutex<dyn FnMut(String) + Send>>;

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(
        &self,
        messages: &[LlmMessage],
        tools: &[ToolDefinition],
        cancel: CancellationToken,
    ) -> Result<LlmResponse>;

    async fn stream_complete(
        &self,
        messages: &[LlmMessage],
        tools: &[ToolDefinition],
        cancel: CancellationToken,
        on_delta: DeltaCallback,
    ) -> Result<LlmResponse>;
}
