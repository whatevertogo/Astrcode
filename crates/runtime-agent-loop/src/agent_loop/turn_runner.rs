//! # Turn 执行器 (Turn Runner)
//!
//! 实现一个完整 Agent Turn 的执行流程，是 `AgentLoop` 的核心实现。
//!
//! ## Turn 内部的 Step 循环
//!
//! 一个 Turn 可能包含多个 Step（LLM 调用 → 工具执行 → 再调用 LLM → ...），
//! 直到 LLM 不再请求工具调用为止。每个 Step 的流程：
//!
//! ```text
//! 1. compose prompt  →  组装系统提示词 + 历史消息
//! 2. call LLM        →  发送到 Provider，流式接收 delta
//! 3. process result   →  如果有 tool_calls → 执行工具 → 回到步骤 1
//!                       如果没有 tool_calls → Turn 结束
//! ```
//!
//! ## 终止条件
//!
//! - LLM 返回纯文本（无工具调用）
//! - 取消信号触发
//! - 任何步骤返回错误
//! - Token 预算耗尽
//!
//! ## Token 预算
//!
//! 用户可以在消息中指定 Token 预算（如 `+50k`），Turn 会在接近预算时
//! 自动停止或请求继续（auto-continue nudge）。

use astrcode_core::{AgentState, CancelToken, ContextStrategy, Result, StorageEvent};
use astrcode_runtime_prompt::{DiagnosticLevel, PromptDiagnostics};

use super::{
    AgentLoop, TurnOutcome, finish_interrupted, finish_turn, finish_with_error, internal_error,
    llm_cycle, tool_cycle,
};
use crate::{
    compaction_runtime::{CompactionArtifact, CompactionReason, CompactionTailSnapshot},
    context_pipeline::{ContextBundleInput, ConversationView},
    context_window::{TokenUsageTracker, is_prompt_too_long},
    request_assembler::{PreparedRequest, StepRequestConfig},
};

// ---------------------------------------------------------------------------
// Error recovery constants (P4)
// ---------------------------------------------------------------------------

/// prompt too long 时 reactive compact 最大重试次数。
/// 每次重试会进一步压缩上下文，超过此次数则终止 turn。
const MAX_REACTIVE_COMPACT_ATTEMPTS: usize = 3;

/// max_tokens 截断时自动继续生成的最大次数。
/// 超过此次数后即使模型仍被截断也终止 turn，避免无限循环。
/// TODO: 更好的数字和可能的可配置化
const MAX_OUTPUT_CONTINUATION_ATTEMPTS: usize = 3;

