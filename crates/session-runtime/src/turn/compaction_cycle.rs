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
    StorageEvent, StorageEventPayload,
};
use astrcode_kernel::KernelGateway;

use crate::{
    context_window::{
        ContextWindowSettings,
        compaction::{CompactConfig, auto_compact},
        file_access::FileAccessTracker,
    },
    turn::request::build_prompt_output,
};

/// reactive compact 最大重试次数。
pub const MAX_REACTIVE_COMPACT_ATTEMPTS: usize = 3;

/// reactive compact 恢复成功后的结果。
pub struct RecoveryResult {
    /// 压缩后的消息历史（含文件恢复消息）。
    pub messages: Vec<LlmMessage>,
    /// 压缩期间产生的事件。
    pub events: Vec<StorageEvent>,
}

/// reactive compact 调用上下文。
///
/// 将分散的参数聚合为结构体，避免函数签名过长。
pub struct ReactiveCompactContext<'a> {
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

/// 尝试通过 reactive compact 从 prompt-too-long 错误恢复。
///
/// 返回 `Some(RecoveryResult)` 表示恢复成功，调用方应替换消息历史并 continue。
/// 返回 `None` 表示无可压缩内容，无法恢复。
pub async fn try_reactive_compact(
    ctx: &ReactiveCompactContext<'_>,
) -> Result<Option<RecoveryResult>> {
    let prompt_output = build_prompt_output(
        ctx.gateway,
        ctx.prompt_facts_provider,
        ctx.session_id,
        ctx.turn_id,
        ctx.working_dir.as_ref(),
        ctx.step_index,
        ctx.messages,
    )
    .await?;

    match auto_compact(
        ctx.gateway,
        ctx.messages,
        Some(&prompt_output.system_prompt),
        CompactConfig {
            keep_recent_turns: ctx.settings.compact_keep_recent_turns,
            trigger: CompactTrigger::Auto,
        },
        ctx.cancel.clone(),
    )
    .await?
    {
        Some(compaction) => {
            let events = vec![StorageEvent {
                turn_id: Some(ctx.turn_id.to_string()),
                agent: ctx.agent.clone(),
                payload: StorageEventPayload::CompactApplied {
                    trigger: CompactTrigger::Auto,
                    summary: compaction.summary,
                    preserved_recent_turns: compaction.preserved_recent_turns as u32,
                    pre_tokens: compaction.pre_tokens as u32,
                    post_tokens_estimate: compaction.post_tokens_estimate as u32,
                    messages_removed: compaction.messages_removed as u32,
                    tokens_freed: compaction.tokens_freed as u32,
                    timestamp: compaction.timestamp,
                },
            }];

            let mut messages = compaction.messages;
            messages.extend(
                ctx.file_access_tracker
                    .build_recovery_messages(ctx.settings.file_recovery_config()),
            );

            Ok(Some(RecoveryResult { messages, events }))
        },
        None => Ok(None),
    }
}
