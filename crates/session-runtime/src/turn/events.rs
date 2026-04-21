#[cfg(test)]
use astrcode_core::ToolOutputStream;
use astrcode_core::{
    AgentEventContext, CompactAppliedMeta, CompactTrigger, LlmUsage, PromptMetricsPayload,
    StorageEvent, StorageEventPayload, ToolCallRequest, ToolExecutionResult, TurnTerminalKind,
    UserMessageOrigin, ports::PromptBuildCacheMetrics,
};
use chrono::{DateTime, Utc};

use crate::context_window::token_usage::PromptTokenSnapshot;

fn saturating_u32(value: usize) -> u32 {
    value.min(u32::MAX as usize) as u32
}

pub(crate) struct CompactAppliedStats {
    pub meta: CompactAppliedMeta,
    pub preserved_recent_turns: usize,
    pub pre_tokens: usize,
    pub post_tokens_estimate: usize,
    pub messages_removed: usize,
    pub tokens_freed: usize,
}

pub(crate) fn session_start_event(
    session_id: impl Into<String>,
    working_dir: impl Into<String>,
    parent_session_id: Option<String>,
    parent_storage_seq: Option<u64>,
    timestamp: DateTime<Utc>,
) -> StorageEvent {
    StorageEvent {
        turn_id: None,
        agent: AgentEventContext::default(),
        payload: StorageEventPayload::SessionStart {
            session_id: session_id.into(),
            timestamp,
            working_dir: working_dir.into(),
            parent_session_id,
            parent_storage_seq,
        },
    }
}

pub(crate) fn user_message_event(
    turn_id: &str,
    agent: &AgentEventContext,
    content: String,
    origin: UserMessageOrigin,
    timestamp: DateTime<Utc>,
) -> StorageEvent {
    StorageEvent {
        turn_id: Some(turn_id.to_string()),
        agent: agent.clone(),
        payload: StorageEventPayload::UserMessage {
            content,
            origin,
            timestamp,
        },
    }
}

pub(crate) fn assistant_final_event(
    turn_id: &str,
    agent: &AgentEventContext,
    content: String,
    reasoning_content: Option<String>,
    reasoning_signature: Option<String>,
    timestamp: Option<DateTime<Utc>>,
) -> StorageEvent {
    StorageEvent {
        turn_id: Some(turn_id.to_string()),
        agent: agent.clone(),
        payload: StorageEventPayload::AssistantFinal {
            content,
            reasoning_content,
            reasoning_signature,
            timestamp,
        },
    }
}

pub(crate) fn turn_done_event(
    turn_id: &str,
    agent: &AgentEventContext,
    reason: Option<String>,
    timestamp: DateTime<Utc>,
) -> StorageEvent {
    StorageEvent {
        turn_id: Some(turn_id.to_string()),
        agent: agent.clone(),
        payload: StorageEventPayload::TurnDone {
            timestamp,
            terminal_kind: TurnTerminalKind::from_legacy_reason(reason.as_deref()),
            reason,
        },
    }
}

pub(crate) fn error_event(
    turn_id: Option<&str>,
    agent: &AgentEventContext,
    message: String,
    timestamp: Option<DateTime<Utc>>,
) -> StorageEvent {
    StorageEvent {
        turn_id: turn_id.map(str::to_string),
        agent: agent.clone(),
        payload: StorageEventPayload::Error { message, timestamp },
    }
}

pub(crate) fn compact_applied_event(
    turn_id: Option<&str>,
    agent: &AgentEventContext,
    trigger: CompactTrigger,
    summary: String,
    stats: CompactAppliedStats,
    timestamp: DateTime<Utc>,
) -> StorageEvent {
    StorageEvent {
        turn_id: turn_id.map(str::to_string),
        agent: agent.clone(),
        payload: StorageEventPayload::CompactApplied {
            trigger,
            summary,
            meta: stats.meta,
            preserved_recent_turns: saturating_u32(stats.preserved_recent_turns),
            pre_tokens: saturating_u32(stats.pre_tokens),
            post_tokens_estimate: saturating_u32(stats.post_tokens_estimate),
            messages_removed: saturating_u32(stats.messages_removed),
            tokens_freed: saturating_u32(stats.tokens_freed),
            timestamp,
        },
    }
}

