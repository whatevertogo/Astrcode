use async_trait::async_trait;
use serde_json::Value;

use crate::{AgentEvent, AstrError, SessionId};

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
