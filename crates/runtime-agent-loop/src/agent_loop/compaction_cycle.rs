//! # Compaction 执行周期
//!
//! 封装单次上下文压缩（compact-and-rebuild）所需的参数和执行逻辑，
//! 以及 LLM prompt-too-long 错误的 reactive compact 恢复。

use std::sync::Arc;

use astrcode_core::{
    AgentEventContext, AgentState, AstrError, CancelToken, Result, StorageEvent,
    StorageEventPayload,
};

use super::{AgentLoop, TurnOutcome, finish_interrupted, finish_with_error};
use crate::{
    compaction_runtime::{
        CompactionArtifact, CompactionReason, CompactionTailSnapshot, MAX_RECOVERED_FILES,
    },
    context_pipeline::{CompactionView, ConversationView},
    context_window::{
        file_access::FileAccessTracker, is_prompt_too_long, merge_compact_prompt_context,
    },
};

/// 单次 compact-and-rebuild 周期所需的全部参数。
pub(crate) struct CompactContext<'a> {
    pub(crate) state: &'a AgentState,
    pub(crate) provider: &'a Arc<dyn astrcode_runtime_llm::LlmProvider>,
    pub(crate) conversation: &'a ConversationView,
    /// 当前正常对话请求的 system prompt，上下文压缩时只把它作为参考材料嵌入模板。
    pub(crate) runtime_system_prompt: Option<&'a str>,
    pub(crate) reason: CompactionReason,
    pub(crate) turn_id: &'a str,
    pub(crate) agent: &'a AgentEventContext,
    pub(crate) cancel: CancelToken,
    pub(crate) tail: CompactionTailSnapshot,
    /// 工具定义列表，用于 pre-compact hook 上下文。
    pub(crate) tools: Vec<astrcode_core::ToolDefinition>,
    /// 当前 turn 已知的最近文件访问，用于在 compact 后恢复关键代码上下文。
    pub(crate) file_access: &'a FileAccessTracker,
}

/// 尝试压缩对话历史，返回压缩后的视图；若无内容可压缩则返回 `None`。
pub(crate) async fn maybe_compact_conversation(
    agent_loop: &AgentLoop,
    ctx: CompactContext<'_>,
    on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
) -> Result<Option<CompactionView>> {
    let decision = agent_loop
        .hooks
        .run_pre_compact(agent_loop.compaction_hook_context_full(
            ctx.state,
            ctx.conversation,
            ctx.reason,
            agent_loop.compact_keep_recent_turns(),
            &ctx.tools,
            ctx.runtime_system_prompt,
        ))
        .await?;

    // 检查 hook 是否阻止压缩
    if !decision.allowed {
        return Err(astrcode_core::AstrError::Validation(
            decision
                .block_reason
                .unwrap_or_else(|| "compaction blocked by hook".to_string()),
        ));
    }

    // 如果 hook 提供了自定义摘要，跳过 LLM 调用直接使用
    let compact_result = if let Some(custom_summary) = &decision.custom_summary {
        log::info!(
            "using custom summary from hook ({} chars)",
            custom_summary.len()
        );
        // 使用自定义摘要构建 artifact
        let keep_turns = decision
            .override_keep_recent_turns
            .unwrap_or(agent_loop.compact_keep_recent_turns());
        crate::compaction_runtime::build_artifact_from_custom_summary(
            &ctx.conversation.messages,
            custom_summary,
            keep_turns,
            ctx.reason,
        )
    } else {
        // 正常的 LLM 压缩流程，应用 hook 修改
        let keep_turns = decision
            .override_keep_recent_turns
            .unwrap_or(agent_loop.compact_keep_recent_turns());
        let compact_prompt_context = merge_compact_prompt_context(
            ctx.runtime_system_prompt,
            decision.additional_system_prompt.as_deref(),
        );

        agent_loop
            .compaction
            .compact_with_keep_recent_turns(
                ctx.provider.as_ref(),
                ctx.conversation,
                compact_prompt_context.as_deref(),
                keep_turns,
                ctx.reason,
                ctx.cancel,
            )
            .await?
    };

    let Some(mut artifact) = compact_result else {
        return Ok(None);
    };
    artifact.recovered_files = ctx.file_access.recent_files(MAX_RECOVERED_FILES);
    let tail = {
        let materialized = ctx.tail.materialize();
        if materialized.is_empty() {
            CompactionTailSnapshot::from_messages(
                &ctx.conversation.messages,
                artifact.preserved_recent_turns,
            )
            .materialize()
        } else {
            materialized
        }
    };
    artifact.record_tail_seq(&tail);
    let compacted_view = agent_loop
        .compaction
        .rebuild_conversation(&artifact, &tail)?;
    agent_loop
        .hooks
        .run_post_compact_best_effort(astrcode_core::CompactionHookResultContext {
            compaction: agent_loop.compaction_hook_context(
                ctx.state,
                ctx.conversation,
                ctx.reason,
                artifact.preserved_recent_turns,
            ),
            summary: artifact.summary.clone(),
            strategy_id: artifact.strategy_id.clone(),
            preserved_recent_turns: artifact.preserved_recent_turns,
            pre_tokens: artifact.pre_tokens,
            post_tokens_estimate: artifact.post_tokens_estimate,
            messages_removed: artifact.messages_removed,
            tokens_freed: artifact.tokens_freed,
        })
        .await;
    // 在确认重建视图可用之后再持久化压缩事件，避免写入内存循环无法继续使用的持久历史。
    emit_compact_applied(ctx.turn_id, ctx.agent, &artifact, on_event)?;
    Ok(Some(compacted_view))
}

