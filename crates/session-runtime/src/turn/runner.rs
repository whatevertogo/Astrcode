//! Turn 执行器。
//!
//! 实现一个完整的 Agent Turn：LLM 调用 → 工具执行 → 循环直到完成。
//! 所有 provider 调用通过 `kernel` gateway 进行，不直接持有 provider。
//!
//! ## 架构：纯编排器
//!
//! `run_turn` 只负责 step 循环的编排，所有细节委托给子模块：
//! - `request` — 最终请求拼装（微压缩 → 裁剪 → 自动压缩 → prompt request）
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
//! - 取消信号触发
//! - 不可恢复错误
//! - Step 上限

mod step;

use std::{collections::HashSet, path::Path, sync::Arc, time::Instant};

use astrcode_core::{
    AgentEventContext, BoundModeToolContractSnapshot, CancelToken, EventStore, EventTranslator,
    LlmMessage, ModeId, Phase, PromptDeclaration, PromptFactsProvider, PromptGovernanceContext,
    ResolvedRuntimeConfig, Result, SESSION_PLAN_DRAFT_APPROVAL_GUARD_MARKER, StorageEvent,
    StorageEventPayload, ToolDefinition, UserMessageOrigin,
};
use astrcode_kernel::{CapabilityRouter, Kernel, KernelGateway};
use chrono::{DateTime, Utc};
use step::{StepOutcome, run_single_step};

use super::{
    TurnOutcome,
    journal::TurnJournal,
    loop_control::{TurnLoopTransition, TurnStopCause},
    summary::{TurnCollaborationSummary, TurnFinishReason, TurnSummary},
};
use crate::{
    SessionState,
    context_window::{
        ContextWindowSettings, file_access::FileAccessTracker, micro_compact::MicroCompactState,
        token_usage::TokenUsageTracker,
    },
    turn::{
        events::turn_terminal_event, finalize::persist_storage_events,
        tool_result_budget::ToolResultReplacementState,
    },
};

/// 可清除的工具名称（这些工具的旧结果可以被 prune pass 替换为占位文本）。
/// 工具结果可被 prune pass 替换为占位文本的工具名称。
/// 这些工具的输出是文件内容，prune 时可以安全替换（需要时重新读取即可）。
const CLEARABLE_TOOLS: &[&str] = &["readFile", "listDir", "grep", "findFiles"];

/// Turn 执行请求。
pub(crate) struct TurnRunRequest {
    pub event_store: Arc<dyn EventStore>,
    pub session_id: String,
    pub working_dir: String,
    pub turn_id: String,
    pub messages: Vec<LlmMessage>,
    pub last_assistant_at: Option<DateTime<Utc>>,
    pub session_state: Arc<SessionState>,
    pub runtime: ResolvedRuntimeConfig,
    pub cancel: CancelToken,
    pub agent: AgentEventContext,
    pub current_mode_id: ModeId,
    pub prompt_facts_provider: Arc<dyn PromptFactsProvider>,
    pub capability_router: Option<CapabilityRouter>,
    pub prompt_declarations: Vec<PromptDeclaration>,
    pub bound_mode_tool_contract: Option<BoundModeToolContractSnapshot>,
    pub prompt_governance: Option<PromptGovernanceContext>,
}

/// Turn 执行结果。
pub(crate) struct TurnRunResult {
    pub outcome: TurnOutcome,
    /// Turn 结束时的完整消息历史（含本次 turn 新增的）。
    pub messages: Vec<LlmMessage>,
    /// run_turn 返回后仍需由 finalize 兜底持久化的尾部事件。
    pub events: Vec<StorageEvent>,
    /// Turn 级稳定汇总（包含耗时、token、续写等指标）。
    pub summary: TurnSummary,
}

