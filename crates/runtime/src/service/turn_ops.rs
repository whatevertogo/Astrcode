use std::sync::atomic::Ordering;
use std::time::Instant;

use anyhow::Result;
use astrcode_core::{AstrError, CancelToken, Phase};
use chrono::Utc;
use uuid::Uuid;

use astrcode_core::StorageEvent;

use super::session_ops::normalize_session_id;
use super::session_state::SessionState;
use super::support::lock_anyhow;
use super::{PromptAccepted, RuntimeService, ServiceError, ServiceResult};
use astrcode_core::EventTranslator;

impl RuntimeService {
    pub async fn submit_prompt(
        &self,
        session_id: &str,
        text: String,
    ) -> ServiceResult<PromptAccepted> {
        let session_id = normalize_session_id(session_id);
        let session = self.ensure_session_loaded(&session_id).await?;
        let turn_id = Uuid::new_v4().to_string();
        let cancel = CancelToken::new();
        {
            let mut guard = lock_anyhow(&session.cancel, "session cancel")?;
            // 无锁互斥守卫：swap(true, SeqCst) 同时完成"测试是否已运行"和"标记为运行中"。
            // SeqCst ordering 确保 running 标志的变更对所有 async task 可见，
            // 包括可能正在读取 running 的 shutdown 路径。比 Acquire/Release 更强，
            // 但此处的性能影响可忽略（每次 turn 只调用一次）。
            if session.running.swap(true, Ordering::SeqCst) {
                return Err(ServiceError::Conflict(format!(
                    "session '{}' is already running",
                    session_id
                )));
            }
            *guard = cancel.clone();
        }

        let state = session.clone();
        let loop_ = self.current_loop().await;
        let text_for_task = text;

        let accepted_turn_id = turn_id.clone();
        let observability = self.observability.clone();
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
                                append_and_broadcast_blocking(&state, &event, &mut translator)
                                    .map_err(|error| AstrError::Internal(error.to_string()))
                            },
                            cancel.clone(),
                        )
                        .await
                }
                Err(error) => Err(error),
            };

            let succeeded = result.is_ok();
            if let Err(error) = result {
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
                };
                let _ = append_and_broadcast(&state, &turn_done, &mut translator).await;
            }

            if let Ok(mut phase) = lock_anyhow(&state.phase, "session phase") {
                *phase = translator.phase;
            }
            // 重置 CancelToken：前一个 token 已被消费（正常完成或被取消），
            // 必须替换为新的空 token 以"重新武装"会话，否则下一次 interrupt()
            // 调用会触发一个已过期的 token，无法取消新的 turn。
            if let Ok(mut guard) = lock_anyhow(&state.cancel, "session cancel") {
                *guard = CancelToken::new();
            }
            state.running.store(false, Ordering::SeqCst);

            let elapsed = turn_started_at.elapsed();
            observability.record_turn_execution(elapsed, succeeded);
            if succeeded {
                if elapsed.as_millis() >= 5_000 {
                    log::warn!(
                        "turn '{}' completed slowly in {}ms",
                        turn_id,
                        elapsed.as_millis()
                    );
                } else {
                    log::info!("turn '{}' completed in {}ms", turn_id, elapsed.as_millis());
                }
            } else {
                log::warn!("turn '{}' failed in {}ms", turn_id, elapsed.as_millis());
            }
        });

        Ok(PromptAccepted {
            turn_id: accepted_turn_id,
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use astrcode_core::AgentEvent;
    use chrono::Utc;

    use astrcode_storage::session::EventLog;

    use super::super::session_state::SessionWriter;
    use super::*;
    use crate::test_support::TestEnvGuard;

    #[tokio::test(flavor = "current_thread")]
    async fn append_and_broadcast_blocking_works_on_current_thread_runtime() {
        let _guard = TestEnvGuard::new();
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
        let mut receiver = state.broadcaster.subscribe();
        let mut translator = EventTranslator::new(Phase::Idle);

        append_and_broadcast_blocking(
            &state,
            &StorageEvent::SessionStart {
                session_id: "test-session".to_string(),
                timestamp: Utc::now(),
                working_dir: temp_dir.path().to_string_lossy().to_string(),
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
}