pub(crate) fn prompt_metrics_event(
    turn_id: &str,
    agent: &AgentEventContext,
    step_index: usize,
    snapshot: PromptTokenSnapshot,
    truncated_tool_results: usize,
    cache_metrics: PromptBuildCacheMetrics,
    provider_cache_metrics_supported: bool,
) -> StorageEvent {
    StorageEvent {
        turn_id: Some(turn_id.to_string()),
        agent: agent.clone(),
        payload: StorageEventPayload::PromptMetrics {
            metrics: PromptMetricsPayload {
                step_index: saturating_u32(step_index),
                estimated_tokens: saturating_u32(snapshot.context_tokens),
                context_window: saturating_u32(snapshot.context_window),
                effective_window: saturating_u32(snapshot.effective_window),
                threshold_tokens: saturating_u32(snapshot.threshold_tokens),
                truncated_tool_results: saturating_u32(truncated_tool_results),
                provider_input_tokens: None,
                provider_output_tokens: None,
                cache_creation_input_tokens: None,
                cache_read_input_tokens: None,
                provider_cache_metrics_supported,
                prompt_cache_reuse_hits: cache_metrics.reuse_hits,
                prompt_cache_reuse_misses: cache_metrics.reuse_misses,
                prompt_cache_unchanged_layers: cache_metrics.unchanged_layers,
            },
        },
    }
}

pub(crate) fn apply_prompt_metrics_usage(
    events: &mut [StorageEvent],
    step_index: usize,
    usage: Option<LlmUsage>,
) {
    let Some(usage) = usage else {
        return;
    };

    let step_index = saturating_u32(step_index);
    let Some(StorageEvent {
        payload: StorageEventPayload::PromptMetrics { metrics },
        ..
    }) = events.iter_mut().rev().find(|event| {
        matches!(
            &event.payload,
            StorageEventPayload::PromptMetrics { metrics }
                if metrics.step_index == step_index
        )
    })
    else {
        return;
    };

    metrics.provider_input_tokens = Some(saturating_u32(usage.input_tokens));
    metrics.provider_output_tokens = Some(saturating_u32(usage.output_tokens));
    metrics.cache_creation_input_tokens = Some(saturating_u32(usage.cache_creation_input_tokens));
    metrics.cache_read_input_tokens = Some(saturating_u32(usage.cache_read_input_tokens));
}

pub(crate) fn tool_call_event(
    turn_id: &str,
    agent: &AgentEventContext,
    tool_call: &ToolCallRequest,
) -> StorageEvent {
    StorageEvent {
        turn_id: Some(turn_id.to_string()),
        agent: agent.clone(),
        payload: StorageEventPayload::ToolCall {
            tool_call_id: tool_call.id.clone(),
            tool_name: tool_call.name.clone(),
            args: tool_call.args.clone(),
        },
    }
}

#[cfg(test)]
pub(crate) fn tool_call_delta_event(
    turn_id: &str,
    agent: &AgentEventContext,
    tool_call_id: String,
    tool_name: String,
    stream: ToolOutputStream,
    delta: String,
) -> StorageEvent {
    StorageEvent {
        turn_id: Some(turn_id.to_string()),
        agent: agent.clone(),
        payload: StorageEventPayload::ToolCallDelta {
            tool_call_id,
            tool_name,
            stream,
            delta,
        },
    }
}

pub(crate) fn tool_result_event(
    turn_id: &str,
    agent: &AgentEventContext,
    result: &ToolExecutionResult,
) -> StorageEvent {
    StorageEvent {
        turn_id: Some(turn_id.to_string()),
        agent: agent.clone(),
        payload: StorageEventPayload::ToolResult {
            tool_call_id: result.tool_call_id.clone(),
            tool_name: result.tool_name.clone(),
            output: result.output.clone(),
            success: result.ok,
            error: result.error.clone(),
            metadata: result.metadata.clone(),
            continuation: result.continuation.clone(),
            duration_ms: result.duration_ms,
        },
    }
}

