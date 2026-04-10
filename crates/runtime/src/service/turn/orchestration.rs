use std::sync::{Arc, Mutex as StdMutex};

use anyhow::Result;
use astrcode_core::{
    AgentEventContext, AstrError, CancelToken, EventTranslator, ExecutionOwner, Phase,
    PromptMetricsPayload, StorageEvent, StorageEventPayload, UserMessageOrigin,
};
use astrcode_runtime_agent_control::AgentControl;
use astrcode_runtime_agent_loop::{
    AgentLoop, CompactionTailSnapshot, TokenBudgetDecision, TurnOutcome, build_auto_continue_nudge,
    check_token_budget, estimate_text_tokens,
};
use astrcode_runtime_prompt::PromptDeclaration;
use astrcode_runtime_session::{
    SessionState, append_and_broadcast, append_and_broadcast_from_turn_callback,
    complete_session_execution as complete_session_execution_state, recent_turn_event_tail,
};
use chrono::Utc;

use super::BudgetSettings;
use crate::service::observability::RuntimeObservability;

#[derive(Debug, Default, Clone, Copy)]
struct TurnExecutionStats {
    estimated_tokens_used: u64,
    last_assistant_output_tokens: usize,
    pending_prompt_tokens: Option<u64>,
}

impl TurnExecutionStats {
    fn record_prompt_metrics(&mut self, estimated_tokens: u32) {
        self.pending_prompt_tokens = Some(estimated_tokens as u64);
    }

    fn record_assistant_output(&mut self, content: &str, reasoning_content: Option<&str>) {
        self.flush_pending_prompt_tokens();
        let output_tokens = estimate_text_tokens(content)
            + reasoning_content
                .map(estimate_text_tokens)
                .unwrap_or_default();
        self.estimated_tokens_used = self
            .estimated_tokens_used
            .saturating_add(output_tokens as u64);
        self.last_assistant_output_tokens = output_tokens;
    }

    fn flush_pending_prompt_tokens(&mut self) {
        if let Some(prompt_tokens) = self.pending_prompt_tokens.take() {
            self.estimated_tokens_used = self.estimated_tokens_used.saturating_add(prompt_tokens);
        }
    }
}

