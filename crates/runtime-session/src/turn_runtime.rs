use std::{collections::VecDeque, sync::atomic::Ordering};

use anyhow::Result;
use astrcode_core::{
    AstrError, CancelToken, EventTranslator, Phase, SessionTurnLease, StorageEvent, StoredEvent,
    UserMessageOrigin,
};

use crate::{SessionState, SessionTokenBudgetState, support::lock_anyhow};

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

pub fn complete_session_execution(session: &SessionState, phase: Phase) {
    session.complete_execution_state(phase);
}

pub async fn append_and_broadcast(
    session: &SessionState,
    event: &StorageEvent,
    translator: &mut EventTranslator,
) -> Result<StoredEvent> {
    let stored = session.writer.clone().append(event.clone()).await?;
    let records = session.translate_store_and_cache(&stored, translator)?;
    for record in records {
        // 故意忽略：broadcast channel 关闭表示 session 已终止，无需处理
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
        sync::{Arc, Mutex as StdMutex},
    };

    use astrcode_core::{
        AgentEventContext, AgentStateProjector, EventLogWriter, Phase, SessionTurnLease,
        StoreResult, StoredEvent, UserMessageOrigin,
    };
    use chrono::Utc;

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

    #[test]
    fn complete_session_execution_recovers_poisoned_mutexes() {
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

        complete_session_execution(&session, Phase::Idle);

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

    #[test]
    fn turn_runtime_surface_exports_session_boundary_primitives() {
        let _prepare_signature = prepare_session_execution;
        let _complete_signature = complete_session_execution;
        let _append_signature = append_and_broadcast;
        let _callback_append_signature = append_and_broadcast_from_turn_callback;
        let _tail_signature = recent_turn_event_tail;
        let _record_signature = should_record_compaction_tail_event;
    }
}