/// 执行一个完整的 agent turn（从用户提示到最终响应）。
///
/// ## Turn 内部的 step 循环
///
/// 一个 turn 可能包含多个 step（LLM 调用 → 工具执行 → 再调用 LLM → ...），
/// 直到 LLM 不再请求工具调用为止。每个 step 的流程：
///
/// ```text
/// 1. compose prompt  →  组装系统提示词 + 历史消息
/// 2. call LLM        →  发送到 provider，流式接收 delta
/// 3. process result   →  如果有 tool_calls → 执行工具 → 回到步骤 1
///                       如果没有 tool_calls → turn 结束
/// ```
///
/// ## 终止条件
///
/// - LLM 返回纯文本（无工具调用）
/// - 取消信号触发
/// - 任何步骤返回错误
pub(crate) async fn run_turn(
    agent_loop: &AgentLoop,
    state: &AgentState,
    turn_id: &str,
    on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
    cancel: CancelToken,
    emit_turn_done: bool,
    compaction_tail: CompactionTailSnapshot,
) -> Result<TurnOutcome> {
    let provider =
        llm_cycle::build_provider(agent_loop.factory.clone(), Some(state.working_dir.clone()))
            .await;
    let provider = match provider {
        Ok(provider) => provider,
        Err(error) => {
            return report_error(
                turn_id,
                internal_error(error).to_string(),
                on_event,
                emit_turn_done,
            );
        },
    };
    let mut conversation = ConversationView::new(state.messages.clone());
    let mut step_index = 0usize;
    let mut output_continuation_count = 0u8;
    let model_limits = provider.model_limits();
    let mut token_tracker = TokenUsageTracker::default();

    'step: loop {
        // 取消是协作式的，在 step 边界检查。若 LLM 正在执行慢速推理，
        // 此处不会立即响应取消——实际的中断由 generate_response 内部的
        // CancelToken 机制处理（取消时 provider 的 HTTP 连接会被 abort）。
        if cancel.is_cancelled() {
            return report_interrupted(turn_id, on_event, emit_turn_done);
        }

        let bundle = match agent_loop.context.build_bundle(
            state,
            ContextBundleInput {
                turn_id,
                step_index,
                prior_compaction_view: Some(&conversation),
                capability_descriptors: agent_loop.prompt.capability_descriptors(),
                keep_recent_turns: agent_loop.compact_keep_recent_turns(),
                model_context_window: model_limits.context_window,
            },
        ) {
            Ok(bundle) => bundle,
            Err(error) => {
                return report_error(turn_id, error.to_string(), on_event, emit_turn_done);
            },
        };

        let build_output = match agent_loop
            .prompt
            .build_plan(state, &bundle.conversation, step_index)
            .await
        {
            Ok(output) => output,
            Err(error) => {
                return report_error(turn_id, error.to_string(), on_event, emit_turn_done);
            },
        };
        log_prompt_diagnostics(&build_output.diagnostics);
        let plan = build_output.plan;
        let PreparedRequest {
            request,
            prompt_snapshot,
            truncated_tool_results,
        } = match agent_loop.request_assembler.build_step_request(
            StepRequestConfig {
                prompt: &plan,
                context: bundle.clone(),
                tools: agent_loop.capabilities.tool_definitions(),
                model_context_window: model_limits.context_window,
                compact_threshold_percent: agent_loop.compact_threshold_percent(),
            },
            &token_tracker,
        ) {
            Ok(prepared) => prepared,
            Err(error) => {
                return report_error(turn_id, error.to_string(), on_event, emit_turn_done);
            },
        };
        on_event(StorageEvent::PromptMetrics {
            turn_id: Some(turn_id.to_string()),
            step_index: step_index as u32,
            estimated_tokens: prompt_snapshot.context_tokens.min(u32::MAX as usize) as u32,
            context_window: prompt_snapshot.context_window.min(u32::MAX as usize) as u32,
            effective_window: prompt_snapshot.effective_window.min(u32::MAX as usize) as u32,
            threshold_tokens: prompt_snapshot.threshold_tokens.min(u32::MAX as usize) as u32,
            truncated_tool_results: truncated_tool_results.min(u32::MAX as usize) as u32,
        })?;
        let decision_input = agent_loop
            .compaction
            .build_context_decision(&prompt_snapshot, truncated_tool_results);
        let policy_ctx = agent_loop.policy_context(state, turn_id, step_index);
        let context_strategy = match agent_loop
            .policy
            .decide_context_strategy(&decision_input, &policy_ctx)
            .await
        {
            Ok(strategy) => strategy,
            Err(error) => {
                return report_error(turn_id, error.to_string(), on_event, emit_turn_done);
            },
        };
        if matches!(context_strategy, ContextStrategy::Compact) {
            let system_prompt_for_compact = request.system_prompt.clone();
            match maybe_compact_conversation(
                agent_loop,
                CompactContext {
                    provider: &provider,
                    conversation: &conversation,
                    base_system_prompt: system_prompt_for_compact.as_deref(),
                    reason: CompactionReason::Auto,
                    turn_id,
                    cancel: cancel.clone(),
                    tail: compaction_tail.clone(),
                },
                on_event,
            )
            .await
            {
                Ok(Some(compacted_view)) => {
                    conversation = compacted_view;
                    continue;
                },
                Ok(None) => {},
                Err(error) => {
                    return if cancel.is_cancelled() {
                        report_interrupted(turn_id, on_event, emit_turn_done)
                    } else {
                        report_error(turn_id, error.to_string(), on_event, emit_turn_done)
                    };
                },
            }
        }
        let policy_ctx = agent_loop.policy_context(state, turn_id, step_index);
        // 保留 system_prompt 和 plan 的克隆，用于 reactive compact 重试 (P4.1)
        let system_prompt_for_retry = request.system_prompt.clone();
        let request = match agent_loop
            .policy
            .check_model_request(request, &policy_ctx)
            .await
        {
            Ok(request) => request,
            Err(error) => {
                return report_error(turn_id, error.to_string(), on_event, emit_turn_done);
            },
        };

        // 413 prompt too long → turn 级别 reactive compact
        // 当 LLM 调用返回 prompt too long 错误时，自动触发压缩并重试。
        // 与 compaction.rs 中的 compact 阶段重试不同，这是在 turn 执行层面的恢复。
        let mut reactive_compact_attempts = 0usize;
        let output = match llm_cycle::generate_response(
            &provider,
            request.clone(),
            turn_id,
            cancel.clone(),
            on_event,
        )
        .await
        {
            Ok(output) => output,
            Err(error) => {
                // 使用结构化错误分类判断是否为 prompt too long (P4.3)
                let is_too_long = is_prompt_too_long(&error);
                if is_too_long
                    && agent_loop.auto_compact_enabled()
                    && reactive_compact_attempts < MAX_REACTIVE_COMPACT_ATTEMPTS
                {
                    reactive_compact_attempts += 1;
                    log::warn!(
                        "[turn {}] LLM returned prompt too long, attempting reactive compact \
                         ({}/{})",
                        turn_id,
                        reactive_compact_attempts,
                        MAX_REACTIVE_COMPACT_ATTEMPTS
                    );

                    match maybe_compact_conversation(
                        agent_loop,
                        CompactContext {
                            provider: &provider,
                            conversation: &conversation,
                            base_system_prompt: system_prompt_for_retry.as_deref(),
                            reason: CompactionReason::Reactive,
                            turn_id,
                            cancel: cancel.clone(),
                            tail: compaction_tail.clone(),
                        },
                        on_event,
                    )
                    .await
                    {
                        Ok(Some(compacted_view)) => {
                            conversation = compacted_view;
                            continue 'step;
                        },
                        Ok(None) => {
                            // compact 返回 None 表示无可压缩内容，无法恢复
                            return report_error(
                                turn_id,
                                format!(
                                    "prompt too long but no compressible history available after \
                                     {} attempts",
                                    reactive_compact_attempts
                                ),
                                on_event,
                                emit_turn_done,
                            );
                        },
                        Err(compact_error) => {
                            if cancel.is_cancelled() {
                                return report_interrupted(turn_id, on_event, emit_turn_done);
                            }
                            return report_error(
                                turn_id,
                                format!(
                                    "reactive compact failed after {} attempts: {}",
                                    reactive_compact_attempts, compact_error
                                ),
                                on_event,
                                emit_turn_done,
                            );
                        },
                    }
                } else if cancel.is_cancelled() {
                    return report_interrupted(turn_id, on_event, emit_turn_done);
                } else {
                    return report_error(turn_id, error.to_string(), on_event, emit_turn_done);
                }
            },
        };
        token_tracker.record_usage(output.usage);

        if !output.content.is_empty() || !output.tool_calls.is_empty() || output.reasoning.is_some()
        {
            on_event(StorageEvent::AssistantFinal {
                turn_id: Some(turn_id.to_string()),
                content: output.content.clone(),
                reasoning_content: output.reasoning.as_ref().map(|value| value.content.clone()),
                reasoning_signature: output
                    .reasoning
                    .as_ref()
                    .and_then(|value| value.signature.clone()),
                timestamp: Some(chrono::Utc::now()),
            })?;
        }

        let tool_calls = output.tool_calls.clone();
        conversation
            .messages
            .push(astrcode_core::LlmMessage::Assistant {
                content: output.content,
                tool_calls: output.tool_calls,
                reasoning: output.reasoning,
            });

        // 检测 max_tokens 截断 → 注入 nudge 消息继续生成
        // 当模型因 max_tokens 限制截断输出时，自动注入一条继续提示，
        // 鼓励模型从截断处继续生成。最多重试 MAX_OUTPUT_CONTINUATION_ATTEMPTS 次。
        if output.finish_reason.is_max_tokens() {
            if output_continuation_count < MAX_OUTPUT_CONTINUATION_ATTEMPTS as u8 {
                output_continuation_count += 1;
                log::warn!(
                    "[turn {}] output truncated by max_tokens, injecting continue nudge ({}/{})",
                    turn_id,
                    output_continuation_count,
                    MAX_OUTPUT_CONTINUATION_ATTEMPTS
                );

                // 注入 nudge 消息，告诉模型继续生成
                conversation.messages.push(astrcode_core::LlmMessage::User {
                    content: "Continue from where you left off. Do not repeat or summarize."
                        .to_string(),
                    origin: astrcode_core::UserMessageOrigin::AutoContinueNudge,
                });

                // 不终止 turn，继续下一轮 step 循环
                step_index += 1;
                continue;
            } else {
                log::warn!(
                    "[turn {}] max_tokens continuation limit reached ({}), ending turn",
                    turn_id,
                    MAX_OUTPUT_CONTINUATION_ATTEMPTS
                );
                // 超过最大续命次数，正常结束 turn
                return complete_turn(turn_id, TurnOutcome::Completed, on_event, emit_turn_done);
            }
        }

        if tool_calls.is_empty() {
            return complete_turn(turn_id, TurnOutcome::Completed, on_event, emit_turn_done);
        }

        let tool_cycle_outcome = match tool_cycle::execute_tool_calls(
            agent_loop,
            &agent_loop.capabilities,
            tool_calls,
            turn_id,
            state,
            step_index,
            &mut conversation.messages,
            on_event,
            &cancel,
        )
        .await
        {
            Ok(outcome) => outcome,
            Err(error) => {
                return report_error(
                    turn_id,
                    internal_error(error).to_string(),
                    on_event,
                    emit_turn_done,
                );
            },
        };

        if matches!(
            tool_cycle_outcome,
            tool_cycle::ToolCycleOutcome::Interrupted
        ) {
            return report_interrupted(turn_id, on_event, emit_turn_done);
        }

        step_index += 1;
    }
}

