use std::sync::atomic::Ordering;

use anyhow::Result;
use astrcode_core::{AstrError, CancelToken, Phase};
use chrono::Utc;
use uuid::Uuid;

use crate::events::StorageEvent;
use crate::projection::project;

use super::replay::EventTranslator;
use super::session_ops::{load_events, normalize_session_id};
use super::session_state::SessionState;
use super::support::{lock_anyhow, lock_service};
use super::{AgentService, PromptAccepted, ServiceError, ServiceResult};

impl AgentService {
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
            let mut guard = lock_service(&session.cancel, "session cancel")?;
            if session.running.swap(true, Ordering::SeqCst) {
                return Err(ServiceError::Conflict(format!(
                    "session '{}' is already running",
                    session_id
                )));
            }
            *guard = cancel.clone();
        }

        let state = session.clone();
        let session_id_for_task = session_id.clone();
        let loop_ = self.loop_.clone();
        let text_for_task = text;

        let accepted_turn_id = turn_id.clone();
        tokio::spawn(async move {
            let initial_phase = lock_anyhow(&state.phase, "session phase")
                .map(|guard| guard.clone())
                .unwrap_or(Phase::Idle);
            let mut translator = EventTranslator::new(initial_phase);

            let user_event = StorageEvent::UserMessage {
                turn_id: Some(turn_id.clone()),
                content: text_for_task,
                timestamp: Utc::now(),
            };

            let task_result = match append_and_broadcast(&state, &user_event, &mut translator).await
            {
                Ok(()) => load_events(&session_id_for_task)
                    .await
                    .map(|events| {
                        events
                            .into_iter()
                            .map(|stored| stored.event)
                            .collect::<Vec<_>>()
                    })
                    .map(|events| project(&events))
                    .map_err(|error| AstrError::Internal(error.to_string())),
                Err(error) => Err(AstrError::Internal(error.to_string())),
            };

            let result = match task_result {
                Ok(projected) => loop_
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
                    .map_err(|error| error),
                Err(error) => Err(error),
            };

            if let Err(error) = result {
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
            if let Ok(mut guard) = lock_anyhow(&state.cancel, "session cancel") {
                *guard = CancelToken::new();
            }
            state.running.store(false, Ordering::SeqCst);
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

async fn append_and_broadcast(
    session: &SessionState,
    event: &StorageEvent,
    translator: &mut EventTranslator,
) -> Result<()> {
    let stored = session.writer.clone().append(event.clone()).await?;
    let records = translator.translate(&stored);
    for record in records {
        let _ = session.broadcaster.send(record);
    }
    Ok(())
}

fn append_and_broadcast_blocking(
    session: &SessionState,
    event: &StorageEvent,
    translator: &mut EventTranslator,
) -> Result<()> {
    let stored = session.writer.append_blocking(event)?;
    let records = translator.translate(&stored);
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

    use crate::event_log::EventLog;

    use super::super::session_state::SessionWriter;
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn append_and_broadcast_blocking_works_on_current_thread_runtime() {
        let temp_dir = tempfile::tempdir().expect("tempdir should be created");
        let log = EventLog::create_at_path(
            "test-session",
            temp_dir.path().join("session-test-session.jsonl"),
        )
        .expect("event log should be created");
        let state = SessionState::new(
            temp_dir.path().to_path_buf(),
            Phase::Idle,
            Arc::new(SessionWriter::new(log)),
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
