use astrcode_core::{HookEventKey, Result};
pub use astrcode_runtime_contract::hooks::{HookDispatchOutcome, HookEffect, HookEventPayload};
use async_trait::async_trait;

/// Hook dispatch request with typed event payload.
#[derive(Debug, Clone)]
pub struct HookDispatchRequest {
    pub snapshot_id: String,
    pub event: HookEventKey,
    pub session_id: String,
    pub turn_id: String,
    pub agent_id: String,
    pub payload: HookEventPayload,
}

/// runtime 消费的抽象 hooks 调度面。
///
/// `agent-runtime` 只知道 snapshot id 和事件点；具体 hook registry、匹配与
/// builtin/external handler 归 plugin-host。
#[async_trait]
pub trait HookDispatcher: Send + Sync {
    async fn dispatch_hook(&self, request: HookDispatchRequest) -> Result<HookDispatchOutcome>;
}