fn complete_turn(
    turn_id: &str,
    outcome: TurnOutcome,
    on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
    emit_turn_done: bool,
) -> Result<TurnOutcome> {
    if emit_turn_done {
        finish_turn(turn_id, outcome, on_event)
    } else {
        Ok(outcome)
    }
}

fn report_error(
    turn_id: &str,
    message: impl Into<String>,
    on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
    emit_turn_done: bool,
) -> Result<TurnOutcome> {
    let message = message.into();
    if emit_turn_done {
        finish_with_error(turn_id, message, on_event)
    } else {
        on_event(StorageEvent::Error {
            turn_id: Some(turn_id.to_string()),
            message: message.clone(),
            timestamp: Some(chrono::Utc::now()),
        })?;
        Ok(TurnOutcome::Error { message })
    }
}

fn report_interrupted(
    turn_id: &str,
    on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
    emit_turn_done: bool,
) -> Result<TurnOutcome> {
    if emit_turn_done {
        finish_interrupted(turn_id, on_event)
    } else {
        on_event(StorageEvent::Error {
            turn_id: Some(turn_id.to_string()),
            message: "interrupted".to_string(),
            timestamp: Some(chrono::Utc::now()),
        })?;
        Ok(TurnOutcome::Cancelled)
    }
}

fn log_prompt_diagnostics(diagnostics: &PromptDiagnostics) {
    for diagnostic in &diagnostics.items {
        let block_id = diagnostic.block_id.as_deref().unwrap_or("-");
        let contributor_id = diagnostic.contributor_id.as_deref().unwrap_or("-");
        let message = format!(
            "prompt diagnostic contributor={contributor_id} block={block_id} reason={:?} \
             suggestion={}",
            diagnostic.reason,
            diagnostic.suggestion.as_deref().unwrap_or("-")
        );

        match diagnostic.level {
            DiagnosticLevel::Info => log::debug!("{message}"),
            DiagnosticLevel::Warning => log::warn!("{message}"),
            DiagnosticLevel::Error => log::error!("{message}"),
        }
    }
}

