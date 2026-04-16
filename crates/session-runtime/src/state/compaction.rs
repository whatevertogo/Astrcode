use std::collections::VecDeque;

use astrcode_core::{
    InvocationKind, StorageEvent, StorageEventPayload, StoredEvent, UserMessageOrigin,
};

/// Manual / auto compact 都应该基于 durable tail，而不是投影后的消息列表。
pub fn recent_turn_event_tail(
    events: &[StoredEvent],
    keep_recent_turns: usize,
) -> Vec<StoredEvent> {
    let keep_recent_turns = keep_recent_turns.max(1);
    let mut tail_refs = Vec::new();
    let mut kept_turn_starts = VecDeque::with_capacity(keep_recent_turns);

    for stored in events {
        if !should_record_compaction_tail_event(&stored.event) {
            continue;
        }
        if matches!(
            &stored.event.payload,
            StorageEventPayload::UserMessage {
                origin: UserMessageOrigin::User,
                ..
            }
        ) {
            kept_turn_starts.push_back(tail_refs.len());
            if kept_turn_starts.len() > keep_recent_turns {
                kept_turn_starts.pop_front();
            }
        }
        tail_refs.push(stored);
    }

    let keep_start = kept_turn_starts.front().copied().unwrap_or(0);
    tail_refs.into_iter().skip(keep_start).cloned().collect()
}

/// 判断事件是否应纳入 compaction tail 记录。
pub fn should_record_compaction_tail_event(event: &StorageEvent) -> bool {
    matches!(
        &event.payload,
        StorageEventPayload::UserMessage { .. }
            | StorageEventPayload::AssistantFinal { .. }
            | StorageEventPayload::ToolCall { .. }
            | StorageEventPayload::ToolResult { .. }
    ) && should_include_in_compaction_tail(event)
}

fn should_include_in_compaction_tail(event: &StorageEvent) -> bool {
    let Some(agent) = event.agent_context() else {
        return true;
    };

    if agent.invocation_kind != Some(InvocationKind::SubRun) {
        return true;
    }

    // 只有语义完整的独立子会话事件才属于子会话自身，应纳入 compaction tail。
    agent.is_independent_sub_run()
}

#[cfg(test)]
mod tests {
    use astrcode_core::{AgentEventContext, StorageEventPayload};

    use super::*;
    use crate::state::test_support::{event, stored};

    #[test]
    fn recent_turn_event_tail_keeps_latest_turn_when_keep_recent_turns_is_zero() {
        let events = vec![
            stored(
                1,
                event(
                    Some("turn-1"),
                    AgentEventContext::default(),
                    StorageEventPayload::UserMessage {
                        content: "first".to_string(),
                        origin: UserMessageOrigin::User,
                        timestamp: chrono::Utc::now(),
                    },
                ),
            ),
            stored(
                2,
                event(
                    Some("turn-1"),
                    AgentEventContext::default(),
                    StorageEventPayload::AssistantFinal {
                        content: "reply-1".to_string(),
                        reasoning_content: None,
                        reasoning_signature: None,
                        timestamp: Some(chrono::Utc::now()),
                    },
                ),
            ),
            stored(
                3,
                event(
                    Some("turn-2"),
                    AgentEventContext::default(),
                    StorageEventPayload::UserMessage {
                        content: "second".to_string(),
                        origin: UserMessageOrigin::User,
                        timestamp: chrono::Utc::now(),
                    },
                ),
            ),
            stored(
                4,
                event(
                    Some("turn-2"),
                    AgentEventContext::default(),
                    StorageEventPayload::AssistantFinal {
                        content: "reply-2".to_string(),
                        reasoning_content: None,
                        reasoning_signature: None,
                        timestamp: Some(chrono::Utc::now()),
                    },
                ),
            ),
        ];

        let tail = recent_turn_event_tail(&events, 0);

        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0].storage_seq, 3);
        assert_eq!(tail[1].storage_seq, 4);
    }

    #[test]
    fn recent_turn_event_tail_excludes_malformed_subrun_events_without_child_session() {
        let malformed_child_agent = AgentEventContext {
            agent_id: Some("agent-child".to_string().into()),
            parent_turn_id: Some("turn-root".to_string().into()),
            agent_profile: Some("explore".to_string()),
            sub_run_id: Some("subrun-malformed".to_string().into()),
            parent_sub_run_id: None,
            invocation_kind: Some(InvocationKind::SubRun),
            storage_mode: Some(astrcode_core::SubRunStorageMode::IndependentSession),
            child_session_id: None,
        };
        let events = vec![
            stored(
                1,
                event(
                    Some("turn-root"),
                    AgentEventContext::default(),
                    StorageEventPayload::UserMessage {
                        content: "root".to_string(),
                        origin: UserMessageOrigin::User,
                        timestamp: chrono::Utc::now(),
                    },
                ),
            ),
            stored(
                2,
                event(
                    Some("turn-root"),
                    AgentEventContext::default(),
                    StorageEventPayload::AssistantFinal {
                        content: "root-answer".to_string(),
                        reasoning_content: None,
                        reasoning_signature: None,
                        timestamp: Some(chrono::Utc::now()),
                    },
                ),
            ),
            stored(
                3,
                event(
                    Some("turn-child"),
                    malformed_child_agent.clone(),
                    StorageEventPayload::UserMessage {
                        content: "child".to_string(),
                        origin: UserMessageOrigin::User,
                        timestamp: chrono::Utc::now(),
                    },
                ),
            ),
            stored(
                4,
                event(
                    Some("turn-child"),
                    malformed_child_agent,
                    StorageEventPayload::AssistantFinal {
                        content: "child-answer".to_string(),
                        reasoning_content: None,
                        reasoning_signature: None,
                        timestamp: Some(chrono::Utc::now()),
                    },
                ),
            ),
        ];

        let tail = recent_turn_event_tail(&events, 1);

        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0].storage_seq, 1);
        assert_eq!(tail[1].storage_seq, 2);
    }

    #[test]
    fn recent_turn_event_tail_keeps_independent_session_subrun_events() {
        let child_agent = AgentEventContext::sub_run(
            "agent-child",
            "turn-root",
            "explore",
            "subrun-independent",
            None,
            astrcode_core::SubRunStorageMode::IndependentSession,
            Some("session-child".to_string().into()),
        );
        let events = vec![
            stored(
                1,
                event(
                    Some("turn-child"),
                    child_agent.clone(),
                    StorageEventPayload::UserMessage {
                        content: "child".to_string(),
                        origin: UserMessageOrigin::User,
                        timestamp: chrono::Utc::now(),
                    },
                ),
            ),
            stored(
                2,
                event(
                    Some("turn-child"),
                    child_agent,
                    StorageEventPayload::AssistantFinal {
                        content: "child-answer".to_string(),
                        reasoning_content: None,
                        reasoning_signature: None,
                        timestamp: Some(chrono::Utc::now()),
                    },
                ),
            ),
        ];

        let tail = recent_turn_event_tail(&events, 1);

        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0].storage_seq, 1);
        assert_eq!(tail[1].storage_seq, 2);
    }
}
