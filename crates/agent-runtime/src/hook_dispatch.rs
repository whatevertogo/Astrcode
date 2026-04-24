use astrcode_core::{HookEventKey, Result};
use async_trait::async_trait;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct HookDispatchRequest {
    pub snapshot_id: String,
    pub event: HookEventKey,
    pub session_id: String,
    pub turn_id: String,
    pub agent_id: String,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookEffectKind {
    Continue,
    Block,
    CancelTurn,
    AugmentPrompt,
    Diagnostic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookEffect {
    pub kind: HookEffectKind,
    pub message: Option<String>,
    pub terminal: bool,
}

impl HookEffect {
    pub fn continue_flow() -> Self {
        Self {
            kind: HookEffectKind::Continue,
            message: None,
            terminal: false,
        }
    }

    pub fn cancel_turn(message: impl Into<String>) -> Self {
        Self {
            kind: HookEffectKind::CancelTurn,
            message: Some(message.into()),
            terminal: true,
        }
    }

    pub fn augment_prompt(message: impl Into<String>) -> Self {
        Self {
            kind: HookEffectKind::AugmentPrompt,
            message: Some(message.into()),
            terminal: false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HookDispatchOutcome {
    pub effects: Vec<HookEffect>,
}

/// runtime 消费的抽象 hooks 调度面。
///
/// `agent-runtime` 只知道 snapshot id 和事件点；具体 hook registry、匹配与
/// builtin/external handler 归 plugin-host。
#[async_trait]
pub trait HookDispatcher: Send + Sync {
    async fn dispatch_hook(&self, request: HookDispatchRequest) -> Result<HookDispatchOutcome>;
}