pub struct SessionTurnRunResult {
    pub outcome: std::result::Result<TurnOutcome, AstrError>,
    pub phase: Phase,
    pub succeeded: bool,
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeTurnInput {
    pub user_event: Option<StorageEvent>,
    pub prompt_declarations: Vec<PromptDeclaration>,
}

impl RuntimeTurnInput {
    pub fn from_user_event(user_event: StorageEvent) -> Self {
        Self {
            user_event: Some(user_event),
            prompt_declarations: Vec::new(),
        }
    }
}

pub async fn complete_session_execution(
    session: &SessionState,
    phase: Phase,
    agent_control: &AgentControl,
) {
    // 在清理 session 状态之前读取 active_turn_id，
    // 否则 complete_session_execution_state 会将其清除为 None。
    let active_turn_id = session
        .active_turn_id
        .lock()
        .ok()
        .and_then(|guard| guard.clone());

    // 后台子执行的生命周期由 agent_control / subrun API 单独管理。
    // 父 turn 正常结束时只清理 session 自己的活跃状态，不取消后台 subrun，
    // 否则 shared-session 子执行会在父回复刚结束时被错误中断。
    complete_session_execution_state(session, phase);

    // 仅当父 turn 被中断时，取消该 turn 下所有仍在运行的子 agent，
    // 防止前台（SharedSession）子 agent 变成孤儿。
    if matches!(phase, Phase::Interrupted) {
        if let Some(turn_id) = active_turn_id.as_deref() {
            // 故意忽略：清理子运行失败不应阻断 turn 完成流程
            let cancelled = agent_control.cancel_for_parent_turn(turn_id).await;
            if !cancelled.is_empty() {
                log::info!(
                    "cancelled {} sub-agents for interrupted parent turn '{}'",
                    cancelled.len(),
                    turn_id
                );
            }
        }
    }
}

// 这里的参数和运行时回调链一一对应，先保留显式签名以避免把调用点语义埋进匿名元组。
#[allow(clippy::too_many_arguments)]
pub async fn run_session_turn(
    session: &SessionState,
    loop_: &AgentLoop,
    turn_id: &str,
    cancel: CancelToken,
    runtime_input: RuntimeTurnInput,
    agent: AgentEventContext,
    execution_owner: ExecutionOwner,
    budget_settings: BudgetSettings,
    observability: Option<Arc<RuntimeObservability>>,
) -> SessionTurnRunResult {
    let initial_phase = session.current_phase().unwrap_or(Phase::Idle);
    let mut translator = EventTranslator::new(initial_phase);
    let outcome = match runtime_input.user_event.as_ref() {
        Some(user_event) => {
            match append_and_broadcast_from_turn_callback(session, user_event, &mut translator) {
                Ok(_) => {
                    execute_turn_chain(
                        session,
                        loop_,
                        turn_id,
                        cancel,
                        &mut translator,
                        agent.clone(),
                        execution_owner,
                        budget_settings,
                        observability.clone(),
                        runtime_input.prompt_declarations,
                    )
                    .await
                },
                Err(error) => Err(AstrError::Internal(error.to_string())),
            }
        },
        None => {
            execute_turn_chain(
                session,
                loop_,
                turn_id,
                cancel,
                &mut translator,
                agent.clone(),
                execution_owner,
                budget_settings,
                observability.clone(),
                runtime_input.prompt_declarations,
            )
            .await
        },
    };
    let succeeded = matches!(
        outcome.as_ref(),
        Ok(TurnOutcome::Completed) | Ok(TurnOutcome::Cancelled)
    );
    if let Err(error) = &outcome {
        let error_event = StorageEvent {
            turn_id: Some(turn_id.to_string()),
            agent: agent.clone(),
            payload: StorageEventPayload::Error {
                message: error.to_string(),
                timestamp: Some(Utc::now()),
            },
        };
        // 故意忽略：广播错误事件失败时已在处理更重要的错误
        let _ = append_and_broadcast_from_turn_callback(session, &error_event, &mut translator);
        let turn_done = StorageEvent {
            turn_id: Some(turn_id.to_string()),
            agent,
            payload: StorageEventPayload::TurnDone {
                timestamp: Utc::now(),
                reason: Some("error".to_string()),
            },
        };
        // 故意忽略：广播 TurnDone 失败时已在处理更重要的错误
        let _ = append_and_broadcast_from_turn_callback(session, &turn_done, &mut translator);
    }

    SessionTurnRunResult {
        outcome,
        phase: translator.phase(),
        succeeded,
    }
}

// 这里继续保持显式参数列表，方便 runtime façade 与测试共享同一条 turn 链执行路径。
#[allow(clippy::too_many_arguments)]
pub async fn execute_turn_chain(
    state: &SessionState,
    loop_: &AgentLoop,
    turn_id: &str,
    cancel: CancelToken,
    translator: &mut EventTranslator,
    agent: AgentEventContext,
    execution_owner: ExecutionOwner,
    budget_settings: BudgetSettings,
    observability: Option<Arc<RuntimeObservability>>,
    runtime_prompt_declarations: Vec<PromptDeclaration>,
) -> std::result::Result<TurnOutcome, AstrError> {
    loop {
        let projected = state
            .snapshot_projected_state()
            .map_err(|error| AstrError::Internal(error.to_string()))?;
        let tail_seed = recent_turn_event_tail(
            &state
                .snapshot_recent_stored_events()
                .map_err(|error| AstrError::Internal(error.to_string()))?,
            loop_.compact_keep_recent_turns(),
        );
        let live_tail = Arc::new(StdMutex::new(Vec::new()));
        let mut stats = TurnExecutionStats::default();
        let outcome = loop_
            .run_turn_without_finish_with_compaction_tail_and_prompt_declarations(
                &projected,
                turn_id,
                &mut |event| {
                    observe_turn_event(&mut stats, &event);
                    observe_runtime_prompt_metrics(observability.as_deref(), &event);
                    let stored = append_and_broadcast_from_turn_callback(state, &event, translator)
                        .map_err(|error| AstrError::Internal(error.to_string()))?;
                    if astrcode_runtime_session::should_record_compaction_tail_event(&event) {
                        let mut tail = live_tail
                            .lock()
                            .expect("compaction live tail lock should not be poisoned");
                        tail.push(stored);
                    }
                    Ok(())
                },
                cancel.clone(),
                agent.clone(),
                execution_owner.clone(),
                CompactionTailSnapshot::from_seed(tail_seed)
                    .with_live_recorder(Arc::clone(&live_tail)),
                &runtime_prompt_declarations,
            )
            .await?;

        if matches!(outcome, TurnOutcome::Completed)
            && maybe_continue_after_turn(
                state,
                turn_id,
                translator,
                agent.clone(),
                stats,
                budget_settings,
            )
            .await
            .map_err(|error| AstrError::Internal(error.to_string()))?
        {
            continue;
        }

        append_and_broadcast(
            state,
            &StorageEvent {
                turn_id: Some(turn_id.to_string()),
                agent: agent.clone(),
                payload: StorageEventPayload::TurnDone {
                    timestamp: Utc::now(),
                    reason: Some(turn_done_reason(&outcome).to_string()),
                },
            },
            translator,
        )
        .await
        .map_err(|error| AstrError::Internal(error.to_string()))?;
        return Ok(outcome);
    }
}

fn observe_runtime_prompt_metrics(
    observability: Option<&RuntimeObservability>,
    event: &StorageEvent,
) {
    let Some(observability) = observability else {
        return;
    };

    let StorageEvent {
        payload:
            StorageEventPayload::PromptMetrics {
                metrics:
                    PromptMetricsPayload {
                        provider_input_tokens: None,
                        prompt_cache_reuse_hits,
                        prompt_cache_reuse_misses,
                        ..
                    },
            },
        ..
    } = event
    else {
        return;
    };

    if *prompt_cache_reuse_hits > 0 {
        observability.record_cache_reuse_hits(u64::from(*prompt_cache_reuse_hits));
    }
    if *prompt_cache_reuse_misses > 0 {
        observability.record_cache_reuse_misses(u64::from(*prompt_cache_reuse_misses));
    }
}

async fn maybe_continue_after_turn(
    state: &SessionState,
    turn_id: &str,
    translator: &mut EventTranslator,
    agent: AgentEventContext,
    stats: TurnExecutionStats,
    budget_settings: BudgetSettings,
) -> Result<bool> {
    let (decision, total_budget, used_tokens) = {
        let mut budget_guard = state
            .token_budget
            .lock()
            .expect("session token budget lock should not be poisoned");
        let Some(budget_state) = budget_guard.as_mut() else {
            return Ok(false);
        };

        budget_state.used_tokens = budget_state
            .used_tokens
            .saturating_add(stats.estimated_tokens_used);
        let decision = check_token_budget(
            budget_state.used_tokens,
            budget_state.total_budget,
            budget_state.continuation_count,
            stats.last_assistant_output_tokens,
            budget_settings.continuation_min_delta_tokens,
            budget_settings.max_continuations,
        );
        let total_budget = budget_state.total_budget;
        let used_tokens = budget_state.used_tokens;
        if matches!(decision, TokenBudgetDecision::Continue) {
            budget_state.continuation_count = budget_state.continuation_count.saturating_add(1);
        } else {
            *budget_guard = None;
        }
        (decision, total_budget, used_tokens)
    };

    if !matches!(decision, TokenBudgetDecision::Continue) {
        return Ok(false);
    }

    append_and_broadcast(
        state,
        &StorageEvent {
            turn_id: Some(turn_id.to_string()),
            agent,
            payload: StorageEventPayload::UserMessage {
                content: build_auto_continue_nudge(used_tokens, total_budget),
                timestamp: Utc::now(),
                origin: UserMessageOrigin::AutoContinueNudge,
            },
        },
        translator,
    )
    .await?;
    Ok(true)
}

fn observe_turn_event(stats: &mut TurnExecutionStats, event: &StorageEvent) {
    match &event.payload {
        StorageEventPayload::PromptMetrics {
            metrics:
                PromptMetricsPayload {
                    estimated_tokens,
                    provider_input_tokens: None,
                    ..
                },
            ..
        } => {
            stats.record_prompt_metrics(*estimated_tokens);
        },
        StorageEventPayload::AssistantFinal {
            content,
            reasoning_content,
            ..
        } => {
            stats.record_assistant_output(content, reasoning_content.as_deref());
        },
        _ => {},
    }
}

fn turn_done_reason(outcome: &TurnOutcome) -> &'static str {
    match outcome {
        TurnOutcome::Completed => "completed",
        TurnOutcome::Cancelled => "cancelled",
        TurnOutcome::Error { .. } => "error",
    }
}
