//! # Turn 执行操作 (Turn Execution Operations)
//!
//! 实现 `RuntimeService` 的 Turn 生命周期管理，包括：
//! - 提交用户 Prompt 并启动异步 Turn 执行
//! - 会话分支（当目标会话正忙时自动创建分支）
//! - Turn 链执行（支持自动续跑 Token 预算未耗尽的情况）
//! - 中断、手动压缩会话
//!
//! ## Turn 提交流程
//!
//! 1. 解析 Token 预算标记（如 `@budget:4000`）
//! 2. 解析提交目标（若会话正忙则自动分支）
//! 3. 获取 Turn Lease 和 CancelToken
//! 4. 在后台任务中执行 Turn 链
//! 5. Turn 完成后重置状态并广播结果
//!
//! ## 自动分支机制
//!
//! 当用户向正在运行的会话提交新 Prompt 时，系统会自动创建分支会话，
//! 继承父会话的稳定历史（排除当前活跃 Turn 的未完成事件）。
//! 分支深度限制为 3 层，防止过深的分支树影响性能。
//!
//! ## Token 预算与自动续跑
//!
//! 若 Turn 完成后 Token 预算未耗尽且满足最小增量要求，
//! 系统会自动注入 `AutoContinueNudge` 消息触发下一轮 Turn，
//! 直到预算耗尽或达到最大续跑次数。

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use astrcode_core::{
    generate_session_id, AstrError, CancelToken, Phase, SessionTurnAcquireResult, SessionTurnLease,
    StorageEvent, StoredEvent, UserMessageOrigin,
};
use chrono::Utc;
use uuid::Uuid;

use super::session_ops::normalize_session_id;
use super::session_state::{SessionState, SessionTokenBudgetState};
use super::support::{lock_anyhow, spawn_blocking_service};
use super::{PromptAccepted, RuntimeService, ServiceError, ServiceResult, SessionCatalogEvent};
use crate::agent_loop::{
    token_budget::{
        build_auto_continue_nudge, check_token_budget, strip_token_budget_marker,
        TokenBudgetDecision,
    },
    TurnOutcome,
};
use crate::config::{
    resolve_continuation_min_delta_tokens, resolve_default_token_budget, resolve_max_continuations,
};
use crate::context_window::{auto_compact, estimate_text_tokens, CompactConfig};
use astrcode_core::EventTranslator;

/// Turn 提交的目标会话及其执行上下文。
///
/// 包含会话状态、Turn Lease（用于并发控制）以及可能的分支来源信息。
struct SubmitTarget {
    /// 实际执行 Turn 的会话 ID（可能是分支后的新会话）
    session_id: String,
    /// 若发生了分支，记录父会话 ID
    branched_from_session_id: Option<String>,
    /// 会话运行时状态
    session: Arc<SessionState>,
    /// Turn 独占锁，保证同一时刻只有一个 Turn 在执行
    turn_lease: Box<dyn SessionTurnLease>,
}

/// 最大允许的并发分支深度。
///
/// 超过此深度说明存在过多的并发提交，应拒绝以避免分支树膨胀。
const MAX_CONCURRENT_BRANCH_DEPTH: usize = 3;

/// Token 预算相关的配置参数。
#[derive(Debug, Clone, Copy)]
struct BudgetSettings {
    /// 触发自动续跑所需的最小剩余 Token 数
    continuation_min_delta_tokens: usize,
    /// 单个 Turn 允许的最大自动续跑次数
    max_continuations: u8,
}

/// Turn 执行过程中的统计信息。
///
/// 用于跟踪 Token 消耗，支持预算检查和自动续跑决策。
#[derive(Debug, Default, Clone, Copy)]
struct TurnExecutionStats {
    /// 累计估算的 Token 使用量（含 prompt + assistant output）
    estimated_tokens_used: u64,
    /// 最近一次助手输出的 Token 数
    last_assistant_output_tokens: usize,
    /// 待计入的 prompt Token 数（仅在模型真正响应后才计费）
    pending_prompt_tokens: Option<u64>,
}