struct TurnExecutionResources<'a> {
    gateway: &'a astrcode_kernel::KernelGateway,
    prompt_facts_provider: &'a dyn PromptFactsProvider,
    session_id: &'a str,
    working_dir: &'a str,
    turn_id: &'a str,
    session_state: &'a Arc<SessionState>,
    runtime: &'a ResolvedRuntimeConfig,
    cancel: &'a CancelToken,
    agent: &'a AgentEventContext,
    current_mode_id: &'a ModeId,
    prompt_declarations: &'a [PromptDeclaration],
    bound_mode_tool_contract: Option<&'a BoundModeToolContractSnapshot>,
    prompt_governance: Option<&'a PromptGovernanceContext>,
    tools: Arc<[ToolDefinition]>,
    settings: ContextWindowSettings,
    clearable_tools: HashSet<String>,
    max_steps: usize,
}

struct TurnExecutionRequestView<'a> {
    prompt_facts_provider: &'a dyn PromptFactsProvider,
    session_id: &'a str,
    working_dir: &'a str,
    turn_id: &'a str,
    session_state: &'a Arc<SessionState>,
    runtime: &'a ResolvedRuntimeConfig,
    cancel: &'a CancelToken,
    agent: &'a AgentEventContext,
    current_mode_id: &'a ModeId,
    prompt_declarations: &'a [PromptDeclaration],
    bound_mode_tool_contract: Option<&'a BoundModeToolContractSnapshot>,
    prompt_governance: Option<&'a PromptGovernanceContext>,
}

struct TurnExecutionContext {
    messages: Vec<LlmMessage>,
    draft_plan_approval_guard_active: bool,
    journal: TurnJournal,
    lifecycle: TurnLifecycle,
    budget: TurnBudgetState,
    tool_result_budget: ToolResultBudgetState,
    streaming_tools: StreamingToolState,
}

struct TurnLifecycle {
    turn_started_at: Instant,
    step_index: usize,
    reactive_compact_attempts: usize,
    max_output_continuation_count: usize,
    last_transition: Option<TurnLoopTransition>,
    stop_cause: Option<TurnStopCause>,
}

struct TurnBudgetState {
    token_tracker: TokenUsageTracker,
    total_cache_read_tokens: u64,
    total_cache_creation_tokens: u64,
    auto_compaction_count: usize,
    micro_compact_state: MicroCompactState,
    file_access_tracker: FileAccessTracker,
}

struct ToolResultBudgetState {
    replacement_state: ToolResultReplacementState,
    replacement_count: usize,
    reapply_count: usize,
    bytes_saved: usize,
    over_budget_message_count: usize,
}

struct StreamingToolState {
    launch_count: usize,
    match_count: usize,
    fallback_count: usize,
    discard_count: usize,
    overlap_ms: u64,
}

struct TurnLifecycleSummary {
    finish_reason: TurnFinishReason,
    stop_cause: TurnStopCause,
    last_transition: Option<TurnLoopTransition>,
    wall_duration: std::time::Duration,
    step_count: usize,
    reactive_compact_count: usize,
    max_output_continuation_count: usize,
}

struct TurnBudgetSummary {
    total_tokens_used: u64,
    cache_read_input_tokens: u64,
    cache_creation_input_tokens: u64,
    auto_compaction_count: usize,
}

struct ToolResultBudgetSummary {
    replacement_count: usize,
    reapply_count: usize,
    bytes_saved: u64,
    over_budget_message_count: usize,
}

struct StreamingToolSummary {
    launch_count: usize,
    match_count: usize,
    fallback_count: usize,
    discard_count: usize,
    overlap_ms: u64,
}

impl<'a> TurnExecutionResources<'a> {
    fn new(
        gateway: &'a astrcode_kernel::KernelGateway,
        request: TurnExecutionRequestView<'a>,
    ) -> Self {
        let settings = ContextWindowSettings::from(request.runtime);
        Self {
            gateway,
            prompt_facts_provider: request.prompt_facts_provider,
            session_id: request.session_id,
            working_dir: request.working_dir,
            turn_id: request.turn_id,
            session_state: request.session_state,
            runtime: request.runtime,
            cancel: request.cancel,
            agent: request.agent,
            current_mode_id: request.current_mode_id,
            prompt_declarations: request.prompt_declarations,
            bound_mode_tool_contract: request.bound_mode_tool_contract,
            prompt_governance: request.prompt_governance,
            tools: Arc::from(gateway.capabilities().tool_definitions()),
            settings,
            clearable_tools: CLEARABLE_TOOLS
                .iter()
                .map(|tool| (*tool).to_string())
                .collect(),
            max_steps: request.runtime.max_steps.max(1),
        }
    }
}

