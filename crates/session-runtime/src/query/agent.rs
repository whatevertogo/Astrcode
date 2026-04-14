//! Agent 只读观察投影。
//!
//! Why: `observe` 负责订阅语义，`agent query` 负责生成同步快照，
//! 两者不要再混在同一个模块里。

use std::collections::{HashMap, HashSet};

use astrcode_core::{
    AgentLifecycleStatus, AgentMailboxEnvelope, AgentState, LlmMessage, MailboxProjection,
    StorageEventPayload, StoredEvent, UserMessageOrigin,
};

#[derive(Debug, Clone)]
pub struct AgentObserveSnapshot {
    pub phase: astrcode_core::Phase,
    pub turn_count: u32,
    pub pending_message_count: usize,
    pub active_task: Option<String>,
    pub pending_task: Option<String>,
    pub recent_mailbox_messages: Vec<String>,
    pub last_output: Option<String>,
}

pub fn build_agent_observe_snapshot(
    lifecycle_status: AgentLifecycleStatus,
    projected: &AgentState,
    mailbox_projection: &MailboxProjection,
    stored_events: &[StoredEvent],
    target_agent_id: &str,
) -> AgentObserveSnapshot {
    let mailbox_messages = collect_mailbox_messages(stored_events, target_agent_id);
    let pending_message_count = mailbox_projection.pending_delivery_ids.len();

    AgentObserveSnapshot {
        phase: projected.phase,
        turn_count: projected.turn_count as u32,
        pending_message_count,
        active_task: active_task_summary(
            lifecycle_status,
            &projected.messages,
            mailbox_projection,
            &mailbox_messages,
        ),
        pending_task: pending_task_summary(mailbox_projection, &mailbox_messages),
        recent_mailbox_messages: recent_mailbox_message_summaries(stored_events, target_agent_id),
        last_output: extract_last_output(&projected.messages),
    }
}

fn extract_last_output(messages: &[LlmMessage]) -> Option<String> {
    messages.iter().rev().find_map(|msg| match msg {
        LlmMessage::Assistant { content, .. } if !content.is_empty() => {
            let char_count = content.chars().count();
            if char_count > 200 {
                Some(content.chars().take(200).collect::<String>() + "...")
            } else {
                Some(content.clone())
            }
        },
        _ => None,
    })
}

fn active_task_summary(
    lifecycle_status: AgentLifecycleStatus,
    messages: &[LlmMessage],
    mailbox_projection: &MailboxProjection,
    mailbox_messages: &HashMap<String, AgentMailboxEnvelope>,
) -> Option<String> {
    if let Some(summary) = first_delivery_summary(
        mailbox_projection.active_delivery_ids.iter(),
        mailbox_messages,
    ) {
        return Some(summary);
    }

    if matches!(
        lifecycle_status,
        AgentLifecycleStatus::Pending | AgentLifecycleStatus::Running
    ) {
        return latest_user_task_summary(messages);
    }

    None
}

fn pending_task_summary(
    mailbox_projection: &MailboxProjection,
    mailbox_messages: &HashMap<String, AgentMailboxEnvelope>,
) -> Option<String> {
    let active_ids: HashSet<_> = mailbox_projection
        .active_delivery_ids
        .iter()
        .cloned()
        .collect();

    first_delivery_summary(
        mailbox_projection
            .pending_delivery_ids
            .iter()
            .filter(|delivery_id| !active_ids.contains(*delivery_id)),
        mailbox_messages,
    )
}

fn first_delivery_summary<'a>(
    delivery_ids: impl IntoIterator<Item = &'a String>,
    mailbox_messages: &HashMap<String, AgentMailboxEnvelope>,
) -> Option<String> {
    delivery_ids.into_iter().find_map(|delivery_id| {
        mailbox_messages
            .get(delivery_id)
            .and_then(|envelope| summarize_task_text(&envelope.message))
    })
}

fn latest_user_task_summary(messages: &[LlmMessage]) -> Option<String> {
    messages.iter().rev().find_map(|message| match message {
        LlmMessage::User { content, origin } if *origin == UserMessageOrigin::User => {
            summarize_task_text(content)
        },
        _ => None,
    })
}

fn collect_mailbox_messages(
    stored_events: &[StoredEvent],
    target_agent_id: &str,
) -> HashMap<String, AgentMailboxEnvelope> {
    let mut messages = HashMap::new();
    for stored in stored_events {
        if let StorageEventPayload::AgentMailboxQueued { payload } = &stored.event.payload {
            if payload.envelope.to_agent_id == target_agent_id {
                messages.insert(
                    payload.envelope.delivery_id.clone(),
                    payload.envelope.clone(),
                );
            }
        }
    }
    messages
}