impl TurnExecutionStats {
    /// 记录 prompt 的 Token 指标，但暂不计入总消耗。
    ///
    /// 这样设计是为了避免纯压缩（compaction-only）快照消耗续跑预算——
    /// 只有当模型实际产生响应后，才通过 `flush_pending_prompt_tokens` 计费。
    fn record_prompt_metrics(&mut self, estimated_tokens: u32) {
        self.pending_prompt_tokens = Some(estimated_tokens as u64);
    }

    /// 记录助手输出内容的 Token 数，并在此时刷入待计的 prompt Token。
    fn record_assistant_output(&mut self, content: &str, reasoning_content: Option<&str>) {
        // Only charge the prompt snapshot once the model actually produced a response, so
        // compaction-only snapshots do not consume the session's continuation budget.
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

    /// 将待计的 prompt Token 刷入总消耗。
    fn flush_pending_prompt_tokens(&mut self) {
        if let Some(prompt_tokens) = self.pending_prompt_tokens.take() {
            self.estimated_tokens_used = self.estimated_tokens_used.saturating_add(prompt_tokens);
        }
    }
}

impl RuntimeService {
    pub async fn submit_prompt(
        &self,
        session_id: &str,
        text: String,
    ) -> ServiceResult<PromptAccepted> {
        let runtime_config = { self.config.lock().await.runtime.clone() };
        let parsed_budget = strip_token_budget_marker(&text);
        let default_token_budget = resolve_default_token_budget(&runtime_config);
        let token_budget = parsed_budget
            .budget
            .or((default_token_budget > 0).then_some(default_token_budget));
        let text = if parsed_budget.cleaned_text.is_empty() {
            text
        } else {
            parsed_budget.cleaned_text
        };
        let budget_settings = BudgetSettings {
            continuation_min_delta_tokens: resolve_continuation_min_delta_tokens(&runtime_config),
            max_continuations: resolve_max_continuations(&runtime_config),
        };
        let turn_id = Uuid::new_v4().to_string();
        let session_id = normalize_session_id(session_id);
        let SubmitTarget {
            session_id,
            branched_from_session_id,
            session,
            turn_lease,
        } = self.resolve_submit_target(&session_id, &turn_id).await?;
        let cancel = CancelToken::new();
        {
            let mut cancel_guard = lock_anyhow(&session.cancel, "session cancel")?;
            let mut lease_guard = lock_anyhow(&session.turn_lease, "session turn lease")?;
            if session.running.swap(true, Ordering::SeqCst) {
                return Err(ServiceError::Conflict(format!(
                    "session '{}' entered an inconsistent running state",
                    session_id
                )));
            }
            *cancel_guard = cancel.clone();
            *lease_guard = Some(turn_lease);
        }
        if let Ok(mut budget_guard) = lock_anyhow(&session.token_budget, "session token budget") {
            *budget_guard = token_budget.map(|total_budget| SessionTokenBudgetState {
                total_budget,
                used_tokens: 0,
                continuation_count: 0,
            });
        }

        let state = session.clone();
        let loop_ = self.current_loop().await;
        let text_for_task = text;

        let accepted_turn_id = turn_id.clone();
        let observability = self.observability.clone();
        let accepted_session_id = session_id.clone();
        tokio::spawn(async move {
            let turn_started_at = Instant::now();
            let initial_phase = lock_anyhow(&state.phase, "session phase")
                .map(|guard| *guard)
                .unwrap_or(Phase::Idle);
            let mut translator = EventTranslator::new(initial_phase);

            let user_event = StorageEvent::UserMessage {
                turn_id: Some(turn_id.clone()),
                content: text_for_task,
                timestamp: Utc::now(),
                origin: UserMessageOrigin::User,
            };

            let task_result = match append_and_broadcast(&state, &user_event, &mut translator).await
            {
                Ok(()) => {
                    execute_turn_chain(
                        &state,
                        &loop_,
                        &turn_id,
                        cancel.clone(),
                        &mut translator,
                        budget_settings,
                    )
                    .await
                }
                Err(error) => Err(AstrError::Internal(error.to_string())),
            };
            let result = task_result;

            let succeeded = matches!(
                result.as_ref(),
                Ok(TurnOutcome::Completed) | Ok(TurnOutcome::Cancelled)
            );
            if let Err(error) = &result {
                // 错误时同时发送 Error 和 TurnDone 事件。TurnDone 必须发送，
                // 即使 turn 失败了——SSE 客户端依赖 TurnDone 来检测 turn 结束，
                // 若不发送客户端会永远等待。let _ = 忽略 broadcast 结果是因为
                // 无活跃订阅者时 send 返回 Err，这是良性情况。
                let error_event = StorageEvent::Error {
                    turn_id: Some(turn_id.clone()),
                    message: error.to_string(),
                    timestamp: Some(Utc::now()),
                };
                let _ = append_and_broadcast(&state, &error_event, &mut translator).await;
                let turn_done = StorageEvent::TurnDone {
                    turn_id: Some(turn_id.clone()),
                    timestamp: Utc::now(),
                    reason: Some("error".to_string()),
                };
                let _ = append_and_broadcast(&state, &turn_done, &mut translator).await;
            }

            if let Ok(mut phase) = lock_anyhow(&state.phase, "session phase") {
                *phase = translator.phase();
            }
            // 重置 CancelToken：前一个 token 已被消费（正常完成或被取消），
            // 必须替换为新的空 token 以"重新武装"会话，否则下一次 interrupt()
            // 调用会触发一个已过期的 token，无法取消新的 turn。
            if let Ok(mut guard) = lock_anyhow(&state.cancel, "session cancel") {
                *guard = CancelToken::new();
            }
            if let Ok(mut lease) = lock_anyhow(&state.turn_lease, "session turn lease") {
                *lease = None;
            }
            if let Ok(mut budget_guard) = lock_anyhow(&state.token_budget, "session token budget") {
                *budget_guard = None;
            }
            state.running.store(false, Ordering::SeqCst);

            let elapsed = turn_started_at.elapsed();
            observability.record_turn_execution(elapsed, succeeded);
            match &result {
                Ok(TurnOutcome::Completed) => {
                    if elapsed.as_millis() >= 5_000 {
                        log::warn!(
                            "turn '{}' completed slowly in {}ms",
                            turn_id,
                            elapsed.as_millis()
                        );
                    } else {
                        log::info!("turn '{}' completed in {}ms", turn_id, elapsed.as_millis());
                    }
                }
                Ok(TurnOutcome::Cancelled) => {
                    log::info!("turn '{}' cancelled in {}ms", turn_id, elapsed.as_millis());
                }
                Ok(TurnOutcome::Error { message }) => {
                    log::warn!(
                        "turn '{}' ended with agent error in {}ms: {}",
                        turn_id,
                        elapsed.as_millis(),
                        message
                    );
                }
                Err(_) => {
                    log::warn!("turn '{}' failed in {}ms", turn_id, elapsed.as_millis());
                }
            }
        });

        Ok(PromptAccepted {
            turn_id: accepted_turn_id,
            session_id: accepted_session_id,
            branched_from_session_id,
        })
    }