pub(crate) fn tool_result_reference_applied_event(
    turn_id: &str,
    agent: &AgentEventContext,
    tool_call_id: &str,
    persisted_output: &astrcode_core::PersistedToolOutput,
    replacement: &str,
    original_bytes: u64,
) -> StorageEvent {
    StorageEvent {
        turn_id: Some(turn_id.to_string()),
        agent: agent.clone(),
        payload: StorageEventPayload::ToolResultReferenceApplied {
            tool_call_id: tool_call_id.to_string(),
            persisted_output: persisted_output.clone(),
            replacement: replacement.to_string(),
            original_bytes,
        },
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{
        AgentEventContext, CompactAppliedMeta, CompactMode, CompactTrigger, LlmUsage,
        StorageEventPayload, ToolCallRequest, ToolExecutionResult, ToolOutputStream,
        UserMessageOrigin, ports::PromptBuildCacheMetrics,
    };
    use chrono::{TimeZone, Utc};
    use serde_json::json;

    use super::{
        CompactAppliedStats, apply_prompt_metrics_usage, assistant_final_event,
        compact_applied_event, error_event, prompt_metrics_event, session_start_event,
        tool_call_delta_event, tool_call_event, tool_result_event, turn_done_event,
        user_message_event,
    };
    use crate::context_window::token_usage::PromptTokenSnapshot;

    #[test]
    fn session_start_event_preserves_parent_lineage_fields() {
        let timestamp = Utc::now();
        let event = session_start_event(
            "session-child",
            "/workspace/project",
            Some("session-parent".to_string()),
            Some(42),
            timestamp,
        );

        assert!(event.turn_id.is_none());
        assert!(matches!(
            event.payload,
            StorageEventPayload::SessionStart {
                session_id,
                timestamp: event_timestamp,
                working_dir,
                parent_session_id,
                parent_storage_seq,
            } if session_id == "session-child"
                && event_timestamp == timestamp
                && working_dir == "/workspace/project"
                && parent_session_id.as_deref() == Some("session-parent")
                && parent_storage_seq == Some(42)
        ));
    }

    #[test]
    fn user_message_event_preserves_origin_and_timestamp() {
        let timestamp = Utc
            .with_ymd_and_hms(2026, 4, 14, 10, 0, 0)
            .single()
            .expect("timestamp should build");
        let agent = AgentEventContext::root_execution("root-agent", "planner");
        let event = user_message_event(
            "turn-user-1",
            &agent,
            "please inspect the diff".to_string(),
            UserMessageOrigin::ReactivationPrompt,
            timestamp,
        );

        assert_eq!(event.turn_id.as_deref(), Some("turn-user-1"));
        assert_eq!(event.agent, agent);
        assert!(matches!(
            event.payload,
            StorageEventPayload::UserMessage {
                content,
                origin,
                timestamp: event_timestamp,
            } if content == "please inspect the diff"
                && origin == UserMessageOrigin::ReactivationPrompt
                && event_timestamp == timestamp
        ));
    }

    #[test]
    fn assistant_final_event_preserves_reasoning_fields_and_optional_timestamp() {
        let timestamp = Utc
            .with_ymd_and_hms(2026, 4, 14, 10, 5, 0)
            .single()
            .expect("timestamp should build");
        let agent = AgentEventContext::root_execution("root-agent", "planner");
        let event = assistant_final_event(
            "turn-assistant-1",
            &agent,
            "done".to_string(),
            Some("reasoned path".to_string()),
            Some("sig-1".to_string()),
            Some(timestamp),
        );

        assert_eq!(event.turn_id.as_deref(), Some("turn-assistant-1"));
        assert_eq!(event.agent, agent);
        assert!(matches!(
            event.payload,
            StorageEventPayload::AssistantFinal {
                content,
                reasoning_content,
                reasoning_signature,
                timestamp: event_timestamp,
            } if content == "done"
                && reasoning_content.as_deref() == Some("reasoned path")
                && reasoning_signature.as_deref() == Some("sig-1")
                && event_timestamp == Some(timestamp)
        ));
    }

    #[test]
    fn turn_done_event_preserves_reason_and_timestamp() {
        let timestamp = Utc
            .with_ymd_and_hms(2026, 4, 14, 10, 10, 0)
            .single()
            .expect("timestamp should build");
        let agent = AgentEventContext::root_execution("root-agent", "planner");
        let event = turn_done_event(
            "turn-done-1",
            &agent,
            Some("completed".to_string()),
            timestamp,
        );

        assert_eq!(event.turn_id.as_deref(), Some("turn-done-1"));
        assert_eq!(event.agent, agent);
        assert!(matches!(
            event.payload,
            StorageEventPayload::TurnDone {
                timestamp: event_timestamp,
                terminal_kind,
                reason,
            } if event_timestamp == timestamp
                && terminal_kind == Some(astrcode_core::TurnTerminalKind::Completed)
                && reason.as_deref() == Some("completed")
        ));
    }

    #[test]
    fn error_event_supports_missing_turn_id_and_timestamp() {
        let agent = AgentEventContext::root_execution("root-agent", "planner");
        let event = error_event(None, &agent, "compact failed".to_string(), None);

        assert!(event.turn_id.is_none());
        assert_eq!(event.agent, agent);
        assert!(matches!(
            event.payload,
            StorageEventPayload::Error { message, timestamp }
                if message == "compact failed" && timestamp.is_none()
        ));
    }

    #[test]
    fn compact_applied_event_saturates_large_stats_and_preserves_metadata() {
        let timestamp = Utc
            .with_ymd_and_hms(2026, 4, 14, 9, 30, 0)
            .single()
            .expect("timestamp should build");
        let agent = AgentEventContext::root_execution("root-agent", "planner");
        let event = compact_applied_event(
            Some("turn-compact-1"),
            &agent,
            CompactTrigger::Auto,
            "condensed older work".to_string(),
            CompactAppliedStats {
                meta: CompactAppliedMeta {
                    mode: CompactMode::RetrySalvage,
                    instructions_present: true,
                    fallback_used: true,
                    retry_count: u32::MAX,
                    input_units: u32::MAX,
                    output_summary_chars: u32::MAX,
                },
                preserved_recent_turns: usize::MAX,
                pre_tokens: usize::MAX,
                post_tokens_estimate: 512,
                messages_removed: usize::MAX,
                tokens_freed: usize::MAX,
            },
            timestamp,
        );

        assert_eq!(event.turn_id.as_deref(), Some("turn-compact-1"));
        assert_eq!(event.agent, agent);
        assert!(matches!(
            event.payload,
            StorageEventPayload::CompactApplied {
                trigger,
                summary,
                meta,
                preserved_recent_turns,
                pre_tokens,
                post_tokens_estimate,
                messages_removed,
                tokens_freed,
                timestamp: event_timestamp,
            } if trigger == CompactTrigger::Auto
                && summary == "condensed older work"
                && meta.mode == CompactMode::RetrySalvage
                && meta.instructions_present
                && meta.fallback_used
                && meta.retry_count == u32::MAX
                && meta.input_units == u32::MAX
                && meta.output_summary_chars == u32::MAX
                && preserved_recent_turns == u32::MAX
                && pre_tokens == u32::MAX
                && post_tokens_estimate == 512
                && messages_removed == u32::MAX
                && tokens_freed == u32::MAX
                && event_timestamp == timestamp
        ));
    }

    #[test]
    fn prompt_metrics_event_maps_snapshot_fields_without_provider_metrics() {
        let agent = AgentEventContext::root_execution("root-agent", "planner");
        let event = prompt_metrics_event(
            "turn-prompt-1",
            &agent,
            7,
            PromptTokenSnapshot {
                context_tokens: 12_345,
                budget_tokens: 9_999,
                context_window: 128_000,
                effective_window: 108_000,
                threshold_tokens: 97_200,
                remaining_context_tokens: 95_655,
                reserved_context_size: 20_000,
            },
            3,
            PromptBuildCacheMetrics {
                reuse_hits: 4,
                reuse_misses: 1,
                unchanged_layers: vec![astrcode_core::SystemPromptLayer::Stable],
            },
            true,
        );

        assert_eq!(event.turn_id.as_deref(), Some("turn-prompt-1"));
        assert_eq!(event.agent, agent);
        assert!(matches!(
            event.payload,
            StorageEventPayload::PromptMetrics { metrics }
                if metrics.step_index == 7
                    && metrics.estimated_tokens == 12_345
                    && metrics.context_window == 128_000
                    && metrics.effective_window == 108_000
                    && metrics.threshold_tokens == 97_200
                    && metrics.truncated_tool_results == 3
                    && metrics.prompt_cache_unchanged_layers
                        == vec![astrcode_core::SystemPromptLayer::Stable]
                    && metrics.provider_input_tokens.is_none()
                    && metrics.provider_output_tokens.is_none()
                    && metrics.cache_creation_input_tokens.is_none()
                    && metrics.cache_read_input_tokens.is_none()
                    && metrics.provider_cache_metrics_supported
                    && metrics.prompt_cache_reuse_hits == 4
                    && metrics.prompt_cache_reuse_misses == 1
        ));
    }

    #[test]
    fn apply_prompt_metrics_usage_backfills_provider_cache_fields() {
        let agent = AgentEventContext::root_execution("root-agent", "planner");
        let mut events = vec![prompt_metrics_event(
            "turn-prompt-1",
            &agent,
            2,
            PromptTokenSnapshot {
                context_tokens: 1_024,
                budget_tokens: 900,
                context_window: 128_000,
                effective_window: 108_000,
                threshold_tokens: 97_200,
                remaining_context_tokens: 106_976,
                reserved_context_size: 20_000,
            },
            0,
            PromptBuildCacheMetrics::default(),
            true,
        )];

        apply_prompt_metrics_usage(
            &mut events,
            2,
            Some(LlmUsage {
                input_tokens: 900,
                output_tokens: 120,
                cache_creation_input_tokens: 700,
                cache_read_input_tokens: 650,
            }),
        );

        assert!(matches!(
            &events[0].payload,
            StorageEventPayload::PromptMetrics { metrics }
                if metrics.provider_input_tokens == Some(900)
                    && metrics.provider_output_tokens == Some(120)
                    && metrics.cache_creation_input_tokens == Some(700)
                    && metrics.cache_read_input_tokens == Some(650)
        ));
    }

    #[test]
    fn tool_call_event_preserves_request_id_name_and_args() {
        let agent = AgentEventContext::root_execution("root-agent", "planner");
        let tool_call = ToolCallRequest {
            id: "call-42".to_string(),
            name: "readFile".to_string(),
            args: json!({
                "path": "src/lib.rs",
                "offset": 10
            }),
        };
        let event = tool_call_event("turn-tool-call-1", &agent, &tool_call);

        assert_eq!(event.turn_id.as_deref(), Some("turn-tool-call-1"));
        assert_eq!(event.agent, agent);
        assert!(matches!(
            event.payload,
            StorageEventPayload::ToolCall {
                tool_call_id,
                tool_name,
                args,
            } if tool_call_id == "call-42"
                && tool_name == "readFile"
                && args == json!({
                    "path": "src/lib.rs",
                    "offset": 10
                })
        ));
    }

    #[test]
    fn tool_call_delta_event_preserves_stream_and_delta_text() {
        let agent = AgentEventContext::root_execution("root-agent", "planner");
        let event = tool_call_delta_event(
            "turn-tool-call-1",
            &agent,
            "call-42".to_string(),
            "readFile".to_string(),
            ToolOutputStream::Stderr,
            "permission denied".to_string(),
        );

        assert_eq!(event.turn_id.as_deref(), Some("turn-tool-call-1"));
        assert_eq!(event.agent, agent);
        assert!(matches!(
            event.payload,
            StorageEventPayload::ToolCallDelta {
                tool_call_id,
                tool_name,
                stream,
                delta,
            } if tool_call_id == "call-42"
                && tool_name == "readFile"
                && stream == ToolOutputStream::Stderr
                && delta == "permission denied"
        ));
    }

    #[test]
    fn tool_result_event_preserves_error_and_metadata_payload() {
        let agent = AgentEventContext::root_execution("root-agent", "planner");
        let result = ToolExecutionResult {
            tool_call_id: "call-7".to_string(),
            tool_name: "readFile".to_string(),
            ok: false,
            output: "partial output".to_string(),
            error: Some("permission denied".to_string()),
            metadata: Some(json!({
                "path": "/workspace/src/lib.rs",
                "truncated": true
            })),
            continuation: None,
            duration_ms: 88,
            truncated: true,
        };
        let event = tool_result_event("turn-tool-1", &agent, &result);

        assert_eq!(event.turn_id.as_deref(), Some("turn-tool-1"));
        assert_eq!(event.agent, agent);
        assert!(matches!(
            event.payload,
            StorageEventPayload::ToolResult {
                tool_call_id,
                tool_name,
                output,
                success,
                error,
                metadata,
                continuation: _,
                duration_ms,
            } if tool_call_id == "call-7"
                && tool_name == "readFile"
                && output == "partial output"
                && !success
                && error.as_deref() == Some("permission denied")
                && metadata == Some(json!({
                    "path": "/workspace/src/lib.rs",
                    "truncated": true
                }))
                && duration_ms == 88
        ));
    }
}
