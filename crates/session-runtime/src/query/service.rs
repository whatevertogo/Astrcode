use std::{sync::Arc, time::Duration};

use astrcode_core::{
    AgentLifecycleStatus, ChildSessionNode, Phase, Result, SessionId, StorageEventPayload,
    StoredEvent,
};
use tokio::time::sleep;

use crate::{
    AgentObserveSnapshot, ConversationSnapshotFacts, ConversationStreamReplayFacts,
    LastCompactMetaSnapshot, ProjectedTurnOutcome, SessionControlStateSnapshot, SessionReplay,
    SessionRuntime, SessionState, TurnTerminalSnapshot, build_agent_observe_snapshot,
    build_conversation_replay_frames, has_terminal_turn_signal, project_conversation_snapshot,
    project_turn_outcome, recoverable_parent_deliveries,
};

pub struct SessionQueries<'a> {
    runtime: &'a SessionRuntime,
}

impl<'a> SessionQueries<'a> {
    pub fn new(runtime: &'a SessionRuntime) -> Self {
        Self { runtime }
    }

    pub async fn observe(
        &self,
        session_id: &SessionId,
    ) -> Result<crate::observe::SessionObserveSnapshot> {
        let actor = self.runtime.ensure_loaded_session(session_id).await?;
        Ok(crate::observe::SessionObserveSnapshot {
            state: actor.snapshot(),
        })
    }

    pub async fn session_state(&self, session_id: &SessionId) -> Result<Arc<SessionState>> {
        let actor = self.runtime.ensure_loaded_session(session_id).await?;
        Ok(Arc::clone(actor.state()))
    }

    pub async fn session_control_state(
        &self,
        session_id: &str,
    ) -> Result<SessionControlStateSnapshot> {
        let session_id = SessionId::from(crate::normalize_session_id(session_id));
        let actor = self.runtime.ensure_loaded_session(&session_id).await?;
        let last_compact_meta = actor
            .state()
            .snapshot_recent_stored_events()?
            .into_iter()
            .rev()
            .find_map(|stored| match stored.event.payload {
                StorageEventPayload::CompactApplied { trigger, meta, .. } => {
                    Some(LastCompactMetaSnapshot { trigger, meta })
                },
                _ => None,
            });
        Ok(SessionControlStateSnapshot {
            phase: actor.state().current_phase()?,
            active_turn_id: actor.state().active_turn_id_snapshot()?,
            manual_compact_pending: actor.state().manual_compact_pending()?,
            compacting: actor.state().compacting(),
            last_compact_meta,
        })
    }

    pub async fn session_child_nodes(&self, session_id: &str) -> Result<Vec<ChildSessionNode>> {
        let session_id = SessionId::from(crate::normalize_session_id(session_id));
        let actor = self.runtime.ensure_loaded_session(&session_id).await?;
        actor.state().list_child_session_nodes()
    }

    pub async fn session_working_dir(&self, session_id: &str) -> Result<String> {
        let session_id = SessionId::from(crate::normalize_session_id(session_id));
        let actor = self.runtime.ensure_loaded_session(&session_id).await?;
        Ok(actor.working_dir().to_string())
    }

    pub async fn stored_events(&self, session_id: &SessionId) -> Result<Vec<StoredEvent>> {
        self.runtime.ensure_session_exists(session_id).await?;
        self.runtime.event_store.replay(session_id).await
    }

