//! Turn 执行器。
//!
//! 实现一个完整的 Agent Turn：LLM 调用 → 工具执行 → 循环直到完成。
//! 所有 provider 调用通过 `kernel` gateway 进行，不直接持有 provider。
//!
//! ## 架构：纯编排器
//!
//! `run_turn` 只负责 step 循环的编排，所有细节委托给子模块：
//! - `request_assembler` — 上下文优化管线（微压缩 → 裁剪 → 自动压缩）
//! - `llm_cycle` — LLM 流式调用
//! - `compaction_cycle` — reactive compact 错误恢复
//! - `tool_cycle` — 工具并发执行
//!
//! ## Turn 内部的 Step 循环
//!
//! 一个 Turn 可能包含多个 Step（LLM → 工具 → LLM → ...），直到 LLM 不再请求工具调用。
//!
//! ## 终止条件
//!
//! - LLM 返回纯文本（无工具调用）
//! - Token 预算耗尽或收益递减
//! - 取消信号触发
//! - 不可恢复错误
//! - Step 上限

use std::{collections::HashSet, path::Path, sync::Arc, time::Instant};

use astrcode_core::{
    AgentEventContext, CancelToken, LlmMessage, PromptFactsProvider, Result, StorageEvent,
    StorageEventPayload, UserMessageOrigin, config::RuntimeConfig,
};
use astrcode_kernel::Kernel;
use chrono::Utc;

use super::{
    TurnOutcome,
    compaction_cycle::{self, ReactiveCompactContext},
    llm_cycle,
    summary::{TurnFinishReason, TurnSummary},
    token_budget::{TokenBudgetDecision, build_auto_continue_nudge, check_token_budget},
    tool_cycle::{self, ToolCycleContext, ToolCycleOutcome},
};
use crate::context_window::{
    file_access::FileAccessTracker,
    micro_compact::MicroCompactState,
    request_assembler::{AssemblePromptRequest, ContextWindowSettings, assemble_prompt_request},
    token_usage::TokenUsageTracker,
};

/// 单个 Turn 的最大 step 数，防止无限循环。
const MAX_STEPS: usize = 50;

/// 可清除的工具名称（这些工具的旧结果可以被 prune pass 替换为占位文本）。
const CLEARABLE_TOOLS: &[&str] = &["readFile", "listDir", "grep", "findFiles"];

/// Turn 执行请求。
pub struct TurnRunRequest {
    pub session_id: String,
    pub working_dir: String,
    pub turn_id: String,
    pub messages: Vec<LlmMessage>,
    pub runtime: RuntimeConfig,
    pub cancel: CancelToken,
    pub agent: AgentEventContext,
    pub prompt_facts_provider: Arc<dyn PromptFactsProvider>,
}

/// Turn 执行结果。
pub struct TurnRunResult {
    pub outcome: TurnOutcome,
    /// Turn 结束时的完整消息历史（含本次 turn 新增的）。
    pub messages: Vec<LlmMessage>,
    /// Turn 执行期间产生的 storage events（用于持久化）。
    pub events: Vec<StorageEvent>,
    /// Turn 级稳定汇总（包含耗时、token、续写等指标）。
    pub summary: TurnSummary,
}

