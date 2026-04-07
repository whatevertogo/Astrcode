use std::{
    collections::VecDeque,
    sync::{Arc, Mutex as StdMutex, atomic::Ordering},
};

use anyhow::Result;
use astrcode_core::{
    AgentEventContext, AstrError, CancelToken, EventTranslator, ExecutionOwner, Phase,
    SessionTurnLease, StorageEvent, StoredEvent, UserMessageOrigin,
};
use astrcode_runtime_agent_control::AgentControl;
use astrcode_runtime_agent_loop::{
    AgentLoop, CompactionTailSnapshot, TokenBudgetDecision, TurnOutcome, build_auto_continue_nudge,
    check_token_budget, estimate_text_tokens,
};
use chrono::Utc;

use crate::{
    SessionState, SessionTokenBudgetState,
    support::{lock_anyhow, with_lock_recovery},
};

#[derive(Debug, Clone, Copy)]
pub struct BudgetSettings {
    pub continuation_min_delta_tokens: usize,
    pub max_continuations: u8,
}

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

pub fn prepare_session_execution(
    session: &SessionState,
    session_id: &str,
    turn_id: &str,
    cancel: CancelToken,
    turn_lease: Box<dyn SessionTurnLease>,
    token_budget: Option<u64>,
) -> Result<()> {
    let mut cancel_guard = lock_anyhow(&session.cancel, "session cancel")?;
    let mut active_turn_guard = lock_anyhow(&session.active_turn_id, "session active turn")?;
    let mut lease_guard = lock_anyhow(&session.turn_lease, "session turn lease")?;
    let mut budget_guard = lock_anyhow(&session.token_budget, "session token budget")?;
    if session.running.swap(true, Ordering::SeqCst) {
        return Err(anyhow::Error::from(AstrError::Validation(format!(
            "session '{}' entered an inconsistent running state",
            session_id
        ))));
    }
    *cancel_guard = cancel;
    *active_turn_guard = Some(turn_id.to_string());
    *lease_guard = Some(turn_lease);
    *budget_guard = token_budget.map(|total_budget| SessionTokenBudgetState {
        total_budget,
        used_tokens: 0,
        continuation_count: 0,
    });
    Ok(())
}

pub async fn complete_session_execution(
    session: &SessionState,
    agent_control: &AgentControl,
    turn_id: &str,
    phase: Phase,
) {
    with_lock_recovery(&session.phase, "session phase", |phase_guard| {
        *phase_guard = phase;
    });
    with_lock_recovery(
        &session.active_turn_id,
        "session active turn",
        |active_turn_guard| {
            *active_turn_guard = None;
        },
    );
    let _ = agent_control.cancel_for_parent_turn(turn_id).await;
    with_lock_recovery(&session.turn_lease, "session turn lease", |lease_guard| {
        *lease_guard = None;
    });
    with_lock_recovery(
        &session.token_budget,
        "session token budget",
        |budget_guard| {
            *budget_guard = None;
        },
    );
    with_lock_recovery(&session.cancel, "session cancel", |cancel_guard| {
        *cancel_guard = CancelToken::new();
    });
    session.running.store(false, Ordering::SeqCst);
}