    pub async fn wait_for_turn_terminal_snapshot(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> Result<TurnTerminalSnapshot> {
        let session_id = SessionId::from(crate::normalize_session_id(session_id));
        loop {
            let state = self.session_state(&session_id).await?;
            let phase = state.current_phase()?;
            if matches!(phase, Phase::Idle | Phase::Interrupted | Phase::Done) {
                let events = self
                    .stored_events(&session_id)
                    .await?
                    .into_iter()
                    .filter(|stored| stored.event.turn_id() == Some(turn_id))
                    .collect::<Vec<_>>();
                if has_terminal_turn_signal(&events) || matches!(phase, Phase::Interrupted) {
                    return Ok(TurnTerminalSnapshot { phase, events });
                }
            }
            sleep(Duration::from_millis(20)).await;
        }
    }

    pub async fn observe_agent_session(
        &self,
        open_session_id: &str,
        target_agent_id: &str,
        lifecycle_status: AgentLifecycleStatus,
    ) -> Result<AgentObserveSnapshot> {
        let session_id = SessionId::from(crate::normalize_session_id(open_session_id));
        let session_state = self.session_state(&session_id).await?;
        let projected = session_state.snapshot_projected_state()?;
        let mailbox_projection = session_state.mailbox_projection_for_agent(target_agent_id)?;
        let stored_events = self.stored_events(&session_id).await?;
        Ok(build_agent_observe_snapshot(
            lifecycle_status,
            &projected,
            &mailbox_projection,
            &stored_events,
            target_agent_id,
        ))
    }

    pub async fn conversation_snapshot(
        &self,
        session_id: &str,
    ) -> Result<ConversationSnapshotFacts> {
        let session_id = SessionId::from(crate::normalize_session_id(session_id));
        let records = self.runtime.replay_history(&session_id, None).await?;
        let phase = self.runtime.session_phase(&session_id).await?;
        Ok(project_conversation_snapshot(&records, phase))
    }

    pub async fn conversation_stream_replay(
        &self,
        session_id: &str,
        last_event_id: Option<&str>,
    ) -> Result<ConversationStreamReplayFacts> {
        let session_id = SessionId::from(crate::normalize_session_id(session_id));
        let actor = self.runtime.ensure_loaded_session(&session_id).await?;
        let all_records = self.runtime.replay_history(&session_id, None).await?;
        let replay_history = self
            .runtime
            .replay_history(&session_id, last_event_id)
            .await?;
        let seed_records = records_before_cursor(&all_records, last_event_id);
        let phase = self.runtime.session_phase(&session_id).await?;

        Ok(ConversationStreamReplayFacts {
            cursor: replay_history.last().map(|record| record.event_id.clone()),
            phase,
            replay_frames: build_conversation_replay_frames(&seed_records, &replay_history),
            seed_records,
            replay: SessionReplay {
                history: replay_history,
                receiver: actor.state().broadcaster.subscribe(),
                live_receiver: actor.state().subscribe_live(),
            },
        })
    }

    pub async fn pending_delivery_ids_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Vec<String>> {
        let session_id = SessionId::from(crate::normalize_session_id(session_id));
        let session_state = self.session_state(&session_id).await?;
        Ok(session_state
            .mailbox_projection_for_agent(agent_id)?
            .pending_delivery_ids
            .into_iter()
            .map(Into::into)
            .collect())
    }

    pub async fn recoverable_parent_deliveries(
        &self,
        parent_session_id: &str,
    ) -> Result<Vec<astrcode_kernel::PendingParentDelivery>> {
        let session_id = SessionId::from(crate::normalize_session_id(parent_session_id));
        let events = self.stored_events(&session_id).await?;
        Ok(recoverable_parent_deliveries(&events))
    }

    pub async fn project_turn_outcome(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> Result<ProjectedTurnOutcome> {
        let terminal = self
            .wait_for_turn_terminal_snapshot(session_id, turn_id)
            .await?;
        Ok(project_turn_outcome(terminal.phase, &terminal.events))
    }
}

fn records_before_cursor(
    records: &[astrcode_core::SessionEventRecord],
    last_event_id: Option<&str>,
) -> Vec<astrcode_core::SessionEventRecord> {
    let Some(last_event_id) = last_event_id else {
        return Vec::new();
    };
    let Some(index) = records
        .iter()
        .position(|record| record.event_id == last_event_id)
    else {
        return Vec::new();
    };
    records[..=index].to_vec()
}