/// 执行一个完整的 Agent Turn。
///
/// 通过 `kernel` gateway 调用 LLM 和工具，不直接持有 provider。
/// 每个重要步骤通过事件回调发出。
pub async fn run_turn(kernel: Arc<Kernel>, request: TurnRunRequest) -> Result<TurnRunResult> {
    let turn_started_at = Instant::now();
    let mut messages = request.messages;
    let mut events = Vec::new();
    let mut token_tracker = TokenUsageTracker::default();
    let settings = ContextWindowSettings::from(&request.runtime);
    let mut total_cache_read_tokens: u64 = 0;
    let mut total_cache_creation_tokens: u64 = 0;

    // 解析 token 预算配置。budget > 0 时启用自动续写。
    let token_budget = request.runtime.default_token_budget.filter(|&b| b > 0);
    let max_continuations = request.runtime.max_continuations.unwrap_or(3).max(1);
    let continuation_min_delta_tokens = request
        .runtime
        .continuation_min_delta_tokens
        .unwrap_or(500)
        .max(1);

    let gateway = kernel.gateway();

    // 获取可用工具定义
    let tools = gateway.capabilities().tool_definitions();

    // 构建可清除工具名称集合
    let clearable_tools: HashSet<String> = CLEARABLE_TOOLS.iter().map(|s| s.to_string()).collect();

    let mut micro_compact_state = MicroCompactState::seed_from_messages(
        &messages,
        settings.micro_compact_config(),
        Instant::now(),
    );
    let mut file_access_tracker = FileAccessTracker::seed_from_messages(
        &messages,
        settings.max_tracked_files,
        Path::new(&request.working_dir),
    );

    let mut step_index: usize = 0;
    let mut reactive_compact_attempts: usize = 0;
    let mut continuation_count: u8 = 0;
    let mut last_delta_tokens: usize = 0;

    /// 构建当前 TurnSummary 的辅助宏，避免闭包借用冲突。
    macro_rules! make_summary {
        ($reason:expr) => {
            TurnSummary {
                finish_reason: $reason,
                wall_duration: turn_started_at.elapsed(),
                step_count: step_index + 1,
                continuation_count,
                total_tokens_used: token_tracker.budget_tokens(0) as u64,
                cache_read_input_tokens: total_cache_read_tokens,
                cache_creation_input_tokens: total_cache_creation_tokens,
                auto_compaction_count: 0,
                reactive_compact_count: reactive_compact_attempts,
            }
        };
    }

    loop {
        // —— 取消检查 ——
        if request.cancel.is_cancelled() {
            return Ok(TurnRunResult {
                outcome: TurnOutcome::Cancelled,
                messages,
                events,
                summary: make_summary!(TurnFinishReason::Cancelled),
            });
        }

        // —— Step 上限检查 ——
        if step_index >= MAX_STEPS {
            return Ok(TurnRunResult {
                outcome: TurnOutcome::Error {
                    message: format!("turn exceeded maximum steps ({MAX_STEPS})"),
                },
                messages,
                events,
                summary: make_summary!(TurnFinishReason::StepLimitExceeded),
            });
        }

        // —— 上下文优化管线（微压缩 → 裁剪 → 自动压缩）——
        let assembled = assemble_prompt_request(AssemblePromptRequest {
            gateway,
            prompt_facts_provider: request.prompt_facts_provider.as_ref(),
            session_id: &request.session_id,
            turn_id: &request.turn_id,
            working_dir: Path::new(&request.working_dir),
            messages,
            cancel: request.cancel.clone(),
            agent: &request.agent,
            step_index,
            token_tracker: &token_tracker,
            tools: tools.clone(),
            settings: &settings,
            clearable_tools: &clearable_tools,
            micro_compact_state: &mut micro_compact_state,
            file_access_tracker: &file_access_tracker,
        })
        .await?;
        messages = assembled.messages;
        events.extend(assembled.events);

        // —— LLM 调用（委托 llm_cycle）——
        let output = match llm_cycle::call_llm_streaming(
            gateway,
            assembled.llm_request,
            &request.turn_id,
            &request.agent,
            &request.cancel,
            &mut events,
        )
        .await
        {
            Ok(output) => output,
            Err(e) => {
                // —— Reactive compact 错误恢复（委托 compaction_cycle）——
                if llm_cycle::is_prompt_too_long(&e)
                    && reactive_compact_attempts < compaction_cycle::MAX_REACTIVE_COMPACT_ATTEMPTS
                {
                    reactive_compact_attempts += 1;
                    log::warn!(
                        "turn {} step {}: prompt too long, reactive compact ({}/{})",
                        request.turn_id,
                        step_index,
                        reactive_compact_attempts,
                        compaction_cycle::MAX_REACTIVE_COMPACT_ATTEMPTS,
                    );

                    let recovery =
                        compaction_cycle::try_reactive_compact(&ReactiveCompactContext {
                            gateway,
                            prompt_facts_provider: request.prompt_facts_provider.as_ref(),
                            messages: &messages,
                            session_id: &request.session_id,
                            working_dir: &request.working_dir,
                            turn_id: &request.turn_id,
                            step_index,
                            agent: &request.agent,
                            cancel: request.cancel.clone(),
                            settings: &settings,
                            file_access_tracker: &file_access_tracker,
                        })
                        .await?;

                    match recovery {
                        Some(result) => {
                            events.extend(result.events);
                            messages = result.messages;
                            continue;
                        },
                        None => return Err(e),
                    }
                }
                return Err(e);
            },
        };

        // —— 记录 token 使用量 ——
        token_tracker.record_usage(output.usage);
        if let Some(usage) = &output.usage {
            last_delta_tokens = usage.output_tokens;
            total_cache_read_tokens =
                total_cache_read_tokens.saturating_add(usage.cache_read_input_tokens as u64);
            total_cache_creation_tokens = total_cache_creation_tokens
                .saturating_add(usage.cache_creation_input_tokens as u64);
        }

        let content = output.content.trim().to_string();
        let has_tool_calls = !output.tool_calls.is_empty();

        // 追加 assistant 消息到历史
        messages.push(LlmMessage::Assistant {
            content: content.clone(),
            tool_calls: output.tool_calls.clone(),
            reasoning: output.reasoning.clone(),
        });

        // 发出 AssistantFinal 事件
        events.push(StorageEvent {
            turn_id: Some(request.turn_id.clone()),
            agent: request.agent.clone(),
            payload: StorageEventPayload::AssistantFinal {
                content,
                reasoning_content: output.reasoning.as_ref().map(|r| r.content.clone()),
                reasoning_signature: output.reasoning.as_ref().and_then(|r| r.signature.clone()),
                timestamp: Some(Utc::now()),
            },
        });

        // 检查 max_tokens 截断
        if matches!(
            output.finish_reason,
            astrcode_core::LlmFinishReason::MaxTokens
        ) {
            log::warn!(
                "turn {} step {}: LLM output truncated by max_tokens",
                request.turn_id,
                step_index
            );
        }

        // —— 无工具调用时，检查 token budget 驱动的自动续写 ——
        if !has_tool_calls {
            let is_max_tokens = matches!(
                output.finish_reason,
                astrcode_core::LlmFinishReason::MaxTokens
            );

            // 当输出被 max_tokens 截断且有预算时，尝试自动续写
            if is_max_tokens {
                if let Some(budget) = token_budget {
                    let turn_tokens_used = token_tracker.budget_tokens(0) as u64;
                    let decision = check_token_budget(
                        turn_tokens_used,
                        budget,
                        continuation_count,
                        last_delta_tokens,
                        continuation_min_delta_tokens,
                        max_continuations,
                    );

                    match decision {
                        TokenBudgetDecision::Continue => {
                            continuation_count += 1;
                            let nudge = build_auto_continue_nudge(turn_tokens_used, budget);
                            messages.push(LlmMessage::User {
                                content: nudge.clone(),
                                origin: UserMessageOrigin::AutoContinueNudge,
                            });
                            events.push(StorageEvent {
                                turn_id: Some(request.turn_id.clone()),
                                agent: request.agent.clone(),
                                payload: StorageEventPayload::UserMessage {
                                    content: nudge,
                                    origin: UserMessageOrigin::AutoContinueNudge,
                                    timestamp: Utc::now(),
                                },
                            });
                            step_index += 1;
                            continue;
                        },
                        TokenBudgetDecision::Stop => {
                            events.push(StorageEvent {
                                turn_id: Some(request.turn_id.clone()),
                                agent: request.agent.clone(),
                                payload: StorageEventPayload::TurnDone {
                                    timestamp: Utc::now(),
                                    reason: Some("budget_exhausted".to_string()),
                                },
                            });
                            return Ok(TurnRunResult {
                                outcome: TurnOutcome::Completed,
                                messages,
                                events,
                                summary: make_summary!(TurnFinishReason::BudgetExhausted),
                            });
                        },
                        TokenBudgetDecision::DiminishingReturns => {
                            events.push(StorageEvent {
                                turn_id: Some(request.turn_id.clone()),
                                agent: request.agent.clone(),
                                payload: StorageEventPayload::TurnDone {
                                    timestamp: Utc::now(),
                                    reason: Some("diminishing_returns".to_string()),
                                },
                            });
                            return Ok(TurnRunResult {
                                outcome: TurnOutcome::Completed,
                                messages,
                                events,
                                summary: make_summary!(TurnFinishReason::DiminishingReturns),
                            });
                        },
                    }
                }
            }

            // 无需自动续写，Turn 自然结束
            events.push(StorageEvent {
                turn_id: Some(request.turn_id.clone()),
                agent: request.agent.clone(),
                payload: StorageEventPayload::TurnDone {
                    timestamp: Utc::now(),
                    reason: if continuation_count > 0 {
                        Some(format!("continued_{continuation_count}x_ended_naturally"))
                    } else {
                        None
                    },
                },
            });
            return Ok(TurnRunResult {
                outcome: TurnOutcome::Completed,
                messages,
                events,
                summary: make_summary!(TurnFinishReason::NaturalEnd),
            });
        }

        // —— 工具执行（委托 tool_cycle）——
        let tool_result = tool_cycle::execute_tool_calls(
            &mut ToolCycleContext {
                gateway,
                session_id: &request.session_id,
                working_dir: &request.working_dir,
                turn_id: &request.turn_id,
                agent: &request.agent,
                cancel: &request.cancel,
                events: &mut events,
                max_concurrency: request.runtime.max_tool_concurrency.unwrap_or(10),
            },
            output.tool_calls,
        )
        .await?;

        // 更新追踪器
        for (call, result) in &tool_result.raw_results {
            file_access_tracker.record_tool_result(call, result, Path::new(&request.working_dir));
            micro_compact_state.record_tool_result(result.tool_call_id.clone(), Instant::now());
        }

        // 追加工具结果消息到历史
        messages.extend(tool_result.tool_messages);

        if matches!(tool_result.outcome, ToolCycleOutcome::Interrupted) {
            return Ok(TurnRunResult {
                outcome: TurnOutcome::Cancelled,
                messages,
                events,
                summary: make_summary!(TurnFinishReason::Cancelled),
            });
        }

        step_index += 1;
    }
}
