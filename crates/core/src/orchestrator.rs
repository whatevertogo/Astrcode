use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{kernel_api::KernelApi, AstrError, CancelToken, SessionId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnContext {
    pub session_id: SessionId,
    pub user_message: String,
    // Cancellation handles are process-local runtime state, so serializing them would create a
    // misleading snapshot that cannot preserve the original atomic linkage.
    #[serde(skip, default)]
    pub cancel: CancelToken,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnOutcome {
    pub completed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[async_trait]
pub trait Orchestrator: Send + Sync {
    async fn run_turn(
        &self,
        ctx: &TurnContext,
        kernel: &dyn KernelApi,
    ) -> std::result::Result<TurnOutcome, AstrError>;
}