// 这里的参数和运行时回调链一一对应，先保留显式签名以避免把调用点语义埋进匿名元组。
#[allow(clippy::too_many_arguments)]
pub async fn run_session_turn(
    session: &SessionState,
    loop_: &AgentLoop,
    turn_id: &str,
    cancel: CancelToken,
    user_event: StorageEvent,
    agent: AgentEventContext,
    execution_owner: ExecutionOwner,
    budget_settings: BudgetSettings,
) -> SessionTurnRunResult {
    let initial_phase = lock_anyhow(&session.phase, "session phase")
        .map(|guard| *guard)
        .unwrap_or(Phase::Idle);
    let mut translator = EventTranslator::new(initial_phase);
    let outcome =
        match append_and_broadcast_from_turn_callback(session, &user_event, &mut translator) {
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
                )
                .await
            },
            Err(error) => Err(AstrError::Internal(error.to_string())),
        };
    let succeeded = matches!(
        outcome.as_ref(),
        Ok(TurnOutcome::Completed) | Ok(TurnOutcome::Cancelled)
    );
    if let Err(error) = &outcome {
        let error_event = StorageEvent::Error {
            turn_id: Some(turn_id.to_string()),
            agent: agent.clone(),
            message: error.to_string(),
            timestamp: Some(Utc::now()),
        };
        let _ = append_and_broadcast_from_turn_callback(session, &error_event, &mut translator);
        let turn_done = StorageEvent::TurnDone {
            turn_id: Some(turn_id.to_string()),
            agent,
            timestamp: Utc::now(),
            reason: Some("error".to_string()),
        };
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
            .run_turn_without_finish_with_compaction_tail(
                &projected,
                turn_id,
                &mut |event| {
                    observe_turn_event(&mut stats, &event);
                    let stored = append_and_broadcast_from_turn_callback(state, &event, translator)
                        .map_err(|error| AstrError::Internal(error.to_string()))?;
                    if should_record_compaction_tail_event(&event) {
                        with_lock_recovery(&live_tail, "compaction live tail", |tail| {
                            tail.push(stored);
                        });
                    }
                    Ok(())
                },
                cancel.clone(),
                agent.clone(),
                execution_owner.clone(),
                CompactionTailSnapshot::from_seed(tail_seed)
                    .with_live_recorder(Arc::clone(&live_tail)),
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
            &StorageEvent::TurnDone {
                turn_id: Some(turn_id.to_string()),
                agent: agent.clone(),
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
    agent: AgentEventContext,
    stats: TurnExecutionStats,
    budget_settings: BudgetSettings,
) -> Result<bool> {
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

    append_and_broadcast(
        state,
        &StorageEvent::UserMessage {
            turn_id: Some(turn_id.to_string()),
            agent,
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
        },
        StorageEvent::AssistantFinal {
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

pub async fn append_and_broadcast(
    session: &SessionState,
    event: &StorageEvent,
    translator: &mut EventTranslator,
) -> Result<StoredEvent> {
    let stored = session.writer.clone().append(event.clone()).await?;
    let records = session.translate_store_and_cache(&stored, translator)?;
    for record in records {
        let _ = session.broadcaster.send(record);
    }
    Ok(stored)
}

fn append_and_broadcast_blocking(
    session: &SessionState,
    event: &StorageEvent,
    translator: &mut EventTranslator,
) -> Result<StoredEvent> {
    let stored = session.writer.append_blocking(event)?;
    let records = session.translate_store_and_cache(&stored, translator)?;
    for record in records {
        let _ = session.broadcaster.send(record);
    }
    Ok(stored)
}

pub fn append_and_broadcast_from_turn_callback(
    session: &SessionState,
    event: &StorageEvent,
    translator: &mut EventTranslator,
) -> Result<StoredEvent> {
    match tokio::runtime::Handle::current().runtime_flavor() {
        tokio::runtime::RuntimeFlavor::CurrentThread => {
            append_and_broadcast_blocking(session, event, translator)
        },
        _ => tokio::task::block_in_place(|| {
            append_and_broadcast_blocking(session, event, translator)
        }),
    }
}

/// Manual / auto compact 都应该基于 durable tail，而不是投影后的消息列表。
/// 公开导出给 runtime façade 使用，避免重复定义。
pub fn recent_turn_event_tail(
    events: &[StoredEvent],
    keep_recent_turns: usize,
) -> Vec<StoredEvent> {
    let keep_recent_turns = keep_recent_turns.max(1);
    let mut tail_refs = Vec::new();
    let mut kept_turn_starts = VecDeque::with_capacity(keep_recent_turns);

    for stored in events {
        if !should_record_compaction_tail_event(&stored.event) {
            continue;
        }
        if matches!(
            &stored.event,
            StorageEvent::UserMessage {
                origin: UserMessageOrigin::User,
                ..
            }
        ) {
            kept_turn_starts.push_back(tail_refs.len());
            if kept_turn_starts.len() > keep_recent_turns {
                kept_turn_starts.pop_front();
            }
        }
        tail_refs.push(stored);
    }

    let keep_start = kept_turn_starts.front().copied().unwrap_or(0);
    tail_refs.into_iter().skip(keep_start).cloned().collect()
}

/// 判断事件是否应纳入 compaction tail 记录。
/// 只有用户消息、助手回复、工具调用和工具结果需要保留用于 compaction。
pub fn should_record_compaction_tail_event(event: &StorageEvent) -> bool {
    matches!(
        event,
        StorageEvent::UserMessage { .. }
            | StorageEvent::AssistantFinal { .. }
            | StorageEvent::ToolCall { .. }
            | StorageEvent::ToolResult { .. }
    )
}

#[cfg(test)]
mod tests {
    use std::{
        panic::{AssertUnwindSafe, catch_unwind},
        sync::Arc,
    };

    use astrcode_core::{
        AgentStateProjector, EventLogWriter, Phase, SessionTurnLease, StoreResult, StoredEvent,
    };
    use astrcode_runtime_agent_control::AgentControl;

    use super::*;
    use crate::SessionWriter;

    #[derive(Default)]
    struct TestLease;

    impl SessionTurnLease for TestLease {}

    #[derive(Default)]
    struct TestEventLogWriter {
        next_seq: u64,
    }

    impl EventLogWriter for TestEventLogWriter {
        fn append(&mut self, event: &StorageEvent) -> StoreResult<StoredEvent> {
            self.next_seq += 1;
            Ok(StoredEvent {
                storage_seq: self.next_seq,
                event: event.clone(),
            })
        }
    }

    fn test_session() -> SessionState {
        SessionState::new(
            Phase::Idle,
            Arc::new(SessionWriter::new(Box::new(TestEventLogWriter::default()))),
            AgentStateProjector::default(),
            Vec::new(),
            Vec::new(),
        )
    }

    fn poison_mutex<T>(mutex: &StdMutex<T>) {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let _guard = mutex.lock().expect("mutex should lock");
            panic!("poison mutex for recovery test");
        }));
    }

    #[test]
    fn prepare_session_execution_keeps_session_idle_when_budget_lock_fails() {
        let session = test_session();
        poison_mutex(&session.token_budget);

        let error = prepare_session_execution(
            &session,
            "session-1",
            "turn-1",
            CancelToken::new(),
            Box::new(TestLease),
            Some(128),
        )
        .expect_err("poisoned budget lock should fail preparation");

        assert!(error.to_string().contains("session token budget"));
        assert!(!session.running.load(Ordering::SeqCst));
        assert!(
            session
                .active_turn_id
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_none()
        );
        assert!(
            session
                .turn_lease
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_none()
        );
        assert!(
            session
                .token_budget
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_none()
        );
    }

    #[tokio::test]
    async fn complete_session_execution_recovers_poisoned_mutexes() {
        let session = test_session();
        session.running.store(true, Ordering::SeqCst);

        let _ = catch_unwind(AssertUnwindSafe(|| {
            let mut guard = session.phase.lock().expect("phase should lock");
            *guard = Phase::CallingTool;
            panic!("poison phase");
        }));
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let mut guard = session
                .active_turn_id
                .lock()
                .expect("active turn should lock");
            *guard = Some("turn-1".to_string());
            panic!("poison active turn");
        }));
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let mut guard = session.turn_lease.lock().expect("turn lease should lock");
            *guard = Some(Box::new(TestLease));
            panic!("poison turn lease");
        }));
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let mut guard = session
                .token_budget
                .lock()
                .expect("token budget should lock");
            *guard = Some(SessionTokenBudgetState {
                total_budget: 512,
                used_tokens: 32,
                continuation_count: 1,
            });
            panic!("poison token budget");
        }));
        poison_mutex(&session.cancel);

        complete_session_execution(&session, &AgentControl::new(), "turn-1", Phase::Idle).await;

        assert_eq!(
            session.current_phase().expect("phase should recover"),
            Phase::Idle
        );
        assert!(
            session
                .active_turn_id
                .lock()
                .expect("active turn lock should recover")
                .is_none()
        );
        assert!(
            session
                .turn_lease
                .lock()
                .expect("turn lease lock should recover")
                .is_none()
        );
        assert!(
            session
                .token_budget
                .lock()
                .expect("token budget lock should recover")
                .is_none()
        );
        assert!(!session.running.load(Ordering::SeqCst));
    }

    #[test]
    fn recent_turn_event_tail_keeps_latest_turn_when_keep_recent_turns_is_zero() {
        let events = vec![
            StoredEvent {
                storage_seq: 1,
                event: StorageEvent::UserMessage {
                    turn_id: Some("turn-1".to_string()),
                    agent: AgentEventContext::default(),
                    content: "first".to_string(),
                    origin: UserMessageOrigin::User,
                    timestamp: Utc::now(),
                },
            },
            StoredEvent {
                storage_seq: 2,
                event: StorageEvent::AssistantFinal {
                    turn_id: Some("turn-1".to_string()),
                    agent: AgentEventContext::default(),
                    content: "reply-1".to_string(),
                    reasoning_content: None,
                    reasoning_signature: None,
                    timestamp: Some(Utc::now()),
                },
            },
            StoredEvent {
                storage_seq: 3,
                event: StorageEvent::UserMessage {
                    turn_id: Some("turn-2".to_string()),
                    agent: AgentEventContext::default(),
                    content: "second".to_string(),
                    origin: UserMessageOrigin::User,
                    timestamp: Utc::now(),
                },
            },
            StoredEvent {
                storage_seq: 4,
                event: StorageEvent::AssistantFinal {
                    turn_id: Some("turn-2".to_string()),
                    agent: AgentEventContext::default(),
                    content: "reply-2".to_string(),
                    reasoning_content: None,
                    reasoning_signature: None,
                    timestamp: Some(Utc::now()),
                },
            },
        ];

        let tail = recent_turn_event_tail(&events, 0);

        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0].storage_seq, 3);
        assert_eq!(tail[1].storage_seq, 4);
    }
}
