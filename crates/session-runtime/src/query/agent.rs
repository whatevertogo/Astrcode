//! Agent 只读观察投影。
//!
//! Why: `observe` 只负责读侧快照，不再暴露 input queue 的建议字段。

use astrcode_core::{AgentLifecycleStatus, AgentState, InputQueueProjection, LlmMessage};

use crate::query::text::{summarize_inline_text, truncate_text};

#[derive(Debug, Clone)]
pub struct AgentObserveSnapshot {
    pub phase: astrcode_core::Phase,
    pub turn_count: u32,
    pub active_task: Option<String>,
    pub last_output_tail: Option<String>,
    pub last_turn_tail: Vec<String>,
}

pub(crate) fn build_agent_observe_snapshot(
    lifecycle_status: AgentLifecycleStatus,
    projected: &AgentState,
    input_queue_projection: &InputQueueProjection,
) -> AgentObserveSnapshot {
    AgentObserveSnapshot {
        phase: projected.phase,
        turn_count: projected.turn_count as u32,
        active_task: active_task_summary(lifecycle_status, projected, input_queue_projection),
        last_output_tail: extract_last_output(&projected.messages),
        last_turn_tail: extract_last_turn_tail(&projected.messages),
    }
}

fn extract_last_output(messages: &[LlmMessage]) -> Option<String> {
    messages.iter().rev().find_map(|msg| match msg {
        LlmMessage::Assistant { content, .. } if !content.is_empty() => truncate_text(content, 200),
        _ => None,
    })
}

fn active_task_summary(
    lifecycle_status: AgentLifecycleStatus,
    projected: &AgentState,
    input_queue_projection: &InputQueueProjection,
) -> Option<String> {
    if !input_queue_projection.active_delivery_ids.is_empty() {
        return extract_last_turn_tail(&projected.messages)
            .into_iter()
            .next();
    }

    if matches!(
        lifecycle_status,
        AgentLifecycleStatus::Pending | AgentLifecycleStatus::Running
    ) {
        return projected
            .messages
            .iter()
            .rev()
            .find_map(|message| match message {
                LlmMessage::User {
                    content,
                    origin: astrcode_core::UserMessageOrigin::User,
                } => summarize_inline_text(content, 120),
                _ => None,
            });
    }

    None
}

fn extract_last_turn_tail(messages: &[LlmMessage]) -> Vec<String> {
    messages
        .iter()
        .rev()
        .filter_map(|message| match message {
            LlmMessage::User { content, .. } => summarize_inline_text(content, 120),
            LlmMessage::Assistant { content, .. } => summarize_inline_text(content, 120),
            LlmMessage::Tool { content, .. } => summarize_inline_text(content, 120),
        })
        .take(3)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use astrcode_core::{AgentState, LlmMessage, ModeId, Phase, UserMessageOrigin};

    use super::{build_agent_observe_snapshot, extract_last_turn_tail};

    fn projected(messages: Vec<LlmMessage>, phase: Phase) -> AgentState {
        AgentState {
            session_id: "session-1".into(),
            working_dir: PathBuf::from("/tmp"),
            phase,
            turn_count: 2,
            mode_id: ModeId::code(),
            messages,
            last_assistant_at: None,
        }
    }

    #[test]
    fn extract_last_turn_tail_returns_recent_message_tail() {
        let tail = extract_last_turn_tail(&[
            LlmMessage::User {
                content: "first".to_string(),
                origin: UserMessageOrigin::User,
            },
            LlmMessage::Assistant {
                content: "second".to_string(),
                tool_calls: Vec::new(),
                reasoning: None,
            },
            LlmMessage::Tool {
                tool_call_id: "call-1".to_string(),
                content: "third".to_string(),
            },
            LlmMessage::Assistant {
                content: "fourth".to_string(),
                tool_calls: Vec::new(),
                reasoning: None,
            },
        ]);

        assert_eq!(tail, vec!["second", "third", "fourth"]);
    }

    #[test]
    fn build_agent_observe_snapshot_prefers_latest_user_task_for_running_agent() {
        let snapshot = build_agent_observe_snapshot(
            astrcode_core::AgentLifecycleStatus::Running,
            &projected(
                vec![
                    LlmMessage::User {
                        content: "请检查 input queue 路径".to_string(),
                        origin: UserMessageOrigin::User,
                    },
                    LlmMessage::Assistant {
                        content: "处理中".to_string(),
                        tool_calls: Vec::new(),
                        reasoning: None,
                    },
                ],
                Phase::Streaming,
            ),
            &astrcode_core::InputQueueProjection::default(),
        );

        assert_eq!(
            snapshot.active_task.as_deref(),
            Some("请检查 input queue 路径")
        );
        assert_eq!(snapshot.last_output_tail.as_deref(), Some("处理中"));
    }

    #[test]
    fn build_agent_observe_snapshot_uses_turn_tail_when_active_delivery_exists() {
        let input_queue_projection = astrcode_core::InputQueueProjection {
            active_delivery_ids: vec!["delivery-1".into()],
            ..Default::default()
        };

        let snapshot = build_agent_observe_snapshot(
            astrcode_core::AgentLifecycleStatus::Idle,
            &projected(
                vec![
                    LlmMessage::User {
                        content: "继续整理父级需要的结论".to_string(),
                        origin: UserMessageOrigin::QueuedInput,
                    },
                    LlmMessage::Assistant {
                        content: "正在合并结果".to_string(),
                        tool_calls: Vec::new(),
                        reasoning: None,
                    },
                ],
                Phase::Thinking,
            ),
            &input_queue_projection,
        );

        assert_eq!(
            snapshot.active_task.as_deref(),
            Some("继续整理父级需要的结论")
        );
    }
}
