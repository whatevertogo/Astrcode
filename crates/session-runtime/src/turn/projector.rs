use astrcode_core::{
    LlmMessage, StorageEventPayload, StoredEvent, TurnProjectionSnapshot, TurnTerminalKind,
};

pub(crate) fn apply_turn_projection_event(
    projection: &mut TurnProjectionSnapshot,
    stored: &StoredEvent,
) {
    match &stored.event.payload {
        StorageEventPayload::TurnDone {
            terminal_kind,
            reason,
            ..
        } => {
            projection.terminal_kind = terminal_kind
                .clone()
                .or_else(|| TurnTerminalKind::from_legacy_reason(reason.as_deref()));
        },
        StorageEventPayload::Error { message, .. } => {
            let message = message.trim();
            if !message.is_empty() {
                projection.last_error = Some(message.to_string());
            }
        },
        _ => {},
    }
}

pub(crate) fn project_turn_projection(events: &[StoredEvent]) -> Option<TurnProjectionSnapshot> {
    if events.is_empty() {
        return None;
    }

    let mut projection = TurnProjectionSnapshot {
        terminal_kind: None,
        last_error: None,
    };
    for stored in events {
        apply_turn_projection_event(&mut projection, stored);
    }
    Some(projection)
}

pub(crate) fn has_terminal_projection(projection: Option<&TurnProjectionSnapshot>) -> bool {
    projection.is_some_and(|projection| {
        projection.terminal_kind.is_some() || projection.last_error.is_some()
    })
}

pub(crate) fn last_non_empty_assistant_message(messages: &[LlmMessage]) -> Option<String> {
    messages.iter().rev().find_map(|message| match message {
        LlmMessage::Assistant { content, .. } if !content.trim().is_empty() => {
            Some(content.trim().to_string())
        },
        _ => None,
    })
}

pub(crate) fn last_non_empty_assistant_event(events: &[StoredEvent]) -> Option<String> {
    events
        .iter()
        .rev()
        .find_map(|stored| match &stored.event.payload {
            StorageEventPayload::AssistantFinal { content, .. } if !content.trim().is_empty() => {
                Some(content.trim().to_string())
            },
            _ => None,
        })
}

pub(crate) fn last_non_empty_error_event(events: &[StoredEvent]) -> Option<String> {
    events
        .iter()
        .rev()
        .find_map(|stored| match &stored.event.payload {
            StorageEventPayload::Error { message, .. } if !message.trim().is_empty() => {
                Some(message.trim().to_string())
            },
            _ => None,
        })
}

#[cfg(test)]
mod tests {
    use astrcode_core::{
        AgentEventContext, StorageEvent, StorageEventPayload, StoredEvent, UserMessageOrigin,
    };

    use super::{
        apply_turn_projection_event, has_terminal_projection, last_non_empty_assistant_event,
        last_non_empty_assistant_message, project_turn_projection,
    };

    #[test]
    fn project_turn_projection_preserves_empty_terminal_state_for_observed_turn() {
        let projection = project_turn_projection(&[StoredEvent {
            storage_seq: 1,
            event: StorageEvent {
                turn_id: Some("turn-1".to_string()),
                agent: AgentEventContext::default(),
                payload: StorageEventPayload::UserMessage {
                    content: "hello".to_string(),
                    origin: UserMessageOrigin::User,
                    timestamp: chrono::Utc::now(),
                },
            },
        }])
        .expect("projection should exist");

        assert!(projection.terminal_kind.is_none());
        assert!(projection.last_error.is_none());
    }

    #[test]
    fn apply_turn_projection_event_projects_legacy_reason() {
        let mut projection = astrcode_core::TurnProjectionSnapshot {
            terminal_kind: None,
            last_error: None,
        };

        apply_turn_projection_event(
            &mut projection,
            &StoredEvent {
                storage_seq: 1,
                event: StorageEvent {
                    turn_id: Some("turn-1".to_string()),
                    agent: AgentEventContext::default(),
                    payload: StorageEventPayload::TurnDone {
                        timestamp: chrono::Utc::now(),
                        terminal_kind: None,
                        reason: Some("completed".to_string()),
                    },
                },
            },
        );

        assert_eq!(
            projection.terminal_kind,
            Some(astrcode_core::TurnTerminalKind::Completed)
        );
    }

    #[test]
    fn has_terminal_projection_detects_terminal_kind() {
        assert!(has_terminal_projection(Some(
            &astrcode_core::TurnProjectionSnapshot {
                terminal_kind: Some(astrcode_core::TurnTerminalKind::Completed),
                last_error: None,
            }
        )));
    }

    #[test]
    fn last_non_empty_assistant_message_skips_blank_entries() {
        let summary = last_non_empty_assistant_message(&[
            astrcode_core::LlmMessage::Assistant {
                content: "  ".to_string(),
                tool_calls: Vec::new(),
                reasoning: None,
            },
            astrcode_core::LlmMessage::Assistant {
                content: "ok".to_string(),
                tool_calls: Vec::new(),
                reasoning: None,
            },
        ]);

        assert_eq!(summary.as_deref(), Some("ok"));
    }

    #[test]
    fn last_non_empty_assistant_event_skips_blank_entries() {
        let summary = last_non_empty_assistant_event(&[
            StoredEvent {
                storage_seq: 1,
                event: StorageEvent {
                    turn_id: Some("turn-1".to_string()),
                    agent: AgentEventContext::default(),
                    payload: StorageEventPayload::AssistantFinal {
                        content: "  ".to_string(),
                        reasoning_content: None,
                        reasoning_signature: None,
                        timestamp: Some(chrono::Utc::now()),
                    },
                },
            },
            StoredEvent {
                storage_seq: 2,
                event: StorageEvent {
                    turn_id: Some("turn-1".to_string()),
                    agent: AgentEventContext::default(),
                    payload: StorageEventPayload::AssistantFinal {
                        content: "ready".to_string(),
                        reasoning_content: None,
                        reasoning_signature: None,
                        timestamp: Some(chrono::Utc::now()),
                    },
                },
            },
        ]);

        assert_eq!(summary.as_deref(), Some("ready"));
    }
}
