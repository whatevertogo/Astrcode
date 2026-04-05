//! # Phase Tracker
//!
//! Tracks session phase transitions and emits `PhaseChanged` events when the
//! phase actually changes. Extracted from `EventTranslator` so the core phase
//! transition logic can be tested in isolation.

use crate::{AgentEvent, AgentEventContext, Phase, StorageEvent};

/// Determines the target phase for a storage event.
pub fn target_phase(event: &StorageEvent) -> Phase {
    match event {
        StorageEvent::SessionStart { .. } => Phase::Idle,
        StorageEvent::UserMessage { .. } => Phase::Thinking,
        StorageEvent::PromptMetrics { .. } | StorageEvent::CompactApplied { .. } => Phase::Idle,
        StorageEvent::AssistantDelta { .. }
        | StorageEvent::ThinkingDelta { .. }
        | StorageEvent::AssistantFinal { .. } => Phase::Streaming,
        StorageEvent::ToolCall { .. }
        | StorageEvent::ToolCallDelta { .. }
        | StorageEvent::ToolResult { .. } => Phase::CallingTool,
        StorageEvent::TurnDone { .. } => Phase::Idle,
        StorageEvent::Error { message, .. } if message == "interrupted" => Phase::Interrupted,
        StorageEvent::Error { .. } => Phase::Idle,
    }
}

/// Stateful phase tracker.
///
/// Call [`Self::on_event`] whenever a new `StorageEvent` arrives. If the event
/// causes a phase transition you'll get back `Some(AgentEvent::PhaseChanged)`
/// and should push it *before* the primary event record.
pub struct PhaseTracker {
    current: Phase,
}

impl PhaseTracker {
    pub fn new(initial: Phase) -> Self {
        Self { current: initial }
    }

    /// Process a storage event and return a `PhaseChanged` event if the phase
    /// actually changed.
    pub fn on_event(
        &mut self,
        event: &StorageEvent,
        turn_id: Option<String>,
        agent: AgentEventContext,
    ) -> Option<AgentEvent> {
        if matches!(
            event,
            StorageEvent::PromptMetrics { .. } | StorageEvent::CompactApplied { .. }
        ) {
            return None;
        }
        let new_phase = target_phase(event);
        if self.current != new_phase {
            self.current = new_phase;
            Some(AgentEvent::PhaseChanged {
                turn_id,
                agent,
                phase: new_phase,
            })
        } else {
            // Update internal state even when caller handles phase externally
            // (e.g. SessionStart always goes to Idle, Error can override).
            self.current = new_phase;
            None
        }
    }

    pub fn current(&self) -> Phase {
        self.current
    }

    /// Force a phase transition. Used by SessionStart and TurnDone where the
    /// phase must change regardless of the event type alone.
    pub fn force_to(
        &mut self,
        phase: Phase,
        turn_id: Option<String>,
        agent: AgentEventContext,
    ) -> AgentEvent {
        self.current = phase;
        AgentEvent::PhaseChanged {
            turn_id,
            agent,
            phase,
        }
    }
}
