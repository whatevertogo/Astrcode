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

use astrcode_core::{CancelToken, Result};
use std::collections::HashMap;

use crate::prompt::{append_unique_tools, DiagnosticLevel, PromptContext, PromptDiagnostics};
use astrcode_core::AgentState;
use astrcode_core::LlmMessage;
use astrcode_core::ModelRequest;
use astrcode_core::StorageEvent;

use crate::context_window::{
    apply_microcompact, auto_compact, build_prompt_snapshot, effective_context_window,
    should_compact, CompactConfig, TokenUsageTracker,
};

use super::{
    finish_interrupted, finish_turn, finish_with_error, internal_error, llm_cycle, tool_cycle,
    AgentLoop, TurnOutcome,
};

// ---------------------------------------------------------------------------
// Error recovery constants (P4)
// ---------------------------------------------------------------------------

/// 413 prompt too long 时 reactive compact 最大重试次数。
/// 每次重试会进一步压缩上下文，超过此次数则终止 turn。
const MAX_REACTIVE_COMPACT_ATTEMPTS: usize = 3;

/// max_tokens 截断时自动继续生成的最大次数。
/// 超过此次数后即使模型仍被截断也终止 turn，避免无限循环。
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
            )
        }
    };
    let mut messages = state.messages.clone();
    let mut step_index = 0usize;
    let model_limits = provider.model_limits();
    let mut token_tracker = TokenUsageTracker::default();

    loop {
        // 取消是协作式的，在 step 边界检查。若 LLM 正在执行慢速推理，
        // 此处不会立即响应取消——实际的中断由 generate_response 内部的
        // CancelToken 机制处理（取消时 provider 的 HTTP 连接会被 abort）。
        if cancel.is_cancelled() {
            return report_interrupted(turn_id, on_event, emit_turn_done);
        }

        let mut vars = HashMap::new();
        if let Some(latest_user_message) = latest_user_message(&messages) {
            vars.insert(
                "turn.user_message".to_string(),
                latest_user_message.to_string(),
            );
        }
        let ctx = PromptContext {
            working_dir: state.working_dir.to_string_lossy().into_owned(),
            tool_names: agent_loop.capabilities.tool_names().to_vec(),
            capability_descriptors: agent_loop.prompt_capability_descriptors.clone(),
            prompt_declarations: agent_loop.prompt_declarations.clone(),
            // 每个 step 都按当前 working dir 解析 skill，确保 project/user override
            // 与最新 runtime surface 一致，不会把过期 base skills 直接塞给 prompt。
            skills: agent_loop
                .skill_catalog
                .resolve_for_working_dir(&state.working_dir.to_string_lossy()),
            step_index,
            turn_index: state.turn_count,
            vars,
        };
        let build_output = match agent_loop.prompt_composer.build(&ctx).await {
            Ok(output) => output,
            Err(error) => {
                return report_error(turn_id, error.to_string(), on_event, emit_turn_done);
            }
        };
        log_prompt_diagnostics(&build_output.diagnostics);
        let plan = build_output.plan;
        let system_prompt = plan.render_system();
        // 消息顺序契约：prepend_messages 包含行为引导（如"编辑前先检查文件"），
        // append_messages 包含尾部指令或 few-shot 示例。用户历史消息夹在两者之间，
        // 确保行为引导始终在对话开头，尾部指令在最后（最靠近模型注意力焦点）。
        let mut request_messages = plan.prepend_messages.clone();
        request_messages.extend(messages.iter().cloned());
        request_messages.extend(plan.append_messages.clone());
        let microcompact_result = apply_microcompact(
            &request_messages,
            &agent_loop.prompt_capability_descriptors,
            agent_loop.tool_result_max_bytes(),
            agent_loop.compact_keep_recent_turns(),
            effective_context_window(model_limits),
        );
        request_messages = microcompact_result.messages;
        let prompt_snapshot = build_prompt_snapshot(
            &token_tracker,
            &request_messages,
            system_prompt.as_deref(),
            model_limits,
            agent_loop.compact_threshold_percent(),
        );
        on_event(StorageEvent::PromptMetrics {
            turn_id: Some(turn_id.to_string()),
            step_index: step_index as u32,
            estimated_tokens: prompt_snapshot.context_tokens.min(u32::MAX as usize) as u32,
            context_window: prompt_snapshot.context_window.min(u32::MAX as usize) as u32,
            effective_window: prompt_snapshot.effective_window.min(u32::MAX as usize) as u32,
            threshold_tokens: prompt_snapshot.threshold_tokens.min(u32::MAX as usize) as u32,
            truncated_tool_results: microcompact_result
                .truncated_tool_results
                .min(u32::MAX as usize) as u32,
        })?;
        if agent_loop.auto_compact_enabled() && should_compact(prompt_snapshot) {
            let compact_result = match auto_compact(
                provider.as_ref(),
                &messages,
                system_prompt.as_deref(),
                CompactConfig {
                    keep_recent_turns: agent_loop.compact_keep_recent_turns(),
                    trigger: astrcode_core::CompactTrigger::Auto,
                },
                cancel.clone(),
            )
            .await
            {
                Ok(result) => result,
                Err(error) => {
                    return if cancel.is_cancelled() {
                        report_interrupted(turn_id, on_event, emit_turn_done)
                    } else {
                        report_error(turn_id, error.to_string(), on_event, emit_turn_done)
                    };
                }
            };

            if let Some(compact_result) = compact_result {
                on_event(StorageEvent::CompactApplied {
                    turn_id: Some(turn_id.to_string()),
                    trigger: astrcode_core::CompactTrigger::Auto,
                    summary: compact_result.summary,
                    preserved_recent_turns: compact_result
                        .preserved_recent_turns
                        .min(u32::MAX as usize) as u32,
                    pre_tokens: compact_result.pre_tokens.min(u32::MAX as usize) as u32,
                    post_tokens_estimate: compact_result.post_tokens_estimate.min(u32::MAX as usize)
                        as u32,
                    messages_removed: compact_result.messages_removed.min(u32::MAX as usize) as u32,
                    tokens_freed: compact_result.tokens_freed.min(u32::MAX as usize) as u32,
                    timestamp: compact_result.timestamp,
                })?;
                messages = compact_result.messages;
                continue;
            }
        }
        let mut tool_definitions = agent_loop.capabilities.tool_definitions();
        append_unique_tools(&mut tool_definitions, plan.extra_tools);
        let policy_ctx = agent_loop.policy_context(state, turn_id, step_index);
        // 保留 system_prompt 和 plan 的克隆，用于 reactive compact 重试 (P4.1)
        let system_prompt_for_retry = system_prompt.clone();
        let prepend_messages = plan.prepend_messages.clone();
        let append_messages = plan.append_messages.clone();
        let request = ModelRequest {
            messages: request_messages,
            tools: tool_definitions,
            system_prompt,
        };
        let mut request = match agent_loop
            .policy
            .check_model_request(request, &policy_ctx)
            .await
        {
            Ok(request) => request,
            Err(error) => {
                return report_error(turn_id, error.to_string(), on_event, emit_turn_done);
            }
        };

        // 413 prompt too long → turn 级别 reactive compact
        // 当 LLM 调用返回 prompt too long 错误时，自动触发压缩并重试。
        // 与 compaction.rs 中的 compact 阶段重试不同，这是在 turn 执行层面的恢复。
        let mut reactive_compact_attempts = 0usize;
        let output = loop {
            match llm_cycle::generate_response(
                &provider,
                request.clone(),
                turn_id,
                cancel.clone(),
                on_event,
            )
            .await
            {
                Ok(output) => break output,
                Err(error) => {
                    // 使用结构化错误分类判断是否为 prompt too long (P4.3)
                    let is_too_long = crate::context_window::is_prompt_too_long(&error);
                    if is_too_long
                        && agent_loop.auto_compact_enabled()
                        && reactive_compact_attempts < MAX_REACTIVE_COMPACT_ATTEMPTS
                    {
                        reactive_compact_attempts += 1;
                        log::warn!(
                            "[turn {}] LLM returned prompt too long, attempting reactive compact ({}/{})",
                            turn_id,
                            reactive_compact_attempts,
                            MAX_REACTIVE_COMPACT_ATTEMPTS
                        );

                        // 触发 reactive compact：压缩当前消息历史
                        let compact_result = auto_compact(
                            provider.as_ref(),
                            &messages,
                            system_prompt_for_retry.as_deref(),
                            CompactConfig {
                                keep_recent_turns: agent_loop.compact_keep_recent_turns(),
                                trigger: astrcode_core::CompactTrigger::Auto,
                            },
                            cancel.clone(),
                        )
                        .await;

                        match compact_result {
                            Ok(Some(compact)) => {
                                on_event(StorageEvent::CompactApplied {
                                    turn_id: Some(turn_id.to_string()),
                                    trigger: astrcode_core::CompactTrigger::Auto,
                                    summary: compact.summary.clone(),
                                    preserved_recent_turns: compact
                                        .preserved_recent_turns
                                        .min(u32::MAX as usize)
                                        as u32,
                                    pre_tokens: compact.pre_tokens.min(u32::MAX as usize) as u32,
                                    post_tokens_estimate: compact
                                        .post_tokens_estimate
                                        .min(u32::MAX as usize)
                                        as u32,
                                    messages_removed: compact
                                        .messages_removed
                                        .min(u32::MAX as usize)
                                        as u32,
                                    tokens_freed: compact.tokens_freed.min(u32::MAX as usize)
                                        as u32,
                                    timestamp: compact.timestamp,
                                })?;
                                messages = compact.messages;
                                // 重新构建请求消息并继续循环重试
                                let mut retry_messages = prepend_messages.clone();
                                retry_messages.extend(messages.iter().cloned());
                                retry_messages.extend(append_messages.clone());
                                let microcompact_result = apply_microcompact(
                                    &retry_messages,
                                    &agent_loop.prompt_capability_descriptors,
                                    agent_loop.tool_result_max_bytes(),
                                    agent_loop.compact_keep_recent_turns(),
                                    effective_context_window(model_limits),
                                );
                                request.messages = microcompact_result.messages;
                                continue;
                            }
                            Ok(None) => {
                                // compact 返回 None 表示无可压缩内容，无法恢复
                                return report_error(
                                    turn_id,
                                    format!(
                                        "prompt too long but no compressible history available after {} attempts",
                                        reactive_compact_attempts
                                    ),
                                    on_event,
                                    emit_turn_done,
                                );
                            }
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
                            }
                        }
                    } else if cancel.is_cancelled() {
                        return report_interrupted(turn_id, on_event, emit_turn_done);
                    } else {
                        return report_error(turn_id, error.to_string(), on_event, emit_turn_done);
                    }
                }
            }
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
        messages.push(LlmMessage::Assistant {
            content: output.content,
            tool_calls: output.tool_calls,
            reasoning: output.reasoning,
        });

        // 检测 max_tokens 截断 → 注入 nudge 消息继续生成
        // 当模型因 max_tokens 限制截断输出时，自动注入一条继续提示，
        // 鼓励模型从截断处继续生成。最多重试 MAX_OUTPUT_CONTINUATION_ATTEMPTS 次。
        if output.finish_reason.is_max_tokens() {
            let continuation_count = 0u8; // 当前 turn 内的续命计数
            if continuation_count < MAX_OUTPUT_CONTINUATION_ATTEMPTS as u8 {
                log::warn!(
                    "[turn {}] output truncated by max_tokens, injecting continue nudge ({}/{})",
                    turn_id,
                    continuation_count + 1,
                    MAX_OUTPUT_CONTINUATION_ATTEMPTS
                );

                // 注入 nudge 消息，告诉模型继续生成
                messages.push(LlmMessage::User {
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
            &mut messages,
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
                )
            }
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

fn latest_user_message(messages: &[LlmMessage]) -> Option<&str> {
    messages.iter().rev().find_map(|message| match message {
        LlmMessage::User { content, .. } => Some(content.as_str()),
        LlmMessage::Assistant { .. } | LlmMessage::Tool { .. } => None,
    })
}

fn log_prompt_diagnostics(diagnostics: &PromptDiagnostics) {
    for diagnostic in &diagnostics.items {
        let block_id = diagnostic.block_id.as_deref().unwrap_or("-");
        let contributor_id = diagnostic.contributor_id.as_deref().unwrap_or("-");
        let message = format!(
            "prompt diagnostic contributor={contributor_id} block={block_id} reason={:?} suggestion={}",
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
