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

use astrcode_core::{
    AgentEventContext, AgentState, CancelToken, ContextStrategy, ExecutionOwner, Result,
    StorageEvent,
};
use astrcode_runtime_prompt::{DiagnosticLevel, PromptDiagnostics};

use super::{
    AgentLoop, TurnOutcome, finish_interrupted, finish_turn, finish_with_error, internal_error,
    llm_cycle, tool_cycle,
};
use crate::{
    compaction_runtime::{
        CompactionArtifact, CompactionReason, CompactionTailSnapshot, MAX_RECOVERED_FILES,
    },
    context_pipeline::{
        CompactionView, ContextBlock, ContextBundleInput, ConversationView, RecoveryRef,
    },
    context_window::{
        TokenUsageTracker, file_access::FileAccessTracker, is_prompt_too_long,
        merge_compact_prompt_context,
    },
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
pub(crate) struct TurnRunContext<'a, F>
where
    F: FnMut(StorageEvent) -> Result<()>,
{
    pub(crate) agent_loop: &'a AgentLoop,
    pub(crate) state: &'a AgentState,
    pub(crate) turn_id: &'a str,
    pub(crate) on_event: &'a mut F,
    pub(crate) cancel: CancelToken,
    pub(crate) emit_turn_done: bool,
    pub(crate) agent: AgentEventContext,
    pub(crate) execution_owner: ExecutionOwner,
    pub(crate) compaction_tail: CompactionTailSnapshot,
}

pub(crate) async fn run_turn<F>(ctx: TurnRunContext<'_, F>) -> Result<TurnOutcome>
where
    F: FnMut(StorageEvent) -> Result<()>,
{
    let TurnRunContext {
        agent_loop,
        state,
        turn_id,
        on_event,
        cancel,
        emit_turn_done,
        agent,
        execution_owner,
        compaction_tail,
    } = ctx;
    let provider =
        llm_cycle::build_provider(agent_loop.factory.clone(), Some(state.working_dir.clone()))
            .await;
    let provider = match provider {
        Ok(provider) => provider,
        Err(error) => {
            return report_error(
                turn_id,
                internal_error(error).to_string(),
                &agent,
                on_event,
                emit_turn_done,
            );
        },
    };
    let mut conversation = ConversationView::new(state.messages.clone());
    let mut recovered_memory = Vec::<ContextBlock>::new();
    let mut recovery_refs = Vec::<RecoveryRef>::new();
    let mut step_index = 0usize;
    let mut output_continuation_count = 0u8;
    let mut reactive_compact_attempts = 0usize;
    let model_limits = provider.model_limits();
    let mut token_tracker = TokenUsageTracker::default();
    let mut file_access = FileAccessTracker::from_stored_events(&compaction_tail.materialize());

    'step: loop {
        // 取消是协作式的，在 step 边界检查。若 LLM 正在执行慢速推理，
        // 此处不会立即响应取消——实际的中断由 generate_response 内部的
        // CancelToken 机制处理（取消时 provider 的 HTTP 连接会被 abort）。
        if cancel.is_cancelled() {
            return report_interrupted(turn_id, &agent, on_event, emit_turn_done);
        }

        let prior_compaction_view = CompactionView {
            messages: conversation.messages.clone(),
            memory_blocks: recovered_memory.clone(),
            recovery_refs: recovery_refs.clone(),
        };
        let bundle = match agent_loop.context.build_bundle(
            state,
            ContextBundleInput {
                turn_id,
                step_index,
                prior_compaction_view: Some(&prior_compaction_view),
                capability_descriptors: agent_loop.prompt.capability_descriptors(),
                keep_recent_turns: agent_loop.compact_keep_recent_turns(),
                model_context_window: model_limits.context_window,
            },
        ) {
            Ok(bundle) => bundle,
            Err(error) => {
                return report_error(turn_id, error.to_string(), &agent, on_event, emit_turn_done);
            },
        };

        let build_output = match agent_loop
            .prompt
            .build_plan(state, &bundle.conversation, step_index)
            .await
        {
            Ok(output) => output,
            Err(error) => {
                return report_error(turn_id, error.to_string(), &agent, on_event, emit_turn_done);
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
                context: &bundle,
                tools: agent_loop.capabilities.tool_definitions(),
                model_context_window: model_limits.context_window,
                compact_threshold_percent: agent_loop.compact_threshold_percent(),
            },
            &token_tracker,
        ) {
            Ok(prepared) => prepared,
            Err(error) => {
                return report_error(turn_id, error.to_string(), &agent, on_event, emit_turn_done);
            },
        };
        emit_event_with_file_tracking(
            &mut file_access,
            on_event,
            StorageEvent::PromptMetrics {
                turn_id: Some(turn_id.to_string()),
                agent: agent.clone(),
                step_index: step_index as u32,
                estimated_tokens: prompt_snapshot.context_tokens.min(u32::MAX as usize) as u32,
                context_window: prompt_snapshot.context_window.min(u32::MAX as usize) as u32,
                effective_window: prompt_snapshot.effective_window.min(u32::MAX as usize) as u32,
                threshold_tokens: prompt_snapshot.threshold_tokens.min(u32::MAX as usize) as u32,
                truncated_tool_results: truncated_tool_results.min(u32::MAX as usize) as u32,
            },
        )?;
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
                return report_error(turn_id, error.to_string(), &agent, on_event, emit_turn_done);
            },
        };
        if matches!(context_strategy, ContextStrategy::Compact) {
            let runtime_system_prompt_for_auto_compact = request.system_prompt.clone();
            let tools = agent_loop.capabilities.tool_definitions();
            match maybe_compact_conversation(
                agent_loop,
                CompactContext {
                    state,
                    provider: &provider,
                    conversation: &conversation,
                    runtime_system_prompt: runtime_system_prompt_for_auto_compact.as_deref(),
                    reason: CompactionReason::Auto,
                    turn_id,
                    agent: &agent,
                    cancel: cancel.clone(),
                    tail: compaction_tail.clone(),
                    tools,
                    file_access: &file_access,
                },
                on_event,
            )
            .await
            {
                Ok(Some(compacted_view)) => {
                    recovered_memory = compacted_view.memory_blocks;
                    recovery_refs = compacted_view.recovery_refs;
                    conversation = ConversationView::new(compacted_view.messages);
                    continue;
                },
                Ok(None) => {},
                Err(error) => {
                    return if cancel.is_cancelled() {
                        report_interrupted(turn_id, &agent, on_event, emit_turn_done)
                    } else {
                        report_error(turn_id, error.to_string(), &agent, on_event, emit_turn_done)
                    };
                },
            }
        }
        let policy_ctx = agent_loop.policy_context(state, turn_id, step_index);
        let request = match agent_loop
            .policy
            .check_model_request(request, &policy_ctx)
            .await
        {
            Ok(request) => request,
            Err(error) => {
                return report_error(turn_id, error.to_string(), &agent, on_event, emit_turn_done);
            },
        };
        // Reactive compact must reuse the exact system prompt that reached the provider after
        // policy rewrites. Otherwise the recovery compaction can summarize against stale
        // instructions and drift from the request shape that actually overflowed.
        let runtime_system_prompt_for_reactive_compact = request.system_prompt.clone();

        // 413 prompt too long → turn 级别 reactive compact
        // 当 LLM 调用返回 prompt too long 错误时，自动触发压缩并重试。
        // 与 compaction.rs 中的 compact 阶段重试不同，这是在 turn 执行层面的恢复。
        let output = match llm_cycle::generate_response(
            &provider,
            request.clone(),
            turn_id,
            agent.clone(),
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
                            state,
                            provider: &provider,
                            conversation: &conversation,
                            runtime_system_prompt: runtime_system_prompt_for_reactive_compact
                                .as_deref(),
                            reason: CompactionReason::Reactive,
                            turn_id,
                            agent: &agent,
                            cancel: cancel.clone(),
                            tail: compaction_tail.clone(),
                            tools: agent_loop.capabilities.tool_definitions(),
                            file_access: &file_access,
                        },
                        on_event,
                    )
                    .await
                    {
                        Ok(Some(compacted_view)) => {
                            recovered_memory = compacted_view.memory_blocks;
                            recovery_refs = compacted_view.recovery_refs;
                            conversation = ConversationView::new(compacted_view.messages);
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
                                &agent,
                                on_event,
                                emit_turn_done,
                            );
                        },
                        Err(compact_error) => {
                            if cancel.is_cancelled() {
                                return report_interrupted(
                                    turn_id,
                                    &agent,
                                    on_event,
                                    emit_turn_done,
                                );
                            }
                            return report_error(
                                turn_id,
                                format!(
                                    "reactive compact failed after {} attempts: {}",
                                    reactive_compact_attempts, compact_error
                                ),
                                &agent,
                                on_event,
                                emit_turn_done,
                            );
                        },
                    }
                } else if cancel.is_cancelled() {
                    return report_interrupted(turn_id, &agent, on_event, emit_turn_done);
                } else {
                    return report_error(
                        turn_id,
                        error.to_string(),
                        &agent,
                        on_event,
                        emit_turn_done,
                    );
                }
            },
        };
        reactive_compact_attempts = 0;
        token_tracker.record_usage(output.usage);

        if !output.content.is_empty() || !output.tool_calls.is_empty() || output.reasoning.is_some()
        {
            emit_event_with_file_tracking(
                &mut file_access,
                on_event,
                StorageEvent::AssistantFinal {
                    turn_id: Some(turn_id.to_string()),
                    agent: agent.clone(),
                    content: output.content.clone(),
                    reasoning_content: output.reasoning.as_ref().map(|value| value.content.clone()),
                    reasoning_signature: output
                        .reasoning
                        .as_ref()
                        .and_then(|value| value.signature.clone()),
                    timestamp: Some(chrono::Utc::now()),
                },
            )?;
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
                return complete_turn(
                    turn_id,
                    TurnOutcome::Completed,
                    &agent,
                    on_event,
                    emit_turn_done,
                );
            }
        }

        if tool_calls.is_empty() {
            return complete_turn(
                turn_id,
                TurnOutcome::Completed,
                &agent,
                on_event,
                emit_turn_done,
            );
        }

        let tool_cycle_outcome = match tool_cycle::execute_tool_calls(
            tool_calls,
            tool_cycle::ToolCycleContext {
                agent_loop,
                capabilities: &agent_loop.capabilities,
                turn_id,
                state,
                step_index,
                agent: &agent,
                execution_owner: &execution_owner,
                messages: &mut conversation.messages,
                on_event: &mut |event| {
                    emit_event_with_file_tracking(&mut file_access, on_event, event)
                },
                cancel: &cancel,
            },
        )
        .await
        {
            Ok(outcome) => outcome,
            Err(error) => {
                return report_error(
                    turn_id,
                    internal_error(error).to_string(),
                    &agent,
                    on_event,
                    emit_turn_done,
                );
            },
        };

        if matches!(
            tool_cycle_outcome,
            tool_cycle::ToolCycleOutcome::Interrupted
        ) {
            return report_interrupted(turn_id, &agent, on_event, emit_turn_done);
        }

        step_index += 1;
    }
}