/// Parameters needed for a single compact-and-rebuild cycle.
struct CompactContext<'a> {
    provider: &'a std::sync::Arc<dyn astrcode_runtime_llm::LlmProvider>,
    conversation: &'a ConversationView,
    base_system_prompt: Option<&'a str>,
    reason: CompactionReason,
    turn_id: &'a str,
    cancel: CancelToken,
    tail: CompactionTailSnapshot,
}

async fn maybe_compact_conversation(
    agent_loop: &AgentLoop,
    ctx: CompactContext<'_>,
    on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
) -> Result<Option<ConversationView>> {
    let compact_result = agent_loop
        .compaction
        .compact(
            ctx.provider.as_ref(),
            ctx.conversation,
            ctx.base_system_prompt,
            ctx.reason,
            ctx.cancel,
        )
        .await?;

    let Some(artifact) = compact_result else {
        return Ok(None);
    };
    emit_compact_applied(ctx.turn_id, &artifact, on_event)?;
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
    agent_loop
        .compaction
        .rebuild_conversation(&artifact, &tail)
        .map(Some)
}

fn emit_compact_applied(
    turn_id: &str,
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
    on_event(StorageEvent::CompactApplied {
        turn_id: Some(turn_id.to_string()),
        trigger: artifact.trigger.as_trigger(),
        summary: artifact.summary.clone(),
        preserved_recent_turns: artifact.preserved_recent_turns.min(u32::MAX as usize) as u32,
        pre_tokens: artifact.pre_tokens.min(u32::MAX as usize) as u32,
        post_tokens_estimate: artifact.post_tokens_estimate.min(u32::MAX as usize) as u32,
        messages_removed: artifact.messages_removed.min(u32::MAX as usize) as u32,
        tokens_freed: artifact.tokens_freed.min(u32::MAX as usize) as u32,
        timestamp: chrono::Utc::now(),
    })
}
