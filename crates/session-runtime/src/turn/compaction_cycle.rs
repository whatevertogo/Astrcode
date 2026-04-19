//! 压缩周期
//!
//! 封装 reactive compact 错误恢复逻辑。
//!
//! 当 LLM 返回 prompt-too-long 错误时，自动触发上下文压缩。
//! 这与 proactive compact（基于阈值的预防性压缩）不同——
//! reactive compact 是 LLM 实际拒绝后的被动恢复。
//!
//! ## 重试策略
//!
//! 最多重试 `MAX_REACTIVE_COMPACT_ATTEMPTS` 次，每次压缩会减少历史消息。
//! 超过上限则终止 turn，避免无限循环。

use astrcode_core::{
    AgentEventContext, CancelToken, CompactTrigger, LlmMessage, PromptFactsProvider, Result,
    StorageEvent,
};
use astrcode_kernel::KernelGateway;

use crate::{
    context_window::{
        ContextWindowSettings,
        compaction::{CompactConfig, CompactResult, auto_compact},
        file_access::FileAccessTracker,
    },
    state::compact_history_event_log_path,
    turn::{
        events::{CompactAppliedStats, compact_applied_event},
        request::{PromptOutputRequest, build_prompt_output},
    },
};

/// reactive compact 恢复成功后的结果。
pub(crate) struct RecoveryResult {
    /// 压缩后的消息历史（含文件恢复消息）。
    pub messages: Vec<LlmMessage>,
    /// 压缩期间产生的事件。
    pub events: Vec<StorageEvent>,
}

/// reactive compact 调用上下文。
///
/// 将分散的参数聚合为结构体，避免函数签名过长。
pub(crate) struct ReactiveCompactContext<'a> {
    pub gateway: &'a KernelGateway,
    pub prompt_facts_provider: &'a dyn PromptFactsProvider,
    pub messages: &'a [LlmMessage],
    pub session_id: &'a str,
    pub working_dir: &'a str,
    pub turn_id: &'a str,
    pub step_index: usize,
    pub agent: &'a AgentEventContext,
    pub cancel: CancelToken,
    pub settings: &'a ContextWindowSettings,
    pub file_access_tracker: &'a FileAccessTracker,
}

fn recovery_result_from_compaction(
    turn_id: &str,
    agent: &AgentEventContext,
    settings: &ContextWindowSettings,
    file_access_tracker: &FileAccessTracker,
    compaction: CompactResult,
) -> RecoveryResult {
    let events = vec![compact_applied_event(
        Some(turn_id),
        agent,
        CompactTrigger::Auto,
        compaction.summary,
        CompactAppliedStats {
            meta: compaction.meta,
            preserved_recent_turns: compaction.preserved_recent_turns,
            pre_tokens: compaction.pre_tokens,
            post_tokens_estimate: compaction.post_tokens_estimate,
            messages_removed: compaction.messages_removed,
            tokens_freed: compaction.tokens_freed,
        },
        compaction.timestamp,
    )];

    let mut messages = compaction.messages;
    messages.extend(file_access_tracker.build_recovery_messages(settings.file_recovery_config()));

    RecoveryResult { messages, events }
}