    pub async fn interrupt(&self, session_id: &str) -> ServiceResult<()> {
        let session_id = normalize_session_id(session_id);
        if let Some(session) = self.sessions.get(&session_id) {
            if let Ok(cancel) = lock_anyhow(&session.cancel, "session cancel") {
                cancel.cancel();
            }
        }
        Ok(())
    }

    pub async fn compact_session(&self, session_id: &str) -> ServiceResult<()> {
        let session_id = normalize_session_id(session_id);
        let session = self.ensure_session_loaded(&session_id).await?;
        if session.running.load(Ordering::SeqCst) {
            return Err(ServiceError::Conflict(format!(
                "session '{}' is busy; manual compact is only allowed while idle",
                session_id
            )));
        }

        let loop_ = self.current_loop().await;
        let projected = session
            .snapshot_projected_state()
            .map_err(|error| ServiceError::Internal(AstrError::Internal(error.to_string())))?;
        let provider = loop_
            .build_provider(Some(projected.working_dir.clone()))
            .await
            .map_err(ServiceError::from)?;
        let compact_result = auto_compact(
            provider.as_ref(),
            &projected.messages,
            None,
            CompactConfig {
                keep_recent_turns: loop_.compact_keep_recent_turns(),
                trigger: astrcode_core::CompactTrigger::Manual,
            },
            CancelToken::new(),
        )
        .await
        .map_err(ServiceError::from)?;

        let Some(compact_result) = compact_result else {
            if let Ok(mut failures) =
                lock_anyhow(&session.compact_failure_count, "compact failures")
            {
                *failures = 0;
            }
            return Ok(());
        };

        let initial_phase = lock_anyhow(&session.phase, "session phase")
            .map(|guard| *guard)
            .unwrap_or(Phase::Idle);
        let mut translator = EventTranslator::new(initial_phase);
        append_and_broadcast(
            &session,
            &StorageEvent::CompactApplied {
                turn_id: None,
                trigger: astrcode_core::CompactTrigger::Manual,
                summary: compact_result.summary,
                preserved_recent_turns: compact_result.preserved_recent_turns.min(u32::MAX as usize)
                    as u32,
                pre_tokens: compact_result.pre_tokens.min(u32::MAX as usize) as u32,
                post_tokens_estimate: compact_result.post_tokens_estimate.min(u32::MAX as usize)
                    as u32,
                messages_removed: compact_result.messages_removed.min(u32::MAX as usize) as u32,
                tokens_freed: compact_result.tokens_freed.min(u32::MAX as usize) as u32,
                timestamp: compact_result.timestamp,
            },
            &mut translator,
        )
        .await
        .map_err(|error| ServiceError::Internal(AstrError::Internal(error.to_string())))?;
        if let Ok(mut phase) = lock_anyhow(&session.phase, "session phase") {
            *phase = translator.phase();
        }
        if let Ok(mut failures) = lock_anyhow(&session.compact_failure_count, "compact failures") {
            *failures = 0;
        }
        Ok(())
    }
}

async fn execute_turn_chain(
    state: &SessionState,
    loop_: &crate::agent_loop::AgentLoop,
    turn_id: &str,
    cancel: CancelToken,
    translator: &mut EventTranslator,
    budget_settings: BudgetSettings,
) -> std::result::Result<TurnOutcome, AstrError> {
    loop {
        let projected = state
            .snapshot_projected_state()
            .map_err(|error| AstrError::Internal(error.to_string()))?;
        let mut stats = TurnExecutionStats::default();
        let outcome = loop_
            .run_turn_without_finish(
                &projected,
                turn_id,
                &mut |event| {
                    observe_turn_event(&mut stats, &event);
                    append_and_broadcast_from_turn_callback(state, &event, translator)
                        .map_err(|error| AstrError::Internal(error.to_string()))
                },
                cancel.clone(),
            )
            .await?;

        if matches!(outcome, TurnOutcome::Completed)
            && maybe_continue_after_turn(state, turn_id, translator, stats, budget_settings)
                .await
                .map_err(|error| AstrError::Internal(error.to_string()))?
        {
            continue;
        }

        append_and_broadcast(
            state,
            &StorageEvent::TurnDone {
                turn_id: Some(turn_id.to_string()),
                timestamp: Utc::now(),
                reason: Some(turn_done_reason(&outcome).to_string()),
            },
            translator,
        )
        .await
        .map_err(|error| AstrError::Internal(error.to_string()))?;
        return Ok(outcome);
    }
}

async fn maybe_continue_after_turn(
    state: &SessionState,
    turn_id: &str,
    translator: &mut EventTranslator,
    stats: TurnExecutionStats,
    budget_settings: BudgetSettings,
) -> ServiceResult<bool> {
    let (decision, total_budget, used_tokens) = {
        let mut budget_guard = lock_anyhow(&state.token_budget, "session token budget")?;
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

    // TODO(claude-auto-compact): if Astrcode grows a queue-based turn scheduler, move auto-
    // continue dispatch there instead of appending the synthetic nudge directly from this task.
    append_and_broadcast(
        state,
        &StorageEvent::UserMessage {
            turn_id: Some(turn_id.to_string()),
            content: build_auto_continue_nudge(used_tokens, total_budget),
            timestamp: Utc::now(),
            origin: UserMessageOrigin::AutoContinueNudge,
        },
        translator,
    )
    .await?;
    Ok(true)
}

fn observe_turn_event(stats: &mut TurnExecutionStats, event: &StorageEvent) {
    match event {
        StorageEvent::PromptMetrics {
            estimated_tokens, ..
        } => {
            stats.record_prompt_metrics(*estimated_tokens);
        }
        StorageEvent::AssistantFinal {
            content,
            reasoning_content,
            ..
        } => {
            stats.record_assistant_output(content, reasoning_content.as_deref());
        }
        _ => {}
    }
}

fn turn_done_reason(outcome: &TurnOutcome) -> &'static str {
    match outcome {
        TurnOutcome::Completed => "completed",
        TurnOutcome::Cancelled => "cancelled",
        TurnOutcome::Error { .. } => "error",
    }
}

impl RuntimeService {
    async fn resolve_submit_target(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> ServiceResult<SubmitTarget> {
        let mut target_session_id = session_id.to_string();
        let mut branched_from_session_id = None;
        let mut branch_depth = 0usize;

        loop {
            let session = self.ensure_session_loaded(&target_session_id).await?;
            let session_manager = Arc::clone(&self.session_manager);
            let acquire_session_id = target_session_id.clone();
            let acquire_turn_id = turn_id.to_string();
            let acquire_result = spawn_blocking_service("acquire session turn lease", move || {
                session_manager
                    .try_acquire_turn(&acquire_session_id, &acquire_turn_id)
                    .map_err(ServiceError::from)
            })
            .await?;

            match acquire_result {
                SessionTurnAcquireResult::Acquired(turn_lease) => {
                    return Ok(SubmitTarget {
                        session_id: target_session_id,
                        branched_from_session_id,
                        session,
                        turn_lease,
                    });
                }
                SessionTurnAcquireResult::Busy(active_turn) => {
                    ensure_branch_depth_within_limit(branch_depth)?;
                    let source_session_id = target_session_id.clone();
                    target_session_id = self
                        .branch_session_from_busy_turn(&source_session_id, &active_turn.turn_id)
                        .await?;
                    self.emit_session_catalog_event(SessionCatalogEvent::SessionBranched {
                        session_id: target_session_id.clone(),
                        source_session_id: source_session_id.clone(),
                    });
                    branched_from_session_id = Some(source_session_id);
                    branch_depth += 1;
                }
            }
        }
    }

    async fn branch_session_from_busy_turn(
        &self,
        source_session_id: &str,
        active_turn_id: &str,
    ) -> ServiceResult<String> {
        let session_manager = Arc::clone(&self.session_manager);
        let source_session_id = source_session_id.to_string();
        let active_turn_id = active_turn_id.to_string();
        spawn_blocking_service("branch running session", move || {
            let source_events = session_manager
                .replay_events(&source_session_id)
                .map_err(ServiceError::from)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(ServiceError::from)?;
            let Some(first_event) = source_events.first() else {
                return Err(ServiceError::NotFound(format!(
                    "session '{}' is empty",
                    source_session_id
                )));
            };
            let working_dir = match &first_event.event {
                StorageEvent::SessionStart { working_dir, .. } => {
                    std::path::PathBuf::from(working_dir)
                }
                _ => {
                    return Err(ServiceError::Internal(AstrError::Internal(format!(
                        "session '{}' is missing sessionStart",
                        source_session_id
                    ))))
                }
            };

            let stable_events = stable_events_before_active_turn(&source_events, &active_turn_id);
            let parent_storage_seq = stable_events.last().map(|event| event.storage_seq);
            let branched_session_id = generate_session_id();
            let mut log = session_manager
                .create_event_log(&branched_session_id, &working_dir)
                .map_err(ServiceError::from)?;
            log.append(&StorageEvent::SessionStart {
                session_id: branched_session_id.clone(),
                timestamp: Utc::now(),
                working_dir: working_dir.to_string_lossy().to_string(),
                parent_session_id: Some(source_session_id.clone()),
                parent_storage_seq,
            })
            .map_err(ServiceError::from)?;

            // 分叉只复制稳定完成的历史；当前活跃 turn 的任何事件都必须排除，
            // 否则新分支会带着半截工具调用或流式输出继续运行，语义会变脏。
            for stored in stable_events {
                if matches!(stored.event, StorageEvent::SessionStart { .. }) {
                    continue;
                }
                log.append(&stored.event).map_err(ServiceError::from)?;
            }

            Ok(branched_session_id)
        })
        .await
    }
}

fn stable_events_before_active_turn(
    events: &[StoredEvent],
    active_turn_id: &str,
) -> Vec<StoredEvent> {
    let cutoff = events
        .iter()
        .position(|stored| stored.event.turn_id() == Some(active_turn_id))
        .unwrap_or(events.len());
    events[..cutoff].to_vec()
}

/// 异步版本：通过 spawn_blocking 将文件 I/O 委托给阻塞线程池。
/// 用于 turn 开始前的初始 UserMessage 追加，不在热路径上。
async fn append_and_broadcast(
    session: &SessionState,
    event: &StorageEvent,
    translator: &mut EventTranslator,
) -> Result<()> {
    let stored = session.writer.clone().append(event.clone()).await?;
    let records = session.translate_store_and_cache(&stored, translator)?;
    for record in records {
        let _ = session.broadcaster.send(record);
    }
    Ok(())
}

/// 同步版本：直接在当前线程执行文件 I/O。
/// 用于 run_turn 的 on_event 回调内部——回调已在 async 上下文中执行，
/// 每个事件额外 spawn_blocking 一次的开销不可接受（可能每秒数十次）。
fn append_and_broadcast_blocking(
    session: &SessionState,
    event: &StorageEvent,
    translator: &mut EventTranslator,
) -> Result<()> {
    let stored = session.writer.append_blocking(event)?;
    let records = session.translate_store_and_cache(&stored, translator)?;
    for record in records {
        let _ = session.broadcaster.send(record);
    }
    Ok(())
}

fn append_and_broadcast_from_turn_callback(
    session: &SessionState,
    event: &StorageEvent,
    translator: &mut EventTranslator,
) -> Result<()> {
    match tokio::runtime::Handle::current().runtime_flavor() {
        tokio::runtime::RuntimeFlavor::CurrentThread => {
            append_and_broadcast_blocking(session, event, translator)
        }
        _ => tokio::task::block_in_place(|| {
            // 只有 current-thread runtime 明确不支持 block_in_place。其余 flavor
            // 默认按“可让出 worker”的路径处理，避免未来 Tokio 扩展 flavor 时
            // 静默退回到直接阻塞事件循环。
            tokio::runtime::Handle::current()
                .block_on(append_and_broadcast(session, event, translator))
        }),
    }
}

fn ensure_branch_depth_within_limit(branch_depth: usize) -> ServiceResult<()> {
    if branch_depth >= MAX_CONCURRENT_BRANCH_DEPTH {
        return Err(ServiceError::Conflict(format!(
            "too many concurrent branch attempts (limit: {})",
            MAX_CONCURRENT_BRANCH_DEPTH
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Arc;

    use astrcode_core::AgentEvent;
    use async_trait::async_trait;
    use chrono::Utc;

    use astrcode_storage::session::EventLog;
    use serde_json::json;

    use super::super::session_state::SessionWriter;
    use super::*;
    use crate::llm::{EventSink, LlmOutput, LlmProvider, LlmRequest, ModelLimits};
    use crate::provider_factory::ProviderFactory;
    use crate::test_support::TestEnvGuard;

    struct ScriptedProvider {
        responses: std::sync::Mutex<VecDeque<LlmOutput>>,
    }

    struct StaticProviderFactory {
        provider: Arc<dyn LlmProvider>,
    }

    impl ProviderFactory for StaticProviderFactory {
        fn build_for_working_dir(
            &self,
            _working_dir: Option<std::path::PathBuf>,
        ) -> astrcode_core::Result<Arc<dyn LlmProvider>> {
            Ok(Arc::clone(&self.provider))
        }
    }

    #[async_trait]
    impl LlmProvider for ScriptedProvider {
        fn model_limits(&self) -> ModelLimits {
            ModelLimits {
                context_window: 200_000,
                max_output_tokens: 4_096,
            }
        }

        async fn generate(
            &self,
            _request: LlmRequest,
            sink: Option<EventSink>,
        ) -> astrcode_core::Result<LlmOutput> {
            let output = self
                .responses
                .lock()
                .expect("responses lock")
                .pop_front()
                .expect("scripted response should be available");
            if let Some(sink) = sink {
                for token in output.content.chars() {
                    sink(crate::llm::LlmEvent::TextDelta(token.to_string()));
                }
            }
            Ok(output)
        }
    }

    fn build_test_state() -> (tempfile::TempDir, SessionState, EventTranslator) {
        let temp_dir = tempfile::tempdir().expect("tempdir should be created");
        let log =
            EventLog::create("test-session", temp_dir.path()).expect("event log should be created");
        let state = SessionState::new(
            temp_dir.path().to_path_buf(),
            Phase::Idle,
            // 测试继续走真实 EventLog，确保 trait object 包装不改变持久化路径。
            Arc::new(SessionWriter::new(Box::new(log))),
            Default::default(),
            Vec::new(),
        );
        (temp_dir, state, EventTranslator::new(Phase::Idle))
    }

    #[tokio::test(flavor = "current_thread")]
    async fn append_and_broadcast_blocking_works_on_current_thread_runtime() {
        let _guard = TestEnvGuard::new();
        let (temp_dir, state, mut translator) = build_test_state();
        let mut receiver = state.broadcaster.subscribe();

        append_and_broadcast_blocking(
            &state,
            &StorageEvent::SessionStart {
                session_id: "test-session".to_string(),
                timestamp: Utc::now(),
                working_dir: temp_dir.path().to_string_lossy().to_string(),
                parent_session_id: None,
                parent_storage_seq: None,
            },
            &mut translator,
        )
        .expect("append should succeed");

        let record = receiver.recv().await.expect("record should be broadcast");
        assert_eq!(record.event_id, "1.0");
        assert!(matches!(
            record.event,
            AgentEvent::SessionStarted { ref session_id } if session_id == "test-session"
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn append_and_broadcast_from_turn_callback_works_on_multi_thread_runtime() {
        let _guard = TestEnvGuard::new();
        let (temp_dir, state, mut translator) = build_test_state();
        let mut receiver = state.broadcaster.subscribe();

        append_and_broadcast_from_turn_callback(
            &state,
            &StorageEvent::SessionStart {
                session_id: "test-session".to_string(),
                timestamp: Utc::now(),
                working_dir: temp_dir.path().to_string_lossy().to_string(),
                parent_session_id: None,
                parent_storage_seq: None,
            },
            &mut translator,
        )
        .expect("append should succeed on multi-thread runtimes too");

        let record = receiver.recv().await.expect("record should be broadcast");
        assert_eq!(record.event_id, "1.0");
    }

    #[test]
    fn stable_events_before_active_turn_stops_at_the_active_turn_boundary() {
        let timestamp = Utc::now();
        let events = vec![
            StoredEvent {
                storage_seq: 1,
                event: StorageEvent::SessionStart {
                    session_id: "session-1".to_string(),
                    timestamp,
                    working_dir: "D:/workspace".to_string(),
                    parent_session_id: None,
                    parent_storage_seq: None,
                },
            },
            StoredEvent {
                storage_seq: 2,
                event: StorageEvent::UserMessage {
                    turn_id: Some("turn-1".to_string()),
                    content: "first".to_string(),
                    origin: UserMessageOrigin::User,
                    timestamp,
                },
            },
            StoredEvent {
                storage_seq: 3,
                event: StorageEvent::TurnDone {
                    turn_id: Some("turn-1".to_string()),
                    timestamp,
                    reason: Some("completed".to_string()),
                },
            },
            StoredEvent {
                storage_seq: 4,
                event: StorageEvent::UserMessage {
                    turn_id: Some("turn-2".to_string()),
                    content: "second".to_string(),
                    origin: UserMessageOrigin::User,
                    timestamp,
                },
            },
            StoredEvent {
                storage_seq: 5,
                event: StorageEvent::ToolCall {
                    turn_id: None,
                    tool_call_id: "call-1".to_string(),
                    tool_name: "echo".to_string(),
                    args: json!({"message": "legacy event without turn id"}),
                },
            },
        ];

        let stable = stable_events_before_active_turn(&events, "turn-2");
        let stable_seq = stable
            .iter()
            .map(|event| event.storage_seq)
            .collect::<Vec<_>>();

        assert_eq!(stable_seq, vec![1, 2, 3]);
    }

    #[test]
    fn branch_depth_guard_rejects_unbounded_branch_chains() {
        let error = ensure_branch_depth_within_limit(MAX_CONCURRENT_BRANCH_DEPTH)
            .expect_err("depth at the configured limit should be rejected");

        assert!(matches!(error, ServiceError::Conflict(_)));
        assert!(
            error
                .to_string()
                .contains("too many concurrent branch attempts"),
            "conflict reason should explain why submit was rejected"
        );
    }

    #[test]
    fn prompt_metrics_only_charge_budget_after_a_real_model_response() {
        let mut stats = TurnExecutionStats::default();

        observe_turn_event(
            &mut stats,
            &StorageEvent::PromptMetrics {
                turn_id: Some("turn-1".to_string()),
                step_index: 0,
                estimated_tokens: 800,
                context_window: 100_000,
                effective_window: 80_000,
                threshold_tokens: 72_000,
                truncated_tool_results: 0,
            },
        );
        assert_eq!(
            stats.estimated_tokens_used, 0,
            "compaction-only snapshots should not be billed yet"
        );

        observe_turn_event(
            &mut stats,
            &StorageEvent::AssistantFinal {
                turn_id: Some("turn-1".to_string()),
                content: "done".to_string(),
                reasoning_content: None,
                reasoning_signature: None,
                timestamp: None,
            },
        );

        assert!(
            stats.estimated_tokens_used >= 800,
            "the prompt charge should be applied once the model actually responded"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn execute_turn_chain_appends_a_single_auto_continue_nudge_before_stopping() {
        let _guard = TestEnvGuard::new();
        let (temp_dir, state, mut translator) = build_test_state();
        let provider: Arc<dyn LlmProvider> = Arc::new(ScriptedProvider {
            responses: std::sync::Mutex::new(VecDeque::from([
                LlmOutput {
                    content: "a".repeat(240),
                    ..LlmOutput::default()
                },
                LlmOutput {
                    content: "done".to_string(),
                    ..LlmOutput::default()
                },
            ])),
        });
        let loop_ = crate::agent_loop::AgentLoop::from_capabilities(
            Arc::new(StaticProviderFactory { provider }),
            crate::test_support::empty_capabilities(),
        );

        append_and_broadcast(
            &state,
            &StorageEvent::SessionStart {
                session_id: "test-session".to_string(),
                timestamp: Utc::now(),
                working_dir: temp_dir.path().to_string_lossy().to_string(),
                parent_session_id: None,
                parent_storage_seq: None,
            },
            &mut translator,
        )
        .await
        .expect("session start should persist");
        append_and_broadcast(
            &state,
            &StorageEvent::UserMessage {
                turn_id: Some("turn-auto".to_string()),
                content: "work ".repeat(200),
                origin: UserMessageOrigin::User,
                timestamp: Utc::now(),
            },
            &mut translator,
        )
        .await
        .expect("user message should persist");

        *lock_anyhow(&state.token_budget, "session token budget").expect("budget lock") =
            Some(SessionTokenBudgetState {
                total_budget: 1_000,
                used_tokens: 850,
                continuation_count: 0,
            });

        let outcome = execute_turn_chain(
            &state,
            &loop_,
            "turn-auto",
            CancelToken::new(),
            &mut translator,
            BudgetSettings {
                continuation_min_delta_tokens: 1,
                max_continuations: 1,
            },
        )
        .await
        .expect("turn chain should complete");

        assert!(matches!(outcome, TurnOutcome::Completed));

        let projected = state
            .snapshot_projected_state()
            .expect("projected state should be readable");
        assert_eq!(projected.messages.len(), 4);
        assert!(matches!(
            &projected.messages[0],
            astrcode_core::LlmMessage::User {
                origin: UserMessageOrigin::User,
                ..
            }
        ));
        assert!(matches!(
            &projected.messages[1],
            astrcode_core::LlmMessage::Assistant { content, .. } if content.len() == 240
        ));
        assert!(matches!(
            &projected.messages[2],
            astrcode_core::LlmMessage::User {
                content,
                origin: UserMessageOrigin::AutoContinueNudge,
            } if content.contains("Stopped at")
        ));
        assert!(matches!(
            &projected.messages[3],
            astrcode_core::LlmMessage::Assistant { content, .. } if content == "done"
        ));
        assert!(
            lock_anyhow(&state.token_budget, "session token budget")
                .expect("budget lock")
                .is_none(),
            "the budget state should be cleared once the chain stops continuing"
        );
    }
}
