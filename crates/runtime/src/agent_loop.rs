mod llm_cycle;
mod tool_cycle;
mod turn_runner;

use astrcode_core::{
    AllowAllPolicyEngine, AstrError, CancelToken, CapabilityRouter, PolicyContext, PolicyEngine,
    Result, ToolContext,
};
use chrono::Utc;
use std::sync::Arc;

use crate::approval_service::{ApprovalBroker, DefaultApprovalBroker};
use crate::prompt::PromptComposer;
use crate::provider_factory::DynProviderFactory;
use astrcode_core::AgentState;
use astrcode_core::StorageEvent;

pub struct AgentLoop {
    factory: DynProviderFactory,
    capabilities: CapabilityRouter,
    policy: Arc<dyn PolicyEngine>,
    approval: Arc<dyn ApprovalBroker>,
    max_steps: Option<usize>,
    prompt_composer: PromptComposer,
}

impl AgentLoop {
    pub fn from_capabilities(factory: DynProviderFactory, capabilities: CapabilityRouter) -> Self {
        Self {
            factory,
            capabilities,
            policy: Arc::new(AllowAllPolicyEngine),
            approval: Arc::new(DefaultApprovalBroker),
            max_steps: None,
            prompt_composer: PromptComposer::with_defaults(),
        }
    }

    pub fn with_policy_engine(mut self, policy: Arc<dyn PolicyEngine>) -> Self {
        self.policy = policy;
        self
    }

    pub fn with_approval_broker(mut self, approval: Arc<dyn ApprovalBroker>) -> Self {
        self.approval = approval;
        self
    }

    pub fn with_max_steps(mut self, max_steps: usize) -> Self {
        self.max_steps = Some(max_steps);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_prompt_composer(mut self, prompt_composer: PromptComposer) -> Self {
        self.prompt_composer = prompt_composer;
        self
    }

    /// Execute one turn of the agent loop.
    ///
    /// `state` provides the conversation history (messages) reconstructed from events.
    /// Every significant result is emitted as a `StorageEvent` via `on_event`.
    /// The loop itself performs no IO besides LLM calls and tool execution.
    pub async fn run_turn(
        &self,
        state: &AgentState,
        turn_id: &str,
        on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
        cancel: CancelToken,
    ) -> Result<()> {
        turn_runner::run_turn(self, state, turn_id, on_event, cancel).await
    }

    pub(crate) fn tool_context(&self, state: &AgentState, cancel: CancelToken) -> ToolContext {
        ToolContext::new(state.session_id.clone(), state.working_dir.clone(), cancel)
    }

    pub(crate) fn policy_context(
        &self,
        state: &AgentState,
        turn_id: &str,
        step_index: usize,
    ) -> PolicyContext {
        PolicyContext {
            session_id: state.session_id.clone(),
            turn_id: turn_id.to_string(),
            step_index,
            working_dir: state.working_dir.to_string_lossy().into_owned(),
            profile: "coding".to_string(),
            metadata: serde_json::Value::Null,
        }
    }
}

pub(crate) fn finish_turn(
    turn_id: &str,
    on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
) -> Result<()> {
    on_event(StorageEvent::TurnDone {
        turn_id: Some(turn_id.to_string()),
        timestamp: Utc::now(),
    })
}

pub(crate) fn finish_with_error(
    turn_id: &str,
    message: impl Into<String>,
    on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
) -> Result<()> {
    on_event(StorageEvent::Error {
        turn_id: Some(turn_id.to_string()),
        message: message.into(),
        timestamp: Some(Utc::now()),
    })?;
    finish_turn(turn_id, on_event)
}

pub(crate) fn finish_interrupted(
    turn_id: &str,
    on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
) -> Result<()> {
    finish_with_error(turn_id, "interrupted", on_event)
}

pub(crate) fn internal_error(error: impl std::fmt::Display) -> AstrError {
    AstrError::Internal(error.to_string())
}

#[cfg(test)]
mod tests;
