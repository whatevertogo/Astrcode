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

use crate::{
    AgentEvent, AgentEventContext, Phase, StorageEvent, StorageEventPayload, TurnTerminalKind,
    UserMessageOrigin,
};

/// Determines the target phase for a storage event.
pub fn target_phase(event: &StorageEvent) -> Phase {
    match &event.payload {
        StorageEventPayload::SessionStart { .. } => Phase::Idle,
        StorageEventPayload::UserMessage { origin, .. } => {
            if matches!(origin, UserMessageOrigin::User) {
                Phase::Thinking
            } else {
                Phase::Idle
            }
        },
        StorageEventPayload::PromptMetrics { .. }
        | StorageEventPayload::CompactApplied { .. }
        | StorageEventPayload::ToolResultReferenceApplied { .. }
        | StorageEventPayload::SubRunStarted { .. }
        | StorageEventPayload::SubRunFinished { .. }
        | StorageEventPayload::ChildSessionNotification { .. }
        | StorageEventPayload::AgentCollaborationFact { .. }
        | StorageEventPayload::ModeChanged { .. }
        | StorageEventPayload::AgentInputQueued { .. }
        | StorageEventPayload::AgentInputBatchStarted { .. }
        | StorageEventPayload::AgentInputBatchAcked { .. }
        | StorageEventPayload::AgentInputDiscarded { .. } => Phase::Idle,
        StorageEventPayload::AssistantDelta { .. }
        | StorageEventPayload::ThinkingDelta { .. }
        | StorageEventPayload::AssistantFinal { .. } => Phase::Streaming,
        StorageEventPayload::ToolCall { .. }
        | StorageEventPayload::ToolCallDelta { .. }
        | StorageEventPayload::ToolResult { .. } => Phase::CallingTool,
        StorageEventPayload::TurnDone { terminal_kind, .. } => {
            if matches!(terminal_kind, Some(TurnTerminalKind::Cancelled)) {
                Phase::Interrupted
            } else {
                Phase::Idle
            }
        },
        StorageEventPayload::Error { .. } => Phase::Idle,
    }
}

/// 规范化冷恢复场景下的 phase。
///
/// `Thinking` / `Streaming` / `CallingTool` 只应存在于活进程内。
/// 如果会话是从磁盘历史冷恢复出来的，却仍停留在这些中间态，
/// 说明上一次进程在 turn 尚未完成时就退出了；此时应显式降级为 `Interrupted`，
/// 避免 UI 把陈旧会话误判成仍在运行。
pub fn normalize_recovered_phase(phase: Phase) -> Phase {
    match phase {
        Phase::Thinking | Phase::Streaming | Phase::CallingTool => Phase::Interrupted,
        other => other,
    }
}

/// Stateful phase tracker.
///
/// 维护当前会话阶段状态，在阶段实际变更时才发出 `PhaseChanged` 事件。
/// 这是 SSE 推送和前端状态指示器的唯一 phase 来源。
///
/// 关键设计：
/// - 内部唤醒消息（QueuedInput / ContinuationPrompt / ReactivationPrompt / RecentUserContextDigest
///   / RecentUserContext / CompactSummary）不触发 phase 变更，避免 UI 闪烁
/// - 辅助事件（PromptMetrics / CompactApplied / SubRun 等）也不触发 phase 变更
/// - `force_to` 用于 SessionStart → Idle 和 TurnDone → Idle 这类必须变更的场景
pub struct PhaseTracker {
    current: Phase,
}

impl PhaseTracker {
    pub fn new(initial: Phase) -> Self {
        Self { current: initial }
    }