fn recent_mailbox_message_summaries(
    stored_events: &[StoredEvent],
    target_agent_id: &str,
) -> Vec<String> {
    const MAX_RECENT_MAILBOX_MESSAGES: usize = 3;

    stored_events
        .iter()
        .filter_map(|stored| match &stored.event.payload {
            StorageEventPayload::AgentMailboxQueued { payload }
                if payload.envelope.to_agent_id == target_agent_id =>
            {
                summarize_task_text(&payload.envelope.message)
            },
            _ => None,
        })
        .rev()
        .take(MAX_RECENT_MAILBOX_MESSAGES)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn summarize_task_text(text: &str) -> Option<String> {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = normalized.trim();
    if trimmed.is_empty() {
        return None;
    }

    const MAX_TASK_SUMMARY_CHARS: usize = 120;
    let char_count = trimmed.chars().count();
    if char_count <= MAX_TASK_SUMMARY_CHARS {
        return Some(trimmed.to_string());
    }

    Some(
        trimmed
            .chars()
            .take(MAX_TASK_SUMMARY_CHARS)
            .collect::<String>()
            + "...",
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use astrcode_core::{
        AgentEventContext, AgentLifecycleStatus, AgentMailboxEnvelope, AgentState, LlmMessage,
        MailboxProjection, MailboxQueuedPayload, Phase, StorageEvent, StorageEventPayload,
        StoredEvent,
    };

    use super::{
        build_agent_observe_snapshot, extract_last_output, recent_mailbox_message_summaries,
        summarize_task_text,
    };

    fn queued_mailbox_event(
        storage_seq: u64,
        delivery_id: &str,
        target_agent_id: &str,
        message: &str,
    ) -> StoredEvent {
        StoredEvent {
            storage_seq,
            event: StorageEvent {
                turn_id: Some("turn-parent".to_string()),
                agent: AgentEventContext::default(),
                payload: StorageEventPayload::AgentMailboxQueued {
                    payload: MailboxQueuedPayload {
                        envelope: AgentMailboxEnvelope {
                            delivery_id: delivery_id.to_string(),
                            from_agent_id: "parent".to_string(),
                            to_agent_id: target_agent_id.to_string(),
                            message: message.to_string(),
                            queued_at: chrono::Utc::now(),
                            sender_lifecycle_status: AgentLifecycleStatus::Idle,
                            sender_last_turn_outcome: None,
                            sender_open_session_id: "session-parent".to_string(),
                        },
                    },
                },
            },
        }
    }

    #[test]
    fn summarize_task_text_trims_and_truncates() {
        assert_eq!(
            summarize_task_text("  review   the   mailbox state  "),
            Some("review the mailbox state".to_string())
        );
        assert!(summarize_task_text("   ").is_none());
        let long = "a".repeat(150);
        assert_eq!(
            summarize_task_text(&long),
            Some(format!("{}...", "a".repeat(120)))
        );
    }

    #[test]
    fn extract_last_output_ignores_empty_assistant() {
        let messages = vec![
            LlmMessage::Assistant {
                content: String::new(),
                tool_calls: Vec::new(),
                reasoning: None,
            },
            LlmMessage::Assistant {
                content: "最后输出".to_string(),
                tool_calls: Vec::new(),
                reasoning: None,
            },
        ];
        assert_eq!(extract_last_output(&messages), Some("最后输出".to_string()));
    }

    #[test]
    fn recent_mailbox_message_summaries_returns_only_tail() {
        let stored_events = vec![
            queued_mailbox_event(1, "d1", "child-1", "第一条"),
            queued_mailbox_event(2, "d2", "child-1", "第二条"),
            queued_mailbox_event(3, "d3", "child-1", "第三条"),
            queued_mailbox_event(4, "d4", "child-1", "第四条"),
        ];

        assert_eq!(
            recent_mailbox_message_summaries(&stored_events, "child-1"),
            vec![
                "第二条".to_string(),
                "第三条".to_string(),
                "第四条".to_string()
            ]
        );
    }

    #[test]
    fn build_agent_observe_snapshot_includes_recent_mailbox_tail() {
        let stored_events = vec![
            queued_mailbox_event(1, "d1", "child-1", "第一条"),
            queued_mailbox_event(2, "d2", "child-1", "第二条"),
        ];
        let snapshot = build_agent_observe_snapshot(
            AgentLifecycleStatus::Idle,
            &AgentState {
                session_id: "session-child".to_string(),
                working_dir: PathBuf::from("/workspace/demo"),
                messages: Vec::new(),
                phase: Phase::Idle,
                turn_count: 2,
                last_assistant_at: None,
            },
            &MailboxProjection::default(),
            &stored_events,
            "child-1",
        );

        assert_eq!(
            snapshot.recent_mailbox_messages,
            vec!["第一条".to_string(), "第二条".to_string()]
        );
    }
}