impl TurnLifecycle {
    fn new(turn_started_at: Instant) -> Self {
        Self {
            turn_started_at,
            step_index: 0,
            reactive_compact_attempts: 0,
            max_output_continuation_count: 0,
            last_transition: None,
            stop_cause: None,
        }
    }

    fn record_transition(&mut self, transition: TurnLoopTransition) {
        self.last_transition = Some(transition);
    }

    fn summarize(
        &mut self,
        outcome: &TurnOutcome,
        stop_cause: TurnStopCause,
    ) -> TurnLifecycleSummary {
        self.stop_cause = Some(stop_cause);
        let terminal_kind = outcome.terminal_kind(stop_cause);
        TurnLifecycleSummary {
            finish_reason: TurnFinishReason::from(&terminal_kind),
            stop_cause,
            last_transition: self.last_transition,
            wall_duration: self.turn_started_at.elapsed(),
            step_count: self.step_index + 1,
            reactive_compact_count: self.reactive_compact_attempts,
            max_output_continuation_count: self.max_output_continuation_count,
        }
    }
}

impl TurnBudgetState {
    fn new(
        resources: &TurnExecutionResources<'_>,
        messages: &[LlmMessage],
        turn_started_at: Instant,
        last_assistant_at: Option<DateTime<Utc>>,
    ) -> Self {
        Self {
            token_tracker: TokenUsageTracker::default(),
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
            auto_compaction_count: 0,
            micro_compact_state: MicroCompactState::seed_from_messages(
                messages,
                resources.settings.micro_compact_config(),
                turn_started_at,
                last_assistant_at,
            ),
            file_access_tracker: FileAccessTracker::seed_from_messages(
                messages,
                resources.settings.max_tracked_files,
                Path::new(resources.working_dir),
            ),
        }
    }

    fn summarize(&self) -> TurnBudgetSummary {
        TurnBudgetSummary {
            total_tokens_used: self.token_tracker.budget_tokens(0) as u64,
            cache_read_input_tokens: self.total_cache_read_tokens,
            cache_creation_input_tokens: self.total_cache_creation_tokens,
            auto_compaction_count: self.auto_compaction_count,
        }
    }
}

impl ToolResultBudgetState {
    fn new(resources: &TurnExecutionResources<'_>) -> Self {
        Self {
            replacement_state: ToolResultReplacementState::seed(resources.session_state)
                .unwrap_or_default(),
            replacement_count: 0,
            reapply_count: 0,
            bytes_saved: 0,
            over_budget_message_count: 0,
        }
    }

    fn summarize(&self) -> ToolResultBudgetSummary {
        ToolResultBudgetSummary {
            replacement_count: self.replacement_count,
            reapply_count: self.reapply_count,
            bytes_saved: self.bytes_saved as u64,
            over_budget_message_count: self.over_budget_message_count,
        }
    }
}

impl StreamingToolState {
    fn new() -> Self {
        Self {
            launch_count: 0,
            match_count: 0,
            fallback_count: 0,
            discard_count: 0,
            overlap_ms: 0,
        }
    }

    fn summarize(&self) -> StreamingToolSummary {
        StreamingToolSummary {
            launch_count: self.launch_count,
            match_count: self.match_count,
            fallback_count: self.fallback_count,
            discard_count: self.discard_count,
            overlap_ms: self.overlap_ms,
        }
    }
}

