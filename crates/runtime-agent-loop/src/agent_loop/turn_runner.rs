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
    AgentEventContext, AgentState, AstrError, CancelToken, ContextStrategy, ExecutionOwner,
    LoopRunnerBoundary, PromptMetricsPayload, Result, StorageEvent, StorageEventPayload,
};
use astrcode_runtime_llm::LlmProvider;
use astrcode_runtime_prompt::{DiagnosticLevel, PromptDeclaration, PromptDiagnostics};
use async_trait::async_trait;

use super::{
    AgentLoop, TurnOutcome,
    compaction_cycle::{
        CompactContext, ReactiveCompactContext, ReactiveCompactOutcome,
        handle_llm_error_with_reactive_compact, maybe_compact_conversation,
    },
    finish_interrupted, finish_turn, finish_with_error, internal_error, llm_cycle, tool_cycle,
};
use crate::{
    compaction_runtime::{CompactionReason, CompactionTailSnapshot},
    context_pipeline::{
        CompactionView, ContextBlock, ContextBundleInput, ConversationView, RecoveryRef,
    },
    context_window::{
        TokenUsageTracker, file_access::FileAccessTracker, token_usage::PromptTokenSnapshot,
    },
    request_assembler::{PreparedRequest, StepRequestConfig},
};

// ---------------------------------------------------------------------------
// Error recovery constants (P4)
// ---------------------------------------------------------------------------

/// max_tokens 截断时自动继续生成的最大次数。
/// 超过此次数后即使模型仍被截断也终止 turn，避免无限循环。
/// TODO: 更好的数字和可能的可配置化
const MAX_OUTPUT_CONTINUATION_ATTEMPTS: usize = 3;

#[derive(Debug, Clone, Copy, Default)]
struct PromptCacheReuseSummary {
    hits: u32,
    misses: u32,
}

// ---------------------------------------------------------------------------
// Step 内部辅助结构
// ---------------------------------------------------------------------------

/// 单个 step 内用于构建 `PromptMetricsPayload` 的公共数据。
///
/// 两次 metrics 发射（LLM 调用前预估值 + LLM 响应后实际值）共享同一组
/// snapshot/cache 数据，差异仅在 provider token 字段。
struct StepMetrics<'a> {
    step_index: u32,
    snapshot: &'a PromptTokenSnapshot,
    truncated_tool_results: usize,
    cache_reuse: PromptCacheReuseSummary,
    cache_metrics_supported: bool,
}

impl StepMetrics<'_> {
    /// 构建 LLM 调用前的预估 metrics（provider 字段全部为 None）。
    fn estimated_payload(&self) -> PromptMetricsPayload {
        let s = self.snapshot;
        PromptMetricsPayload {
            step_index: self.step_index,
            estimated_tokens: s.context_tokens.min(u32::MAX as usize) as u32,
            context_window: s.context_window.min(u32::MAX as usize) as u32,
            effective_window: s.effective_window.min(u32::MAX as usize) as u32,
            threshold_tokens: s.threshold_tokens.min(u32::MAX as usize) as u32,
            truncated_tool_results: self.truncated_tool_results.min(u32::MAX as usize) as u32,
            provider_input_tokens: None,
            provider_output_tokens: None,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
            provider_cache_metrics_supported: self.cache_metrics_supported,
            prompt_cache_reuse_hits: self.cache_reuse.hits,
            prompt_cache_reuse_misses: self.cache_reuse.misses,
        }
    }

    /// 构建 LLM 响应后的实际 metrics，填入 provider 返回的精确 token 用量。
    fn actual_payload(
        &self,
        usage: astrcode_runtime_llm::LlmUsage,
        provider: &dyn LlmProvider,
    ) -> PromptMetricsPayload {
        let mut payload = self.estimated_payload();
        payload.provider_input_tokens = Some(
            provider
                .prompt_metrics_input_tokens(usage)
                .min(u32::MAX as usize) as u32,
        );
        payload.provider_output_tokens = Some(usage.output_tokens.min(u32::MAX as usize) as u32);
        payload.cache_creation_input_tokens = self
            .cache_metrics_supported
            .then_some(usage.cache_creation_input_tokens.min(u32::MAX as usize) as u32);
        payload.cache_read_input_tokens = self
            .cache_metrics_supported
            .then_some(usage.cache_read_input_tokens.min(u32::MAX as usize) as u32);
        payload
    }
}

