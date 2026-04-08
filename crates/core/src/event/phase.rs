//! # 阶段追踪器
//!
//! 追踪会话的阶段转换，在阶段实际发生变化时发送 `PhaseChanged` 事件。
//! 从 `EventTranslator` 中提取出来，以便独立测试阶段转换逻辑。
//!
//! ## 阶段类型
//!
//! - `Idle`: 空闲状态，等待用户输入
//! - `Thinking`: 正在思考/生成响应
//! - `Streaming`: 正在流式输出内容
//! - `CallingTool`: 正在调用工具
//! - `Interrupted`: 被用户中断

use crate::{AgentEvent, AgentEventContext, Phase, StorageEvent};

/// Determines the target phase for a storage event.
pub fn target_phase(event: &StorageEvent) -> Phase {
    match event {
        StorageEvent::SessionStart { .. } => Phase::Idle,
        StorageEvent::UserMessage { .. } => Phase::Thinking,
        StorageEvent::PromptMetrics { .. }
        | StorageEvent::CompactApplied { .. }
        | StorageEvent::SubRunStarted { .. }
        | StorageEvent::SubRunFinished { .. }
        | StorageEvent::ChildSessionNotification { .. } => Phase::Idle,
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
            StorageEvent::PromptMetrics { .. }
                | StorageEvent::CompactApplied { .. }
                | StorageEvent::SubRunStarted { .. }
                | StorageEvent::SubRunFinished { .. }
                | StorageEvent::ChildSessionNotification { .. }
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
