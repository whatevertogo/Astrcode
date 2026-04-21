use astrcode_core::{
    AgentEventContext, CompactTrigger, LlmMessage, StorageEvent, StorageEventPayload,
    UserMessageOrigin,
};

use crate::{
    context_window::{
        compaction::CompactResult,
        file_access::{FileAccessTracker, FileRecoveryConfig},
    },
    turn::events::{CompactAppliedStats, compact_applied_event},
};

pub(crate) fn build_post_compact_events(
    turn_id: Option<&str>,
    agent: &AgentEventContext,
    trigger: CompactTrigger,
    compaction: &CompactResult,
) -> Vec<StorageEvent> {
    let mut events = vec![compact_applied_event(
        turn_id,
        agent,
        trigger,
        compaction.summary.clone(),
        CompactAppliedStats {
            meta: compaction.meta.clone(),
            preserved_recent_turns: compaction.preserved_recent_turns,
            pre_tokens: compaction.pre_tokens,
            post_tokens_estimate: compaction.post_tokens_estimate,
            messages_removed: compaction.messages_removed,
            tokens_freed: compaction.tokens_freed,
        },
        compaction.timestamp,
    )];

    if let Some(digest) = compaction.recent_user_context_digest.clone() {
        events.push(StorageEvent {
            turn_id: turn_id.map(str::to_string),
            agent: agent.clone(),
            payload: StorageEventPayload::UserMessage {
                content: digest,
                origin: UserMessageOrigin::RecentUserContextDigest,
                timestamp: compaction.timestamp,
            },
        });
    }
    for content in &compaction.recent_user_context_messages {
        events.push(StorageEvent {
            turn_id: turn_id.map(str::to_string),
            agent: agent.clone(),
            payload: StorageEventPayload::UserMessage {
                content: content.clone(),
                origin: UserMessageOrigin::RecentUserContext,
                timestamp: compaction.timestamp,
            },
        });
    }

    events
}

pub(crate) fn build_post_compact_recovery_messages(
    file_access_tracker: &FileAccessTracker,
    config: FileRecoveryConfig,
) -> Vec<LlmMessage> {
    file_access_tracker.build_recovery_messages(config)
}

#[cfg(test)]
mod tests {
    use astrcode_core::{
        AgentEventContext, CompactAppliedMeta, CompactMode, CompactTrigger, StorageEventPayload,
    };
    use chrono::{TimeZone, Utc};

    use super::build_post_compact_events;

    #[test]
    fn build_post_compact_events_emits_summary_and_recent_user_context() {
        let timestamp = Utc
            .with_ymd_and_hms(2026, 4, 21, 11, 0, 0)
            .single()
            .expect("timestamp should build");
        let events = build_post_compact_events(
            Some("turn-1"),
            &AgentEventContext::default(),
            CompactTrigger::Manual,
            &crate::context_window::compaction::CompactResult {
                messages: Vec::new(),
                summary: "summary".to_string(),
                recent_user_context_digest: Some("digest".to_string()),
                recent_user_context_messages: vec!["ctx-1".to_string()],
                meta: CompactAppliedMeta {
                    mode: CompactMode::Full,
                    instructions_present: false,
                    fallback_used: false,
                    retry_count: 0,
                    input_units: 0,
                    output_summary_chars: 7,
                },
                preserved_recent_turns: 1,
                pre_tokens: 10,
                post_tokens_estimate: 5,
                messages_removed: 2,
                tokens_freed: 5,
                timestamp,
            },
        );

        assert_eq!(events.len(), 3);
        assert!(matches!(
            &events[0].payload,
            StorageEventPayload::CompactApplied { trigger, summary, .. }
                if *trigger == CompactTrigger::Manual && summary == "summary"
        ));
        assert!(matches!(
            &events[1].payload,
            StorageEventPayload::UserMessage { origin, content, .. }
                if *origin == astrcode_core::UserMessageOrigin::RecentUserContextDigest
                    && content == "digest"
        ));
        assert!(matches!(
            &events[2].payload,
            StorageEventPayload::UserMessage { origin, content, .. }
                if *origin == astrcode_core::UserMessageOrigin::RecentUserContext
                    && content == "ctx-1"
        ));
    }
}