fn complete_turn(
    turn_id: &str,
    outcome: TurnOutcome,
    agent: &AgentEventContext,
    on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
    emit_turn_done: bool,
) -> Result<TurnOutcome> {
    if emit_turn_done {
        finish_turn(turn_id, outcome, agent.clone(), on_event)
    } else {
        Ok(outcome)
    }
}

fn report_error(
    turn_id: &str,
    message: impl Into<String>,
    agent: &AgentEventContext,
    on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
    emit_turn_done: bool,
) -> Result<TurnOutcome> {
    let message = message.into();
    if emit_turn_done {
        finish_with_error(turn_id, message, agent.clone(), on_event)
    } else {
        on_event(StorageEvent::Error {
            turn_id: Some(turn_id.to_string()),
            agent: agent.clone(),
            message: message.clone(),
            timestamp: Some(chrono::Utc::now()),
        })?;
        Ok(TurnOutcome::Error { message })
    }
}

fn report_interrupted(
    turn_id: &str,
    agent: &AgentEventContext,
    on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
    emit_turn_done: bool,
) -> Result<TurnOutcome> {
    if emit_turn_done {
        finish_interrupted(turn_id, agent.clone(), on_event)
    } else {
        on_event(StorageEvent::Error {
            turn_id: Some(turn_id.to_string()),
            agent: agent.clone(),
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
    state: &'a AgentState,
    provider: &'a std::sync::Arc<dyn astrcode_runtime_llm::LlmProvider>,
    conversation: &'a ConversationView,
    /// 当前正常对话请求的 system prompt，上下文压缩时只把它作为参考材料嵌入模板。
    runtime_system_prompt: Option<&'a str>,
    reason: CompactionReason,
    turn_id: &'a str,
    agent: &'a AgentEventContext,
    cancel: CancelToken,
    tail: CompactionTailSnapshot,
    /// 工具定义列表，用于 pre-compact hook 上下文。
    tools: Vec<astrcode_core::ToolDefinition>,
    /// 当前 turn 已知的最近文件访问，用于在 compact 后恢复关键代码上下文。
    file_access: &'a FileAccessTracker,
}

async fn maybe_compact_conversation(
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
    // Persist the compaction event only after we have proven the rebuilt view is usable. That
    // avoids emitting durable history that the in-memory loop cannot continue from.
    emit_compact_applied(ctx.turn_id, ctx.agent, &artifact, on_event)?;
    Ok(Some(compacted_view))
}

fn emit_compact_applied(
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
    on_event(StorageEvent::CompactApplied {
        turn_id: Some(turn_id.to_string()),
        agent: agent.clone(),
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

fn emit_event_with_file_tracking(
    file_access: &mut FileAccessTracker,
    on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
    event: StorageEvent,
) -> Result<()> {
    // 文件恢复依赖 tool result metadata，因此需要在事件离开 turn_runner 之前同步记账，
    // 这样同一个 turn 内的后续 compact 才能恢复刚刚读过/改过的文件内容。
    file_access.record_event(&event);
    on_event(event)
}