/// 尝试通过 reactive compact 从 prompt-too-long 错误恢复。
///
/// 返回 `Some(RecoveryResult)` 表示恢复成功，调用方应替换消息历史并 continue。
/// 返回 `None` 表示无可压缩内容，无法恢复。
pub async fn try_reactive_compact(
    ctx: &ReactiveCompactContext<'_>,
) -> Result<Option<RecoveryResult>> {
    let prompt_output = build_prompt_output(PromptOutputRequest {
        gateway: ctx.gateway,
        prompt_facts_provider: ctx.prompt_facts_provider,
        session_id: ctx.session_id,
        turn_id: ctx.turn_id,
        working_dir: ctx.working_dir.as_ref(),
        step_index: ctx.step_index,
        messages: ctx.messages,
        session_state: None,
        current_agent_id: ctx.agent.agent_id.as_ref().map(|id| id.as_str()),
        submission_prompt_declarations: &[],
        prompt_governance: None,
    })
    .await?;

    match auto_compact(
        ctx.gateway,
        ctx.messages,
        Some(&prompt_output.system_prompt),
        CompactConfig {
            keep_recent_turns: ctx.settings.compact_keep_recent_turns,
            trigger: CompactTrigger::Auto,
            summary_reserve_tokens: ctx.settings.summary_reserve_tokens,
            max_retry_attempts: ctx.settings.compact_max_retry_attempts,
            history_path: Some(compact_history_event_log_path(
                ctx.session_id,
                std::path::Path::new(ctx.working_dir),
            )?),
            custom_instructions: None,
        },
        ctx.cancel.clone(),
    )
    .await?
    {
        Some(compaction) => Ok(Some(recovery_result_from_compaction(
            ctx.turn_id,
            ctx.agent,
            ctx.settings,
            ctx.file_access_tracker,
            compaction,
        ))),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, time::Duration};

    use astrcode_core::{
        AgentEventContext, CompactAppliedMeta, CompactMode, CompactTrigger, LlmMessage,
        StorageEventPayload, ToolCallRequest, UserMessageOrigin,
    };
    use chrono::{TimeZone, Utc};
    use serde_json::json;
    use tempfile::tempdir;

    use super::{CompactResult, recovery_result_from_compaction};
    use crate::context_window::{ContextWindowSettings, file_access::FileAccessTracker};

    fn test_settings() -> ContextWindowSettings {
        ContextWindowSettings {
            auto_compact_enabled: true,
            compact_threshold_percent: 80,
            reserved_context_size: 20_000,
            summary_reserve_tokens: 20_000,
            compact_max_retry_attempts: 3,
            tool_result_max_bytes: 16_384,
            compact_keep_recent_turns: 1,
            max_tracked_files: 8,
            max_recovered_files: 2,
            recovery_token_budget: 512,
            aggregate_result_bytes_budget: 16_384,
            micro_compact_gap_threshold: Duration::from_secs(30),
            micro_compact_keep_recent_results: 2,
        }
    }

    #[test]
    fn recovery_result_from_compaction_emits_event_and_appends_file_recovery_messages() {
        let tempdir = tempdir().expect("tempdir should exist");
        let working_dir = tempdir.path();
        let file_path = working_dir.join("src").join("lib.rs");
        fs::create_dir_all(file_path.parent().expect("parent dir should exist"))
            .expect("parent dir should write");
        fs::write(&file_path, "pub fn recovered() {}\n").expect("file should write");

        let file_access_messages = vec![
            LlmMessage::Assistant {
                content: String::new(),
                tool_calls: vec![ToolCallRequest {
                    id: "call-read-1".to_string(),
                    name: "readFile".to_string(),
                    args: json!({ "path": "src/lib.rs" }),
                }],
                reasoning: None,
            },
            LlmMessage::Tool {
                tool_call_id: "call-read-1".to_string(),
                content: "pub fn recovered() {}".to_string(),
            },
        ];
        let tracker = FileAccessTracker::seed_from_messages(&file_access_messages, 8, working_dir);
        let timestamp = Utc
            .with_ymd_and_hms(2026, 4, 14, 8, 0, 0)
            .single()
            .expect("timestamp should build");
        let compacted_message = LlmMessage::User {
            content: "compressed history".to_string(),
            origin: UserMessageOrigin::CompactSummary,
        };
        let result = recovery_result_from_compaction(
            "turn-compact-1",
            &AgentEventContext::default(),
            &test_settings(),
            &tracker,
            CompactResult {
                messages: vec![compacted_message.clone()],
                summary: "older context summary".to_string(),
                meta: CompactAppliedMeta {
                    mode: CompactMode::RetrySalvage,
                    instructions_present: false,
                    fallback_used: true,
                    retry_count: 2,
                    input_units: 5,
                    output_summary_chars: 21,
                },
                preserved_recent_turns: 2,
                pre_tokens: 1_500,
                post_tokens_estimate: 400,
                messages_removed: 6,
                tokens_freed: 1_100,
                timestamp,
            },
        );

        assert_eq!(result.events.len(), 1);
        assert_eq!(result.events[0].turn_id.as_deref(), Some("turn-compact-1"));
        assert!(matches!(
            &result.events[0].payload,
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
            } if *trigger == CompactTrigger::Auto
                && summary == "older context summary"
                && meta.mode == CompactMode::RetrySalvage
                && !meta.instructions_present
                && meta.fallback_used
                && meta.retry_count == 2
                && meta.input_units == 5
                && meta.output_summary_chars == 21
                && *preserved_recent_turns == 2
                && *pre_tokens == 1_500
                && *post_tokens_estimate == 400
                && *messages_removed == 6
                && *tokens_freed == 1_100
                && *event_timestamp == timestamp
        ));

        assert_eq!(result.messages.len(), 2);
        assert_eq!(result.messages[0], compacted_message);
        assert!(matches!(
            &result.messages[1],
            LlmMessage::User { content, origin }
                if *origin == UserMessageOrigin::ReactivationPrompt
                    && content.contains("Recovered file context after compaction.")
                    && content.contains("lib.rs")
                    && content.contains("recovered")
        ));
    }
}
