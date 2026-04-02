use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use astrcode_core::{
    generate_session_id, AstrError, CancelToken, Phase, SessionTurnAcquireResult, SessionTurnLease,
    StorageEvent, StoredEvent,
};
use chrono::Utc;
use uuid::Uuid;

use super::session_ops::normalize_session_id;
use super::session_state::SessionState;
use super::support::{lock_anyhow, spawn_blocking_service};
use super::{PromptAccepted, RuntimeService, ServiceError, ServiceResult};
use crate::agent_loop::TurnOutcome;
use astrcode_core::EventTranslator;

struct SubmitTarget {
    session_id: String,
    branched_from_session_id: Option<String>,
    session: Arc<SessionState>,
    turn_lease: Box<dyn SessionTurnLease>,
}

const MAX_CONCURRENT_BRANCH_DEPTH: usize = 3;

impl RuntimeService {
    pub async fn submit_prompt(
        &self,
        session_id: &str,
        text: String,
    ) -> ServiceResult<PromptAccepted> {
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
            };

            let task_result = match append_and_broadcast(&state, &user_event, &mut translator).await
            {
                Ok(()) => state
                    .snapshot_projected_state()
                    .map_err(|error| AstrError::Internal(error.to_string())),
                Err(error) => Err(AstrError::Internal(error.to_string())),
            };

            let result = match task_result {
                Ok(projected) => {
                    loop_
                        .run_turn(
                            &projected,
                            &turn_id,
                            &mut |event| {
                                append_and_broadcast_from_turn_callback(
                                    &state,
                                    &event,
                                    &mut translator,
                                )
                                .map_err(|error| AstrError::Internal(error.to_string()))
                            },
                            cancel.clone(),
                        )
                        .await
                }
                Err(error) => Err(error),
            };

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
    use std::sync::Arc;

    use astrcode_core::AgentEvent;
    use chrono::Utc;

    use astrcode_storage::session::EventLog;
    use serde_json::json;

    use super::super::session_state::SessionWriter;
    use super::*;
    use crate::test_support::TestEnvGuard;

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
}
