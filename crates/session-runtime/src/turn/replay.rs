use astrcode_core::{Result, SessionId, replay_records};

use crate::{SessionHistorySnapshot, SessionReplay, SessionRuntime, SessionViewSnapshot};

impl SessionRuntime {
    pub async fn session_history(&self, session_id: &str) -> Result<SessionHistorySnapshot> {
        let session_id = SessionId::from(crate::state::normalize_session_id(session_id));
        let records = self.replay_history(&session_id, None).await?;
        let phase = self.session_phase(&session_id).await?;
        Ok(SessionHistorySnapshot {
            cursor: records.last().map(|record| record.event_id.clone()),
            history: records,
            phase,
        })
    }

    pub async fn session_view(&self, session_id: &str) -> Result<SessionViewSnapshot> {
        let history = self.session_history(session_id).await?;
        Ok(SessionViewSnapshot {
            focus_history: history.history.clone(),
            direct_children_history: Vec::new(),
            cursor: history.cursor,
            phase: history.phase,
        })
    }

    pub async fn session_replay(
        &self,
        session_id: &str,
        last_event_id: Option<&str>,
    ) -> Result<SessionReplay> {
        let session_id = SessionId::from(crate::state::normalize_session_id(session_id));
        let actor = self.ensure_loaded_session(&session_id).await?;
        let history = self.replay_history(&session_id, last_event_id).await?;
        Ok(SessionReplay {
            history,
            receiver: actor.state().broadcaster.subscribe(),
            live_receiver: actor.state().subscribe_live(),
        })
    }

    pub(crate) async fn replay_history(
        &self,
        session_id: &SessionId,
        last_event_id: Option<&str>,
    ) -> Result<Vec<astrcode_core::SessionEventRecord>> {
        let actor = self.ensure_loaded_session(session_id).await?;
        if let Some(history) = actor.state().recent_records_after(last_event_id)? {
            return Ok(history);
        }

        let stored = self.event_store.replay(session_id).await?;
        Ok(replay_records(&stored, last_event_id))
    }
}