    /// 处理存储事件，若阶段实际变更则返回 `PhaseChanged` 事件。
    ///
    /// 返回的事件应在主事件之前推送（before-push），
    /// 这样前端先收到 PhaseChanged 再收到实际内容，保证状态指示器及时更新。
    pub fn on_event(
        &mut self,
        event: &StorageEvent,
        turn_id: Option<String>,
        agent: AgentEventContext,
    ) -> Option<AgentEvent> {
        if matches!(
            &event.payload,
            StorageEventPayload::UserMessage {
                origin: UserMessageOrigin::QueuedInput
                    | UserMessageOrigin::ContinuationPrompt
                    | UserMessageOrigin::ReactivationPrompt
                    | UserMessageOrigin::RecentUserContextDigest
                    | UserMessageOrigin::RecentUserContext
                    | UserMessageOrigin::CompactSummary,
                ..
            }
        ) {
            return None;
        }
        if matches!(
            &event.payload,
            StorageEventPayload::PromptMetrics { .. }
                | StorageEventPayload::CompactApplied { .. }
                | StorageEventPayload::ToolResultReferenceApplied { .. }
                | StorageEventPayload::SubRunStarted { .. }
                | StorageEventPayload::SubRunFinished { .. }
                | StorageEventPayload::ChildSessionNotification { .. }
                | StorageEventPayload::AgentCollaborationFact { .. }
                | StorageEventPayload::ModeChanged { .. }
                | StorageEventPayload::AgentInputQueued { .. }
                | StorageEventPayload::AgentInputBatchStarted { .. }
                | StorageEventPayload::AgentInputBatchAcked { .. }
                | StorageEventPayload::AgentInputDiscarded { .. }
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

    /// 强制切换到指定阶段，无视事件类型推断。
    ///
    /// 用于 SessionStart（必须到 Idle）和 TurnDone（必须回到 Idle）等
    /// 不依赖事件类型就能确定目标阶段的场景。
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

#[cfg(test)]
mod tests {
    use super::{PhaseTracker, normalize_recovered_phase, target_phase};
    use crate::{AgentEventContext, Phase, StorageEvent, StorageEventPayload, UserMessageOrigin};

    fn user_message(origin: UserMessageOrigin) -> StorageEvent {
        StorageEvent {
            turn_id: Some("turn-1".to_string()),
            agent: AgentEventContext::default(),
            payload: StorageEventPayload::UserMessage {
                content: "message".to_string(),
                origin,
                timestamp: chrono::Utc::now(),
            },
        }
    }

    #[test]
    fn normalize_recovered_phase_maps_transient_runtime_states_to_interrupted() {
        assert_eq!(
            normalize_recovered_phase(Phase::Thinking),
            Phase::Interrupted
        );
        assert_eq!(
            normalize_recovered_phase(Phase::Streaming),
            Phase::Interrupted
        );
        assert_eq!(
            normalize_recovered_phase(Phase::CallingTool),
            Phase::Interrupted
        );
    }

    #[test]
    fn normalize_recovered_phase_preserves_terminal_and_stable_states() {
        assert_eq!(normalize_recovered_phase(Phase::Idle), Phase::Idle);
        assert_eq!(
            normalize_recovered_phase(Phase::Interrupted),
            Phase::Interrupted
        );
        assert_eq!(normalize_recovered_phase(Phase::Done), Phase::Done);
    }

    #[test]
    fn internal_user_origins_do_not_request_thinking_phase() {
        assert_eq!(
            target_phase(&user_message(UserMessageOrigin::ReactivationPrompt)),
            Phase::Idle
        );
        assert_eq!(
            target_phase(&user_message(UserMessageOrigin::CompactSummary)),
            Phase::Idle
        );
        assert_eq!(
            target_phase(&user_message(UserMessageOrigin::ContinuationPrompt)),
            Phase::Idle
        );
    }

    #[test]
    fn phase_tracker_ignores_internal_user_origins() {
        let mut tracker = PhaseTracker::new(Phase::Idle);
        assert!(
            tracker
                .on_event(
                    &user_message(UserMessageOrigin::ReactivationPrompt),
                    Some("turn-1".to_string()),
                    AgentEventContext::default(),
                )
                .is_none()
        );
        assert_eq!(tracker.current(), Phase::Idle);
        assert!(
            tracker
                .on_event(
                    &user_message(UserMessageOrigin::ContinuationPrompt),
                    Some("turn-1".to_string()),
                    AgentEventContext::default(),
                )
                .is_none()
        );
        assert_eq!(tracker.current(), Phase::Idle);
    }
}
