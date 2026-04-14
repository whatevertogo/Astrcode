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
    AgentEventContext, CancelToken, LlmMessage, PromptFactsProvider, ResolvedRuntimeConfig, Result,
    StorageEvent, StorageEventPayload, ToolDefinition,
};
use astrcode_kernel::Kernel;
use step::{StepOutcome, run_single_step};

use super::{
    TurnOutcome,
    summary::{TurnCollaborationSummary, TurnFinishReason, TurnSummary},
};
use crate::{
    SessionState,
    context_window::{
        ContextWindowSettings, file_access::FileAccessTracker, micro_compact::MicroCompactState,
        token_usage::TokenUsageTracker,
    },
};

/// 可清除的工具名称（这些工具的旧结果可以被 prune pass 替换为占位文本）。
/// 工具结果可被 prune pass 替换为占位文本的工具名称。
/// 这些工具的输出是文件内容，prune 时可以安全替换（需要时重新读取即可）。
const CLEARABLE_TOOLS: &[&str] = &["readFile", "listDir", "grep", "findFiles"];

/// Turn 执行请求。
pub struct TurnRunRequest {
    pub session_id: String,
    pub working_dir: String,
    pub turn_id: String,
    pub messages: Vec<LlmMessage>,
    pub session_state: Arc<SessionState>,
    pub runtime: ResolvedRuntimeConfig,
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
    tools: Vec<ToolDefinition>,
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
}

struct TurnExecutionContext {
    turn_started_at: Instant,
    messages: Vec<LlmMessage>,
    events: Vec<StorageEvent>,
    token_tracker: TokenUsageTracker,
    total_cache_read_tokens: u64,
    total_cache_creation_tokens: u64,
    micro_compact_state: MicroCompactState,
    file_access_tracker: FileAccessTracker,
    step_index: usize,
    reactive_compact_attempts: usize,
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
            tools: gateway.capabilities().tool_definitions(),
            settings,
            clearable_tools: CLEARABLE_TOOLS
                .iter()
                .map(|tool| (*tool).to_string())
                .collect(),
            max_steps: request.runtime.max_steps.max(1),
        }
    }
}

impl TurnExecutionContext {
    fn new(resources: &TurnExecutionResources<'_>, messages: Vec<LlmMessage>) -> Self {
        Self {
            turn_started_at: Instant::now(),
            micro_compact_state: MicroCompactState::seed_from_messages(
                &messages,
                resources.settings.micro_compact_config(),
                Instant::now(),
            ),
            file_access_tracker: FileAccessTracker::seed_from_messages(
                &messages,
                resources.settings.max_tracked_files,
                Path::new(resources.working_dir),
            ),
            messages,
            events: Vec::new(),
            token_tracker: TokenUsageTracker::default(),
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
            step_index: 0,
            reactive_compact_attempts: 0,
        }
    }

    fn finish(
        self,
        resources: &TurnExecutionResources<'_>,
        outcome: TurnOutcome,
        reason: TurnFinishReason,
    ) -> TurnRunResult {
        TurnRunResult {
            outcome,
            messages: self.messages,
            events: self.events,
            summary: TurnSummary {
                finish_reason: reason,
                wall_duration: self.turn_started_at.elapsed(),
                step_count: self.step_index + 1,
                total_tokens_used: self.token_tracker.budget_tokens(0) as u64,
                cache_read_input_tokens: self.total_cache_read_tokens,
                cache_creation_input_tokens: self.total_cache_creation_tokens,
                auto_compaction_count: 0,
                reactive_compact_count: self.reactive_compact_attempts,
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
        session_id,
        working_dir,
        turn_id,
        messages,
        session_state,
        runtime,
        cancel,
        agent,
        prompt_facts_provider,
    } = request;
    let gateway = kernel.gateway();
    let resources = TurnExecutionResources::new(
        gateway,
        TurnExecutionRequestView {
            prompt_facts_provider: prompt_facts_provider.as_ref(),
            session_id: &session_id,
            working_dir: &working_dir,
            turn_id: &turn_id,
            session_state: &session_state,
            runtime: &runtime,
            cancel: &cancel,
            agent: &agent,
        },
    );
    let mut execution = TurnExecutionContext::new(&resources, messages);

    loop {
        if resources.cancel.is_cancelled() {
            return Ok(execution.finish(
                &resources,
                TurnOutcome::Cancelled,
                TurnFinishReason::Cancelled,
            ));
        }

        if execution.step_index >= resources.max_steps {
            return Ok(execution.finish(
                &resources,
                TurnOutcome::Error {
                    message: format!("turn exceeded maximum steps ({})", resources.max_steps),
                },
                TurnFinishReason::StepLimitExceeded,
            ));
        }

        match run_single_step(&mut execution, &resources).await? {
            StepOutcome::Continue => {},
            StepOutcome::Completed => {
                return Ok(execution.finish(
                    &resources,
                    TurnOutcome::Completed,
                    TurnFinishReason::NaturalEnd,
                ));
            },
            StepOutcome::Cancelled => {
                return Ok(execution.finish(
                    &resources,
                    TurnOutcome::Cancelled,
                    TurnFinishReason::Cancelled,
                ));
            },
        }
    }
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