/// 发送 `CompactApplied` 事件，记录压缩结果。
pub(crate) fn emit_compact_applied(
    turn_id: &str,
    agent: &AgentEventContext,
    artifact: &CompactionArtifact,
    on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
) -> Result<()> {
    log::debug!(
        "compaction strategy={} source_range={}..{} tail_start={} seq={}",
        artifact.strategy_id,
        artifact.source_range.start,
        artifact.source_range.end,
        artifact.preserved_tail_start,
        artifact.compacted_at_seq
    );
    on_event(StorageEvent {
        turn_id: Some(turn_id.to_string()),
        agent: agent.clone(),
        payload: StorageEventPayload::CompactApplied {
            trigger: artifact.trigger.as_trigger(),
            summary: artifact.summary.clone(),
            preserved_recent_turns: artifact.preserved_recent_turns.min(u32::MAX as usize) as u32,
            pre_tokens: artifact.pre_tokens.min(u32::MAX as usize) as u32,
            post_tokens_estimate: artifact.post_tokens_estimate.min(u32::MAX as usize) as u32,
            messages_removed: artifact.messages_removed.min(u32::MAX as usize) as u32,
            tokens_freed: artifact.tokens_freed.min(u32::MAX as usize) as u32,
            timestamp: chrono::Utc::now(),
        },
    })
}

// ---------------------------------------------------------------------------
// Reactive compact 错误恢复
// ---------------------------------------------------------------------------

/// prompt too long 时 reactive compact 最大重试次数。
/// 每次重试会进一步压缩上下文，超过此次数则终止 turn。
pub(crate) const MAX_REACTIVE_COMPACT_ATTEMPTS: usize = 3;

/// LLM 调用失败后的 reactive compact 恢复结果。
pub(crate) enum ReactiveCompactOutcome {
    /// Reactive compact 成功恢复，返回压缩后的视图。
    /// 调用方应调用 `step_state.apply_compaction(view)` 后 `continue 'step`。
    Recovered(CompactionView),
    /// 无法恢复，已通过 `on_event` 发送终止事件。
    /// 调用方应直接 `return Ok(outcome)`。
    Terminal(TurnOutcome),
}

