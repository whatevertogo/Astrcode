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
    AgentEventContext, CancelToken, CompactTrigger, LlmMessage, PromptBuildRequest, PromptFacts,
    PromptFactsProvider, PromptFactsRequest, Result, StorageEvent, StorageEventPayload,
    UserMessageOrigin,
};
use astrcode_kernel::KernelGateway;

use crate::context_window::{
    compaction::{CompactConfig, auto_compact},
    file_access::FileAccessTracker,
    request_assembler::ContextWindowSettings,
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
    // 获取 system prompt 供压缩模板使用
    let prompt_facts = ctx
        .prompt_facts_provider
        .resolve_prompt_facts(&PromptFactsRequest {
            session_id: Some(ctx.session_id.to_string().into()),
            turn_id: Some(ctx.turn_id.to_string().into()),
            working_dir: ctx.working_dir.to_string().into(),
        })
        .await?;
    let metadata = prompt_metadata(ctx, &prompt_facts);
    let turn_index = count_user_turns(ctx.messages);
    let PromptFacts {
        profile,
        profile_context,
        metadata: _,
        skills,
        agent_profiles,
        prompt_declarations,
    } = prompt_facts;
    let prompt_output = ctx
        .gateway
        .build_prompt(PromptBuildRequest {
            session_id: Some(ctx.session_id.to_string().into()),
            turn_id: Some(ctx.turn_id.to_string().into()),
            working_dir: ctx.working_dir.to_string().into(),
            profile,
            step_index: ctx.step_index,
            turn_index,
            profile_context,
            capabilities: ctx.gateway.capabilities().capability_specs(),
            skills,
            agent_profiles,
            prompt_declarations,
            metadata,
        })
        .await
        .map_err(|e| astrcode_core::AstrError::Internal(e.to_string()))?;

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

fn prompt_metadata(ctx: &ReactiveCompactContext<'_>, facts: &PromptFacts) -> serde_json::Value {
    let mut metadata = match &facts.metadata {
        serde_json::Value::Object(map) => map.clone(),
        _ => serde_json::Map::new(),
    };
    metadata.insert(
        "sessionId".to_string(),
        serde_json::Value::String(ctx.session_id.to_string()),
    );
    metadata.insert(
        "turnId".to_string(),
        serde_json::Value::String(ctx.turn_id.to_string()),
    );
    if let Some(content) = ctx.messages.iter().rev().find_map(|message| match message {
        LlmMessage::User {
            content,
            origin: UserMessageOrigin::User,
        } => Some(content.clone()),
        _ => None,
    }) {
        metadata.insert(
            "latestUserMessage".to_string(),
            serde_json::Value::String(content),
        );
    }
    serde_json::Value::Object(metadata)
}

fn count_user_turns(messages: &[LlmMessage]) -> usize {
    messages
        .iter()
        .filter(|message| {
            matches!(
                message,
                LlmMessage::User {
                    origin: UserMessageOrigin::User,
                    ..
                }
            )
        })
        .count()
}