/// Step 循环内的可变状态。
///
/// 将 step 循环中频繁变更的状态聚合到一处，`run_turn` 的初始化段构造一次，
/// 后续通过方法操作，避免散落的 `let mut` 声明。
struct StepState {
    conversation: ConversationView,
    recovered_memory: Vec<ContextBlock>,
    recovery_refs: Vec<RecoveryRef>,
    step_index: usize,
    output_continuation_count: u8,
    reactive_compact_attempts: usize,
    token_tracker: TokenUsageTracker,
    file_access: FileAccessTracker,
}

impl StepState {
    /// 将 compaction 结果应用到当前状态，替换对话历史和恢复数据。
    fn apply_compaction(&mut self, view: CompactionView) {
        self.recovered_memory = view.memory_blocks;
        self.recovery_refs = view.recovery_refs;
        self.conversation = ConversationView::new(view.messages);
    }
}

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
    pub(crate) runtime_prompt_declarations: &'a [PromptDeclaration],
}

/// 执行一个完整的 agent turn。
///
/// 整体流程分为三段：
///
/// ```text
/// ┌───────────── 初始化 ─────────────┐
/// │  构造 Provider、ConversationView │
/// │  初始化 token/file 追踪器        │
/// └──────────────────────────────────┘
///          ↓
/// ┌── Step 循环 (可能多轮) ──────────────────────────────────┐
/// │  ① 组装上下文 → 构建 prompt plan → 装配 LLM 请求        │
/// │  ② Policy 决策是否需要自动压缩上下文                      │
/// │  ③ 调用 LLM → 若 prompt too long 则触发 reactive compact │
/// │  ④ 处理输出：空响应/截断续命/工具调用/纯文本结束          │
/// │  ⑤ 若有工具调用 → 执行工具 → 回到 ①                      │
/// └──────────────────────────────────────────────────────────┘
/// ```
pub(crate) async fn run_turn<F>(ctx: TurnRunContext<'_, F>) -> Result<TurnOutcome>
where
    F: FnMut(StorageEvent) -> Result<()>,
{
    // ── 初始化阶段：解构上下文、构造 LLM Provider ──────────────────────
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
        runtime_prompt_declarations,
    } = ctx;

    // 根据 factory 构造本次 turn 使用的 LLM Provider（含 working_dir 用于文件类工具）
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

    // 会话视图：在 turn 内部逐步追加 assistant/tool 消息，compact 时整体替换
    // 从之前 compaction 中恢复的内存块和恢复引用，compact 后会更新
    let mut step_state = StepState {
        conversation: ConversationView::new(state.messages.clone()),
        recovered_memory: Vec::new(),
        recovery_refs: Vec::new(),
        step_index: 0,
        output_continuation_count: 0,
        reactive_compact_attempts: 0,
        token_tracker: TokenUsageTracker::default(),
        file_access: FileAccessTracker::from_stored_events(&compaction_tail.materialize()),
    };
    let model_limits = provider.model_limits();

    // ── Step 循环：每轮 = 一次 LLM 调用 + 可选的工具执行 ────────────────
    'step: loop {
        // 取消是协作式的，在 step 边界检查。若 LLM 正在执行慢速推理，
        // 此处不会立即响应取消——实际的中断由 generate_response 内部的
        // CancelToken 机制处理（取消时 provider 的 HTTP 连接会被 abort）。
        if cancel.is_cancelled() {
            return report_interrupted(turn_id, &agent, on_event, emit_turn_done);
        }

        // ── ① 组装上下文 ──────────────────────────────────────────────
        // 将当前对话消息、恢复的内存块等打包为 CompactionView，
        // 供 context pipeline 判断需要保留/压缩哪些内容。
        let prior_compaction_view = CompactionView {
            messages: step_state.conversation.messages.clone(),
            memory_blocks: step_state.recovered_memory.clone(),
            recovery_refs: step_state.recovery_refs.clone(),
        };
        let bundle = match agent_loop.context.build_bundle(
            state,
            ContextBundleInput {
                turn_id,
                step_index: step_state.step_index,
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

        // ── ② 构建 Prompt Plan ─────────────────────────────────────────
        // 合并 runtime 级别的 prompt 声明（如动态注入的工具描述），
        // 然后由 prompt planner 根据对话状态生成最终的 prompt 排布方案。
        let mut prompt_declarations = bundle.prompt_declarations();
        prompt_declarations.extend_from_slice(runtime_prompt_declarations);
        let build_output = match agent_loop
            .prompt
            .build_plan(
                state,
                &bundle.conversation,
                &prompt_declarations,
                step_state.step_index,
            )
            .await
        {
            Ok(output) => output,
            Err(error) => {
                return report_error(turn_id, error.to_string(), &agent, on_event, emit_turn_done);
            },
        };
        // 汇总 prompt 层面的 cache 命中/未命中情况，用于后续 metrics 上报
        let prompt_cache_reuse = summarize_prompt_cache_reuse(&build_output.diagnostics);
        log_prompt_diagnostics(&build_output.diagnostics);
        let plan = build_output.plan;

        // ── ③ 装配 LLM 请求 ──────────────────────────────────────────
        // 根据 prompt plan 和 token 预算，组装最终发给 Provider 的 request，
        // 包括系统提示词裁剪、工具结果截断等。
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
            &step_state.token_tracker,
        ) {
            Ok(prepared) => prepared,
            Err(error) => {
                return report_error(turn_id, error.to_string(), &agent, on_event, emit_turn_done);
            },
        };
        // ── ④ 发送预估 token metrics ──────────────────────────────────
        // 在调用 LLM 之前先上报基于 prompt plan 的预估 token 用量，
        // 上层可以在 LLM 响应到达前就展示进度。
        let step_metrics = StepMetrics {
            step_index: step_state.step_index as u32,
            snapshot: &prompt_snapshot,
            truncated_tool_results,
            cache_reuse: prompt_cache_reuse,
            cache_metrics_supported: provider.supports_cache_metrics(),
        };
        emit_event_with_file_tracking(
            &mut step_state.file_access,
            on_event,
            StorageEvent {
                turn_id: Some(turn_id.to_string()),
                agent: agent.clone(),
                payload: StorageEventPayload::PromptMetrics {
                    metrics: step_metrics.estimated_payload(),
                },
            },
        )?;
        // ── ⑤ Policy 决策：是否需要自动压缩上下文 ──────────────────────
        // 将 prompt 快照交给 compaction 策略判断当前上下文是否接近窗口阈值，
        // 由 policy 引擎决定本轮是正常发请求还是先压缩上下文。
        let decision_input = agent_loop
            .compaction
            .build_context_decision(&prompt_snapshot, truncated_tool_results);
        let policy_ctx = agent_loop.policy_context(state, turn_id, step_state.step_index);
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
        // ── ⑥ Policy 检查：可能改写 LLM 请求 ──────────────────────────
        // policy 引擎可以修改请求（如替换 system prompt），需要检测是否发生了改写，
        // 若改写则清空旧的 system_prompt_blocks 以避免缓存不一致。
        // 此步骤必须在 compact 之前执行，这样 proactive compact 和 reactive compact
        // 都能拿到 policy 改写后的完整 system prompt，避免摘要基于过时指令生成。
        let policy_ctx = agent_loop.policy_context(state, turn_id, step_state.step_index);
        let original_system_prompt = request.system_prompt.clone();
        let mut request = match agent_loop
            .policy
            .check_model_request(request, &policy_ctx)
            .await
        {
            Ok(request) => request,
            Err(error) => {
                return report_error(turn_id, error.to_string(), &agent, on_event, emit_turn_done);
            },
        };
        if request.system_prompt != original_system_prompt {
            request.system_prompt_blocks.clear();
        }
        // policy 改写后的 system prompt，同时供 proactive 和 reactive compact 使用。
        let runtime_system_prompt_for_compact = request.system_prompt.clone();

        // ── ⑦ 执行自动压缩（如果 Policy 决策需要） ─────────────────────
        // Policy 判定上下文已接近窗口阈值时，在调用 LLM 之前先压缩一次，
        // 压缩后 continue 回到 step 循环顶部重新组装上下文。
        // 使用 policy 改写后的 system prompt，确保摘要基于完整指令上下文。
        if matches!(context_strategy, ContextStrategy::Compact) {
            let tools = agent_loop.capabilities.tool_definitions();
            let compact_ctx = CompactContext {
                state,
                provider: &provider,
                conversation: &step_state.conversation,
                runtime_system_prompt: runtime_system_prompt_for_compact.as_deref(),
                reason: CompactionReason::Auto,
                turn_id,
                agent: &agent,
                cancel: cancel.clone(),
                tail: compaction_tail.clone(),
                tools,
                file_access: &step_state.file_access,
            };
            match maybe_compact_conversation(agent_loop, &compact_ctx, on_event).await {
                Ok(Some(compacted_view)) => {
                    step_state.apply_compaction(compacted_view);
                    continue;
                },
                Ok(None) => {},
                Err(error) => {
                    return if error.is_cancelled() {
                        report_interrupted(turn_id, &agent, on_event, emit_turn_done)
                    } else {
                        report_error(turn_id, error.to_string(), &agent, on_event, emit_turn_done)
                    };
                },
            }
        }
        // ── ⑧ 调用 LLM ────────────────────────────────────────────────
        // 将组装好的请求发送给 Provider，流式接收响应。
        // 若返回 prompt too long 错误且 auto_compact 开启，则触发 reactive compact。
        // reactive compact 同样使用步骤⑥中 policy 改写后的 system prompt。
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
                match handle_llm_error_with_reactive_compact(
                    agent_loop,
                    ReactiveCompactContext {
                        llm_error: error,
                        compact: CompactContext {
                            state,
                            provider: &provider,
                            conversation: &step_state.conversation,
                            runtime_system_prompt: runtime_system_prompt_for_compact.as_deref(),
                            reason: CompactionReason::Reactive,
                            turn_id,
                            agent: &agent,
                            cancel: cancel.clone(),
                            tail: compaction_tail.clone(),
                            tools: agent_loop.capabilities.tool_definitions(),
                            file_access: &step_state.file_access,
                        },
                        reactive_compact_attempts: &mut step_state.reactive_compact_attempts,
                        emit_turn_done,
                    },
                    on_event,
                )
                .await?
                {
                    ReactiveCompactOutcome::Recovered(view) => {
                        step_state.apply_compaction(view);
                        continue 'step;
                    },
                    ReactiveCompactOutcome::Terminal(outcome) => return Ok(outcome),
                }
            },
        };
        // reactive compact 成功后重置计数器，回到 step 循环顶部重新组装请求
        step_state.reactive_compact_attempts = 0;
        // 累积记录本次 step 的 token 用量，供后续 step 的上下文预算参考
        step_state.token_tracker.record_usage(output.usage);
        // ── ⑨ 发送实际 token metrics ──────────────────────────────────
        // LLM 响应到达后，用 Provider 返回的真实 token 用量覆盖之前的预估值，
        // 包含 input/output/cache 各维度的精确数据。
        if let Some(usage) = output.usage {
            emit_event_with_file_tracking(
                &mut step_state.file_access,
                on_event,
                StorageEvent {
                    turn_id: Some(turn_id.to_string()),
                    agent: agent.clone(),
                    payload: StorageEventPayload::PromptMetrics {
                        metrics: step_metrics.actual_payload(usage, provider.as_ref()),
                    },
                },
            )?;
        }

        // ── ⑩ 空响应防护 ─────────────────────────────────────────────
        // Provider 有时返回无内容、无推理、无工具调用的空 completion，
        // 这属于异常情况，直接报错终止 turn。
        if is_empty_completion(&output) {
            return report_error(
                turn_id,
                "provider returned an empty completion without content, reasoning, or tool calls",
                &agent,
                on_event,
                emit_turn_done,
            );
        }

        // ── ⑪ 发送 assistant 消息事件 ──────────────────────────────────
        // 将 LLM 的输出（文本/推理/工具调用）作为 assistant 消息发送给上层，
        // 上层会将其展示给用户或持久化到存储。
        if !output.content.is_empty() || !output.tool_calls.is_empty() || output.reasoning.is_some()
        {
            emit_event_with_file_tracking(
                &mut step_state.file_access,
                on_event,
                StorageEvent {
                    turn_id: Some(turn_id.to_string()),
                    agent: agent.clone(),
                    payload: StorageEventPayload::AssistantFinal {
                        content: output.content.clone(),
                        reasoning_content: output
                            .reasoning
                            .as_ref()
                            .map(|value| value.content.clone()),
                        reasoning_signature: output
                            .reasoning
                            .as_ref()
                            .and_then(|value| value.signature.clone()),
                        timestamp: Some(chrono::Utc::now()),
                    },
                },
            )?;
        }

        // ── ⑫ 将 assistant 响应追加到会话历史 ─────────────────────────
        // 后续的 compact 和下一次 step 的 prompt 组装都会用到这个追加后的历史。
        let tool_calls = output.tool_calls.clone();
        step_state
            .conversation
            .messages
            .push(astrcode_core::LlmMessage::Assistant {
                content: output.content,
                tool_calls: output.tool_calls,
                reasoning: output.reasoning,
            });

        // ── ⑬ max_tokens 截断续命 ─────────────────────────────────────
        // 当模型因 max_tokens 限制被截断时，自动注入一条 nudge 消息鼓励继续生成。
        // 最多重试 MAX_OUTPUT_CONTINUATION_ATTEMPTS 次，防止无限循环。
        if output.finish_reason.is_max_tokens() {
            if step_state.output_continuation_count < MAX_OUTPUT_CONTINUATION_ATTEMPTS as u8 {
                step_state.output_continuation_count += 1;
                log::warn!(
                    "[turn {}] output truncated by max_tokens, injecting continue nudge ({}/{})",
                    turn_id,
                    step_state.output_continuation_count,
                    MAX_OUTPUT_CONTINUATION_ATTEMPTS
                );

                // 注入 nudge 消息，告诉模型继续生成
                step_state
                    .conversation
                    .messages
                    .push(astrcode_core::LlmMessage::User {
                        content: "Continue from where you left off. Do not repeat or summarize."
                            .to_string(),
                        origin: astrcode_core::UserMessageOrigin::AutoContinueNudge,
                    });

                // 不终止 turn，继续下一轮 step 循环
                step_state.step_index += 1;
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

        // ── ⑭ 无工具调用 → Turn 完成 ─────────────────────────────────
        // LLM 返回纯文本（没有请求工具调用），意味着 turn 正常结束。
        if tool_calls.is_empty() {
            return complete_turn(
                turn_id,
                TurnOutcome::Completed,
                &agent,
                on_event,
                emit_turn_done,
            );
        }

        // ── ⑮ 执行工具调用 ────────────────────────────────────────────
        // LLM 请求了工具调用，交给 tool_cycle 执行。
        // 工具执行结果会追加到 conversation.messages，然后回到 step 循环顶部，
        // 让 LLM 基于工具结果继续生成下一轮响应。
        let tool_cycle_outcome = match tool_cycle::execute_tool_calls(
            tool_calls,
            tool_cycle::ToolCycleContext {
                agent_loop,
                capabilities: &agent_loop.capabilities,
                turn_id,
                state,
                step_index: step_state.step_index,
                agent: &agent,
                execution_owner: &execution_owner,
                messages: &mut step_state.conversation.messages,
                on_event: &mut |event| {
                    emit_event_with_file_tracking(&mut step_state.file_access, on_event, event)
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

        step_state.step_index += 1;
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

fn is_empty_completion(output: &astrcode_runtime_llm::LlmOutput) -> bool {
    output.content.trim().is_empty() && output.tool_calls.is_empty() && output.reasoning.is_none()
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
        on_event(StorageEvent {
            turn_id: Some(turn_id.to_string()),
            agent: agent.clone(),
            payload: StorageEventPayload::Error {
                message: message.clone(),
                timestamp: Some(chrono::Utc::now()),
            },
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

fn summarize_prompt_cache_reuse(diagnostics: &PromptDiagnostics) -> PromptCacheReuseSummary {
    diagnostics.items.iter().fold(
        PromptCacheReuseSummary::default(),
        |mut summary, diagnostic| {
            match diagnostic.reason {
                astrcode_runtime_prompt::diagnostics::DiagnosticReason::CacheReuseHit {
                    ..
                } => {
                    summary.hits = summary.hits.saturating_add(1);
                },
                astrcode_runtime_prompt::diagnostics::DiagnosticReason::CacheReuseMiss {
                    ..
                } => {
                    summary.misses = summary.misses.saturating_add(1);
                },
                _ => {},
            }
            summary
        },
    )
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

#[async_trait]
pub trait TurnRunnerRuntime: Send + Sync {
    async fn run_session_turn(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> std::result::Result<(), AstrError>;
}

/// `runtime-agent-loop` 对外暴露的主循环 trait surface。
///
/// 具体 session lookup / durable append 仍由外部 owner 注入，loop crate 只保留
/// “run a turn” 这一条稳定 surface，而不是直接依赖 runtime façade。
#[derive(Clone)]
pub struct TurnRunner<T> {
    runtime: T,
}

impl<T> TurnRunner<T> {
    pub fn new(runtime: T) -> Self {
        Self { runtime }
    }

    pub fn runtime(&self) -> &T {
        &self.runtime
    }
}

#[async_trait]
impl<T> LoopRunnerBoundary for TurnRunner<T>
where
    T: TurnRunnerRuntime,
{
    async fn run_session_turn(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> std::result::Result<(), AstrError> {
        self.runtime.run_session_turn(session_id, turn_id).await
    }
}

#[cfg(test)]
mod boundary_tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use async_trait::async_trait;

    use super::{LoopRunnerBoundary, TurnRunner, TurnRunnerRuntime};

    #[derive(Clone)]
    struct StubTurnRunner {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl TurnRunnerRuntime for StubTurnRunner {
        async fn run_session_turn(
            &self,
            _session_id: &str,
            _turn_id: &str,
        ) -> std::result::Result<(), astrcode_core::AstrError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn turn_runner_surface_delegates_loop_boundary_calls() {
        let calls = Arc::new(AtomicUsize::new(0));
        let runner = TurnRunner::new(StubTurnRunner {
            calls: Arc::clone(&calls),
        });

        runner
            .run_session_turn("session-1", "turn-1")
            .await
            .expect("turn should run");

        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
