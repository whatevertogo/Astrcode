use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{AgentEvent, AstrError, CancelToken, SessionId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnContext {
    pub session_id: SessionId,
    pub user_message: String,
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

#[async_trait]
pub trait RuntimeHandle: Send + Sync {
    fn runtime_name(&self) -> &'static str;

    fn runtime_kind(&self) -> &'static str;

    async fn shutdown(&self, timeout_secs: u64) -> std::result::Result<(), AstrError>;
}

#[async_trait]
pub trait ManagedRuntimeComponent: Send + Sync {
    fn component_name(&self) -> String;

    async fn shutdown_component(&self) -> std::result::Result<(), AstrError>;
}

#[async_trait]
pub trait KernelApi: Send + Sync {
    async fn emit_event(
        &self,
        session_id: SessionId,
        event: AgentEvent,
    ) -> std::result::Result<(), AstrError>;

    async fn invoke_capability(
        &self,
        name: &str,
        payload: Value,
    ) -> std::result::Result<Value, AstrError>;
}