impl TurnExecutionContext {
    fn new(
        resources: &TurnExecutionResources<'_>,
        messages: Vec<LlmMessage>,
        last_assistant_at: Option<DateTime<Utc>>,
    ) -> Self {
        let now = Instant::now();
        let budget = TurnBudgetState::new(resources, &messages, now, last_assistant_at);
        Self {
            draft_plan_approval_guard_active: messages.iter().any(|message| {
                matches!(
                    message,
                    LlmMessage::User { content, origin }
                        if *origin == UserMessageOrigin::ReactivationPrompt
                            && content.contains(SESSION_PLAN_DRAFT_APPROVAL_GUARD_MARKER)
                )
            }),
            messages,
            journal: TurnJournal::default(),
            lifecycle: TurnLifecycle::new(now),
            budget,
            tool_result_budget: ToolResultBudgetState::new(resources),
            streaming_tools: StreamingToolState::new(),
        }
    }

    fn finish(
        mut self,
        resources: &TurnExecutionResources<'_>,
        outcome: TurnOutcome,
        stop_cause: TurnStopCause,
    ) -> TurnRunResult {
        let lifecycle = self.lifecycle.summarize(&outcome, stop_cause);
        let budget = self.budget.summarize();
        let tool_result_budget = self.tool_result_budget.summarize();
        let streaming_tools = self.streaming_tools.summarize();
        TurnRunResult {
            outcome,
            messages: self.messages,
            events: Vec::new(),
            summary: TurnSummary {
                finish_reason: lifecycle.finish_reason,
                stop_cause: lifecycle.stop_cause,
                last_transition: lifecycle.last_transition,
                wall_duration: lifecycle.wall_duration,
                step_count: lifecycle.step_count,
                total_tokens_used: budget.total_tokens_used,
                cache_read_input_tokens: budget.cache_read_input_tokens,
                cache_creation_input_tokens: budget.cache_creation_input_tokens,
                auto_compaction_count: budget.auto_compaction_count,
                reactive_compact_count: lifecycle.reactive_compact_count,
                max_output_continuation_count: lifecycle.max_output_continuation_count,
                tool_result_replacement_count: tool_result_budget.replacement_count,
                tool_result_reapply_count: tool_result_budget.reapply_count,
                tool_result_bytes_saved: tool_result_budget.bytes_saved,
                tool_result_over_budget_message_count: tool_result_budget.over_budget_message_count,
                streaming_tool_launch_count: streaming_tools.launch_count,
                streaming_tool_match_count: streaming_tools.match_count,
                streaming_tool_fallback_count: streaming_tools.fallback_count,
                streaming_tool_discard_count: streaming_tools.discard_count,
                streaming_tool_overlap_ms: streaming_tools.overlap_ms,
                collaboration: turn_collaboration_summary(
                    resources.session_state,
                    resources.turn_id,
                ),
            },
        }
    }
}

