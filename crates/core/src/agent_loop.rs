mod llm_cycle;
mod tool_cycle;
mod turn_runner;

use anyhow::Result;
use chrono::Utc;
use tokio_util::sync::CancellationToken;

use crate::events::StorageEvent;
use crate::projection::AgentState;
use crate::provider_factory::DynProviderFactory;
use crate::tools::registry::ToolRegistry;

pub struct AgentLoop {
    factory: DynProviderFactory,
    tools: ToolRegistry,
    max_steps: Option<usize>,
}

impl AgentLoop {
    pub fn new(factory: DynProviderFactory, tools: ToolRegistry) -> Self {
        Self {
            factory,
            tools,
            max_steps: None,
        }
    }

    pub fn with_max_steps(mut self, max_steps: usize) -> Self {
        self.max_steps = Some(max_steps);
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
        on_event: &mut impl FnMut(StorageEvent),
        cancel: CancellationToken,
    ) -> Result<()> {
        turn_runner::run_turn(self, state, on_event, cancel).await
    }
}

pub(crate) fn finish_turn(on_event: &mut impl FnMut(StorageEvent)) {
    on_event(StorageEvent::TurnDone {
        timestamp: Utc::now(),
    });
}

pub(crate) fn finish_with_error(
    message: impl Into<String>,
    on_event: &mut impl FnMut(StorageEvent),
) {
    on_event(StorageEvent::Error {
        message: message.into(),
    });
    finish_turn(on_event);
}

pub(crate) fn finish_interrupted(on_event: &mut impl FnMut(StorageEvent)) {
    finish_with_error("interrupted", on_event);
}

#[cfg(test)]
mod tests;