/// 处理 LLM 调用错误，尝试通过 reactive compact 恢复。
///
/// 当 LLM 返回 prompt-too-long 错误且 auto_compact 开启时，自动触发上下文压缩并重试。
/// 这与 `run_turn` 中基于 Policy 的 proactive compact 不同——这里是 LLM 实际拒绝后的被动恢复。
/// TODO: NOW ALLOW CLIPPY TOO MANY ARGUMENTS, 需要重构 CompactContext 来简化参数传递。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_llm_error_with_reactive_compact(
    llm_error: AstrError,
    agent_loop: &AgentLoop,
    state: &AgentState,
    provider: &Arc<dyn astrcode_runtime_llm::LlmProvider>,
    conversation: &ConversationView,
    file_access: &FileAccessTracker,
    runtime_system_prompt: Option<&str>,
    reactive_compact_attempts: &mut usize,
    turn_id: &str,
    agent: &AgentEventContext,
    cancel: &CancelToken,
    compaction_tail: &CompactionTailSnapshot,
    emit_turn_done: bool,
    on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
) -> Result<ReactiveCompactOutcome> {
    let is_too_long = is_prompt_too_long(&llm_error);

    if is_too_long
        && agent_loop.auto_compact_enabled()
        && *reactive_compact_attempts < MAX_REACTIVE_COMPACT_ATTEMPTS
    {
        *reactive_compact_attempts += 1;
        log::warn!(
            "[turn {}] LLM returned prompt too long, attempting reactive compact ({}/{})",
            turn_id,
            *reactive_compact_attempts,
            MAX_REACTIVE_COMPACT_ATTEMPTS
        );

        let outcome = maybe_compact_conversation(
            agent_loop,
            CompactContext {
                state,
                provider,
                conversation,
                runtime_system_prompt,
                reason: CompactionReason::Reactive,
                turn_id,
                agent,
                cancel: cancel.clone(),
                tail: compaction_tail.clone(),
                tools: agent_loop.capabilities.tool_definitions(),
                file_access,
            },
            on_event,
        )
        .await;

        match outcome {
            Ok(Some(compacted_view)) => Ok(ReactiveCompactOutcome::Recovered(compacted_view)),
            Ok(None) => {
                // compact 返回 None 表示无可压缩内容，无法恢复
                let msg = format!(
                    "prompt too long but no compressible history available after {} attempts",
                    *reactive_compact_attempts
                );
                let turn_outcome = report_error(turn_id, &msg, agent, on_event, emit_turn_done)?;
                Ok(ReactiveCompactOutcome::Terminal(turn_outcome))
            },
            Err(compact_error) => {
                if compact_error.is_cancelled() {
                    let turn_outcome =
                        report_interrupted(turn_id, agent, on_event, emit_turn_done)?;
                    return Ok(ReactiveCompactOutcome::Terminal(turn_outcome));
                }
                let msg = format!(
                    "reactive compact failed after {} attempts: {}",
                    *reactive_compact_attempts, compact_error
                );
                let turn_outcome = report_error(turn_id, &msg, agent, on_event, emit_turn_done)?;
                Ok(ReactiveCompactOutcome::Terminal(turn_outcome))
            },
        }
    } else if cancel.is_cancelled() {
        let turn_outcome = report_interrupted(turn_id, agent, on_event, emit_turn_done)?;
        Ok(ReactiveCompactOutcome::Terminal(turn_outcome))
    } else {
        let turn_outcome = report_error(
            turn_id,
            &llm_error.to_string(),
            agent,
            on_event,
            emit_turn_done,
        )?;
        Ok(ReactiveCompactOutcome::Terminal(turn_outcome))
    }
}

/// 发送错误事件并返回 TurnOutcome。
fn report_error(
    turn_id: &str,
    message: &str,
    agent: &AgentEventContext,
    on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
    emit_turn_done: bool,
) -> Result<TurnOutcome> {
    if emit_turn_done {
        finish_with_error(turn_id, message, agent.clone(), on_event)
    } else {
        on_event(StorageEvent {
            turn_id: Some(turn_id.to_string()),
            agent: agent.clone(),
            payload: StorageEventPayload::Error {
                message: message.to_string(),
                timestamp: Some(chrono::Utc::now()),
            },
        })?;
        Ok(TurnOutcome::Error {
            message: message.to_string(),
        })
    }
}

/// 发送中断事件并返回 TurnOutcome。
fn report_interrupted(
    turn_id: &str,
    agent: &AgentEventContext,
    on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
    emit_turn_done: bool,
) -> Result<TurnOutcome> {
    if emit_turn_done {
        finish_interrupted(turn_id, agent.clone(), on_event)
    } else {
        on_event(StorageEvent {
            turn_id: Some(turn_id.to_string()),
            agent: agent.clone(),
            payload: StorageEventPayload::Error {
                message: "interrupted".to_string(),
                timestamp: Some(chrono::Utc::now()),
            },
        })?;
        Ok(TurnOutcome::Cancelled)
    }
}