/// 执行一个完整的 Agent Turn。
///
/// 通过 `kernel` gateway 调用 LLM 和工具，不直接持有 provider。
/// 每个重要步骤通过事件回调发出。
pub async fn run_turn(kernel: Arc<Kernel>, request: TurnRunRequest) -> Result<TurnRunResult> {
    let TurnRunRequest {
        event_store,
        session_id,
        working_dir,
        turn_id,
        messages,
        last_assistant_at,
        session_state,
        runtime,
        cancel,
        agent,
        current_mode_id,
        prompt_facts_provider,
        capability_router,
        prompt_declarations,
        bound_mode_tool_contract,
        prompt_governance,
    } = request;
    let gateway = scoped_gateway(kernel.gateway(), capability_router)?;
    let resources = TurnExecutionResources::new(
        &gateway,
        TurnExecutionRequestView {
            prompt_facts_provider: prompt_facts_provider.as_ref(),
            session_id: &session_id,
            working_dir: &working_dir,
            turn_id: &turn_id,
            session_state: &session_state,
            runtime: &runtime,
            cancel: &cancel,
            agent: &agent,
            current_mode_id: &current_mode_id,
            prompt_declarations: &prompt_declarations,
            bound_mode_tool_contract: bound_mode_tool_contract.as_ref(),
            prompt_governance: prompt_governance.as_ref(),
        },
    );
    let mut execution = TurnExecutionContext::new(&resources, messages, last_assistant_at);
    let mut translator = EventTranslator::new(session_state.current_phase().unwrap_or(Phase::Idle));

    loop {
        if resources.cancel.is_cancelled() {
            execution.journal.clear();
            execution.journal.push(turn_terminal_event(
                resources.turn_id,
                resources.agent,
                TurnStopCause::Cancelled,
                Utc::now(),
            ));
            flush_pending_events(
                &event_store,
                resources.session_state,
                resources.session_id,
                &mut translator,
                &mut execution.journal,
            )
            .await?;
            return Ok(execution.finish(
                &resources,
                TurnOutcome::Cancelled,
                TurnStopCause::Cancelled,
            ));
        }

        if execution.lifecycle.step_index >= resources.max_steps {
            execution.journal.clear();
            execution.journal.push(turn_terminal_event(
                resources.turn_id,
                resources.agent,
                TurnStopCause::StepLimitExceeded,
                Utc::now(),
            ));
            flush_pending_events(
                &event_store,
                resources.session_state,
                resources.session_id,
                &mut translator,
                &mut execution.journal,
            )
            .await?;
            return Ok(execution.finish(
                &resources,
                TurnOutcome::Error {
                    message: format!("turn exceeded maximum steps ({})", resources.max_steps),
                },
                TurnStopCause::StepLimitExceeded,
            ));
        }

        match run_single_step(&mut execution, &resources).await? {
            StepOutcome::Continue(transition) => {
                flush_pending_events(
                    &event_store,
                    resources.session_state,
                    resources.session_id,
                    &mut translator,
                    &mut execution.journal,
                )
                .await?;
                execution.lifecycle.record_transition(transition);
            },
            StepOutcome::Completed(stop_cause) => {
                execution.journal.push(turn_terminal_event(
                    resources.turn_id,
                    resources.agent,
                    stop_cause,
                    Utc::now(),
                ));
                flush_pending_events(
                    &event_store,
                    resources.session_state,
                    resources.session_id,
                    &mut translator,
                    &mut execution.journal,
                )
                .await?;
                return Ok(execution.finish(&resources, TurnOutcome::Completed, stop_cause));
            },
            StepOutcome::Cancelled(stop_cause) => {
                execution.journal.clear();
                execution.journal.push(turn_terminal_event(
                    resources.turn_id,
                    resources.agent,
                    stop_cause,
                    Utc::now(),
                ));
                flush_pending_events(
                    &event_store,
                    resources.session_state,
                    resources.session_id,
                    &mut translator,
                    &mut execution.journal,
                )
                .await?;
                return Ok(execution.finish(&resources, TurnOutcome::Cancelled, stop_cause));
            },
        }
    }
}

async fn flush_pending_events(
    event_store: &Arc<dyn EventStore>,
    session_state: &Arc<SessionState>,
    session_id: &str,
    translator: &mut EventTranslator,
    journal: &mut TurnJournal,
) -> Result<()> {
    if journal.is_empty() {
        return Ok(());
    }
    let events = journal.take_events();
    persist_storage_events(event_store, session_state, session_id, translator, &events)
        .await
        .map(|_| ())
}

fn scoped_gateway(
    gateway: &KernelGateway,
    capability_router: Option<CapabilityRouter>,
) -> Result<KernelGateway> {
    Ok(match capability_router {
        Some(router) => gateway.with_capabilities(router),
        None => gateway.clone(),
    })
}

/// 从 session 事件流中聚合当前 turn 的 collaboration facts，生成 turn 级摘要。
fn turn_collaboration_summary(
    session_state: &SessionState,
    turn_id: &str,
) -> TurnCollaborationSummary {
    let facts = session_state
        .snapshot_recent_stored_events()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|stored| match stored.event.payload {
            StorageEventPayload::AgentCollaborationFact { fact, .. }
                if stored.event.turn_id() == Some(turn_id) =>
            {
                Some(fact)
            },
            _ => None,
        })
        .collect::<Vec<_>>();
    TurnCollaborationSummary::from_facts(&facts)
}
