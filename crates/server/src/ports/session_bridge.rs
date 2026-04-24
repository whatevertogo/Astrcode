use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use astrcode_core::{
    AgentCollaborationFact, AgentEventContext, AgentLifecycleStatus, AstrError, ExecutionAccepted,
    InputBatchAckedPayload, InputBatchStartedPayload, InputDiscardedPayload, InputQueuedPayload,
    InvocationKind, ResolvedExecutionLimitsSnapshot, ResolvedRuntimeConfig,
    ResolvedSubagentContextOverrides, SessionEventRecord, SessionId, SessionMeta,
    StorageEventPayload, StoredEvent, SubRunResult, SubRunStorageMode, TurnId, replay_records,
};
use astrcode_host_session::{
    ForkPoint, InputQueueProjection, ProjectedTurnOutcome, SessionCatalog, SubRunHandle,
};
use async_trait::async_trait;
use tokio::sync::broadcast;

use super::{
    AgentSessionPort, AppAgentPromptSubmission, AppSessionPort, DurableSubRunStatusSummary,
    RecoverableParentDelivery, SessionObserveSnapshot, SessionTurnOutcomeSummary,
    SessionTurnTerminalState,
};
use crate::{
    conversation_read_model::{
        ConversationSnapshotFacts, ConversationStreamReplayFacts, SessionReplay,
        SessionTranscriptSnapshot, build_conversation_replay_frames, project_conversation_snapshot,
    },
    session_identity::normalize_external_session_id,
    session_runtime_port::SessionRuntimePort,
    session_use_cases::SessionForkSelector,
};

pub(crate) fn build_server_session_bridge(
    session_catalog: Arc<SessionCatalog>,
    session_runtime: Arc<dyn SessionRuntimePort>,
) -> Arc<ServerSessionBridge> {
    Arc::new(ServerSessionBridge {
        session_catalog,
        session_runtime,
    })
}

pub(crate) struct ServerSessionBridge {
    session_catalog: Arc<SessionCatalog>,
    session_runtime: Arc<dyn SessionRuntimePort>,
}

impl ServerSessionBridge {
    fn session_id(session_id: &str) -> SessionId {
        SessionId::from(normalize_external_session_id(session_id))
    }

    async fn replay_history(
        &self,
        session_id: &SessionId,
        last_event_id: Option<&str>,
    ) -> astrcode_core::Result<Vec<SessionEventRecord>> {
        let state = self.session_catalog.session_state(session_id).await?;
        if let Some(history) = state.recent_records_after(last_event_id)? {
            return Ok(history);
        }

        let stored = self.session_catalog.stored_events(session_id).await?;
        Ok(replay_records(&stored, last_event_id))
    }

    async fn session_phase(
        &self,
        session_id: &SessionId,
    ) -> astrcode_core::Result<astrcode_core::Phase> {
        self.session_catalog
            .session_state(session_id)
            .await?
            .current_phase()
    }

    async fn session_meta(&self, session_id: &str) -> astrcode_core::Result<SessionMeta> {
        let requested = normalize_external_session_id(session_id);
        self.session_catalog
            .list_session_metas()
            .await?
            .into_iter()
            .find(|meta| normalize_external_session_id(&meta.session_id) == requested)
            .ok_or_else(|| AstrError::SessionNotFound(session_id.to_string()))
    }

    async fn durable_subrun_status_summary(
        &self,
        parent_session_id: &str,
        requested_subrun_id: &str,
    ) -> astrcode_core::Result<Option<DurableSubRunStatusSummary>> {
        let requested_parent_id = normalize_external_session_id(parent_session_id);
        for meta in self.session_catalog.list_session_metas().await? {
            if meta
                .parent_session_id
                .as_deref()
                .map(normalize_external_session_id)
                .as_deref()
                != Some(requested_parent_id.as_str())
            {
                continue;
            }

            let child_session_id = Self::session_id(&meta.session_id);
            let stored_events = self
                .session_catalog
                .stored_events(&child_session_id)
                .await?;
            if let Some(snapshot) = project_durable_subrun_status_summary(
                parent_session_id,
                meta.session_id.as_str(),
                requested_subrun_id,
                &stored_events,
            ) {
                return Ok(Some(snapshot));
            }
        }

        Ok(None)
    }
}

#[async_trait]
impl AppSessionPort for ServerSessionBridge {
    fn subscribe_catalog_events(
        &self,
    ) -> broadcast::Receiver<astrcode_host_session::SessionCatalogEvent> {
        self.session_catalog.subscribe_catalog_events()
    }

    async fn list_session_metas(&self) -> astrcode_core::Result<Vec<SessionMeta>> {
        self.session_catalog.list_session_metas().await
    }

    async fn create_session(&self, working_dir: String) -> astrcode_core::Result<SessionMeta> {
        self.session_catalog.create_session(working_dir).await
    }

    async fn fork_session(
        &self,
        session_id: &str,
        selector: SessionForkSelector,
    ) -> astrcode_core::Result<SessionMeta> {
        let fork_point = match selector {
            SessionForkSelector::Latest => ForkPoint::Latest,
            SessionForkSelector::TurnEnd { turn_id } => ForkPoint::TurnEnd(turn_id),
            SessionForkSelector::StorageSeq { storage_seq } => ForkPoint::StorageSeq(storage_seq),
        };
        let result = self
            .session_catalog
            .fork_session(&Self::session_id(session_id), fork_point)
            .await?;
        self.session_meta(result.new_session_id.as_str()).await
    }

    async fn delete_session(&self, session_id: &str) -> astrcode_core::Result<()> {
        self.session_catalog
            .delete_session(&Self::session_id(session_id))
            .await
    }

    async fn delete_project(
        &self,
        working_dir: &str,
    ) -> astrcode_core::Result<astrcode_core::DeleteProjectResult> {
        self.session_catalog.delete_project(working_dir).await
    }

    async fn get_session_working_dir(&self, session_id: &str) -> astrcode_core::Result<String> {
        Ok(self.session_meta(session_id).await?.working_dir)
    }

    async fn submit_prompt_for_agent(
        &self,
        session_id: &str,
        text: String,
        runtime: ResolvedRuntimeConfig,
        submission: AppAgentPromptSubmission,
    ) -> astrcode_core::Result<ExecutionAccepted> {
        self.session_runtime
            .submit_prompt_for_agent(session_id, text, runtime, submission)
            .await
    }

    async fn interrupt_session(&self, session_id: &str) -> astrcode_core::Result<()> {
        self.session_runtime.interrupt_session(session_id).await
    }

    async fn compact_session(
        &self,
        session_id: &str,
        runtime: ResolvedRuntimeConfig,
        instructions: Option<String>,
    ) -> astrcode_core::Result<bool> {
        self.session_runtime
            .compact_session(session_id, runtime, instructions)
            .await
    }

    async fn session_transcript_snapshot(
        &self,
        session_id: &str,
    ) -> astrcode_core::Result<SessionTranscriptSnapshot> {
        let session_id = Self::session_id(session_id);
        let records = self.replay_history(&session_id, None).await?;
        Ok(SessionTranscriptSnapshot {
            cursor: records.last().map(|record| record.event_id.clone()),
            phase: self.session_phase(&session_id).await?,
            records,
        })
    }

    async fn conversation_snapshot(
        &self,
        session_id: &str,
    ) -> astrcode_core::Result<ConversationSnapshotFacts> {
        let transcript = self.session_transcript_snapshot(session_id).await?;
        Ok(project_conversation_snapshot(
            &transcript.records,
            transcript.phase,
        ))
    }

    async fn session_control_state(
        &self,
        session_id: &str,
    ) -> astrcode_core::Result<astrcode_host_session::SessionControlStateSnapshot> {
        self.session_catalog
            .session_control_state(&Self::session_id(session_id))
            .await
    }

    async fn active_task_snapshot(
        &self,
        session_id: &str,
        owner: &str,
    ) -> astrcode_core::Result<Option<astrcode_core::TaskSnapshot>> {
        self.session_catalog
            .active_task_snapshot(&Self::session_id(session_id), owner)
            .await
    }

    async fn session_mode_state(
        &self,
        session_id: &str,
    ) -> astrcode_core::Result<astrcode_host_session::SessionModeState> {
        self.session_catalog
            .session_mode_state(&Self::session_id(session_id))
            .await
    }

    async fn switch_mode(
        &self,
        session_id: &str,
        from: astrcode_core::ModeId,
        to: astrcode_core::ModeId,
    ) -> astrcode_core::Result<StoredEvent> {
        self.session_runtime.switch_mode(session_id, from, to).await
    }

    async fn session_child_nodes(
        &self,
        session_id: &str,
    ) -> astrcode_core::Result<Vec<astrcode_core::ChildSessionNode>> {
        self.session_catalog
            .session_child_nodes(&Self::session_id(session_id))
            .await
    }

    async fn session_stored_events(
        &self,
        session_id: &str,
    ) -> astrcode_core::Result<Vec<StoredEvent>> {
        self.session_catalog
            .stored_events(&Self::session_id(session_id))
            .await
    }

    async fn durable_subrun_status_snapshot(
        &self,
        parent_session_id: &str,
        requested_subrun_id: &str,
    ) -> astrcode_core::Result<Option<DurableSubRunStatusSummary>> {
        self.durable_subrun_status_summary(parent_session_id, requested_subrun_id)
            .await
    }

    async fn session_replay(
        &self,
        session_id: &str,
        last_event_id: Option<&str>,
    ) -> astrcode_core::Result<SessionReplay> {
        let session_id = Self::session_id(session_id);
        let state = self.session_catalog.session_state(&session_id).await?;
        Ok(SessionReplay {
            history: self.replay_history(&session_id, last_event_id).await?,
            receiver: state.broadcaster.subscribe(),
            live_receiver: state.subscribe_live(),
        })
    }

    async fn conversation_stream_replay(
        &self,
        session_id: &str,
        last_event_id: Option<&str>,
    ) -> astrcode_core::Result<ConversationStreamReplayFacts> {
        let session_id = Self::session_id(session_id);
        let replay = self
            .session_catalog
            .conversation_stream_replay(&session_id, last_event_id)
            .await?;
        Ok(ConversationStreamReplayFacts {
            cursor: replay.cursor,
            phase: self.session_phase(&session_id).await?,
            replay_frames: build_conversation_replay_frames(&replay.seed_records, &replay.history),
            replay_history: replay.history,
            seed_records: replay.seed_records,
        })
    }
}

#[async_trait]
impl AgentSessionPort for ServerSessionBridge {
    async fn create_child_session(
        &self,
        working_dir: &str,
        parent_session_id: &str,
    ) -> astrcode_core::Result<SessionMeta> {
        self.session_catalog
            .create_child_session(
                working_dir,
                normalize_external_session_id(parent_session_id),
                None,
            )
            .await
    }

    async fn submit_prompt_for_agent_with_submission(
        &self,
        session_id: &str,
        text: String,
        runtime: ResolvedRuntimeConfig,
        submission: AppAgentPromptSubmission,
    ) -> astrcode_core::Result<ExecutionAccepted> {
        self.session_runtime
            .submit_prompt_for_agent_with_submission(session_id, text, runtime, submission)
            .await
    }

    async fn try_submit_prompt_for_agent_with_turn_id(
        &self,
        session_id: &str,
        turn_id: TurnId,
        text: String,
        runtime: ResolvedRuntimeConfig,
        submission: AppAgentPromptSubmission,
    ) -> astrcode_core::Result<Option<ExecutionAccepted>> {
        self.session_runtime
            .try_submit_prompt_for_agent_with_turn_id(
                session_id, turn_id, text, runtime, submission,
            )
            .await
    }

    async fn submit_queued_inputs_for_agent_with_turn_id(
        &self,
        session_id: &str,
        turn_id: TurnId,
        queued_inputs: Vec<String>,
        runtime: ResolvedRuntimeConfig,
        submission: AppAgentPromptSubmission,
    ) -> astrcode_core::Result<Option<ExecutionAccepted>> {
        self.session_runtime
            .submit_queued_inputs_for_agent_with_turn_id(
                session_id,
                turn_id,
                queued_inputs,
                runtime,
                submission,
            )
            .await
    }

    async fn append_agent_input_queued(
        &self,
        session_id: &str,
        turn_id: &str,
        agent: AgentEventContext,
        payload: InputQueuedPayload,
    ) -> astrcode_core::Result<StoredEvent> {
        self.session_catalog
            .append_agent_input_queued(&Self::session_id(session_id), turn_id, agent, payload)
            .await
    }

    async fn append_agent_input_discarded(
        &self,
        session_id: &str,
        turn_id: &str,
        agent: AgentEventContext,
        payload: InputDiscardedPayload,
    ) -> astrcode_core::Result<StoredEvent> {
        self.session_catalog
            .append_agent_input_discarded(&Self::session_id(session_id), turn_id, agent, payload)
            .await
    }

    async fn append_agent_input_batch_started(
        &self,
        session_id: &str,
        turn_id: &str,
        agent: AgentEventContext,
        payload: InputBatchStartedPayload,
    ) -> astrcode_core::Result<StoredEvent> {
        self.session_catalog
            .append_agent_input_batch_started(
                &Self::session_id(session_id),
                turn_id,
                agent,
                payload,
            )
            .await
    }

    async fn append_agent_input_batch_acked(
        &self,
        session_id: &str,
        turn_id: &str,
        agent: AgentEventContext,
        payload: InputBatchAckedPayload,
    ) -> astrcode_core::Result<StoredEvent> {
        self.session_catalog
            .append_agent_input_batch_acked(&Self::session_id(session_id), turn_id, agent, payload)
            .await
    }

    async fn append_child_session_notification(
        &self,
        session_id: &str,
        turn_id: &str,
        agent: AgentEventContext,
        notification: astrcode_core::ChildSessionNotification,
    ) -> astrcode_core::Result<StoredEvent> {
        self.session_catalog
            .append_child_session_notification(
                &Self::session_id(session_id),
                turn_id,
                agent,
                notification,
            )
            .await
    }

    async fn append_agent_collaboration_fact(
        &self,
        session_id: &str,
        turn_id: &str,
        agent: AgentEventContext,
        fact: AgentCollaborationFact,
    ) -> astrcode_core::Result<StoredEvent> {
        self.session_catalog
            .append_agent_collaboration_fact(&Self::session_id(session_id), turn_id, agent, fact)
            .await
    }

    async fn pending_delivery_ids_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> astrcode_core::Result<Vec<String>> {
        self.session_catalog
            .pending_delivery_ids_for_agent(&Self::session_id(session_id), agent_id)
            .await
    }

    async fn recoverable_parent_deliveries(
        &self,
        parent_session_id: &str,
    ) -> astrcode_core::Result<Vec<RecoverableParentDelivery>> {
        let stored_events = self
            .session_catalog
            .stored_events(&Self::session_id(parent_session_id))
            .await?;
        Ok(recoverable_parent_deliveries(&stored_events))
    }

    async fn observe_agent_session(
        &self,
        open_session_id: &str,
        target_agent_id: &str,
        lifecycle_status: AgentLifecycleStatus,
    ) -> astrcode_core::Result<SessionObserveSnapshot> {
        self.session_runtime
            .observe_agent_session(open_session_id, target_agent_id, lifecycle_status)
            .await
    }

    async fn project_turn_outcome(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> astrcode_core::Result<SessionTurnOutcomeSummary> {
        let outcome = self
            .session_catalog
            .project_turn_outcome(&Self::session_id(session_id), turn_id)
            .await?;
        Ok(projected_turn_outcome_summary(outcome))
    }

    async fn wait_for_turn_terminal_snapshot(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> astrcode_core::Result<SessionTurnTerminalState> {
        let snapshot = self
            .session_catalog
            .wait_for_turn_terminal_snapshot(&Self::session_id(session_id), turn_id)
            .await?;
        Ok(SessionTurnTerminalState {
            phase: snapshot.phase,
            projection: snapshot.projection,
            events: snapshot.events,
        })
    }
}

fn projected_turn_outcome_summary(value: ProjectedTurnOutcome) -> SessionTurnOutcomeSummary {
    match value {
        ProjectedTurnOutcome::Completed { summary } => SessionTurnOutcomeSummary {
            outcome: astrcode_core::AgentTurnOutcome::Completed,
            summary,
            technical_message: String::new(),
        },
        ProjectedTurnOutcome::Cancelled { summary } => SessionTurnOutcomeSummary {
            outcome: astrcode_core::AgentTurnOutcome::Cancelled,
            summary,
            technical_message: String::new(),
        },
        ProjectedTurnOutcome::Failed {
            summary,
            technical_message,
        } => SessionTurnOutcomeSummary {
            outcome: astrcode_core::AgentTurnOutcome::Failed,
            summary,
            technical_message,
        },
    }
}

#[derive(Debug, Clone)]
struct DurableSubRunProjection {
    handle: SubRunHandle,
    tool_call_id: Option<String>,
    result: Option<SubRunResult>,
    step_count: Option<u32>,
    estimated_tokens: Option<u64>,
    resolved_overrides: Option<ResolvedSubagentContextOverrides>,
}

fn project_durable_subrun_status_summary(
    parent_session_id: &str,
    child_session_id: &str,
    requested_subrun_id: &str,
    stored_events: &[StoredEvent],
) -> Option<DurableSubRunStatusSummary> {
    let mut projection: Option<DurableSubRunProjection> = None;

    for stored in stored_events {
        let agent = &stored.event.agent;
        if !matches_requested_subrun(agent, requested_subrun_id) {
            continue;
        }

        match &stored.event.payload {
            StorageEventPayload::SubRunStarted {
                tool_call_id,
                resolved_overrides,
                resolved_limits,
                ..
            } => {
                projection = Some(DurableSubRunProjection {
                    handle: build_subrun_handle(
                        parent_session_id,
                        child_session_id,
                        requested_subrun_id,
                        agent,
                        AgentLifecycleStatus::Running,
                        None,
                        resolved_limits.clone(),
                    ),
                    tool_call_id: tool_call_id.clone(),
                    result: None,
                    step_count: None,
                    estimated_tokens: None,
                    resolved_overrides: Some(resolved_overrides.clone()),
                });
            },
            StorageEventPayload::SubRunFinished {
                tool_call_id,
                result,
                step_count,
                estimated_tokens,
                ..
            } => {
                let entry = projection.get_or_insert_with(|| DurableSubRunProjection {
                    handle: build_subrun_handle(
                        parent_session_id,
                        child_session_id,
                        requested_subrun_id,
                        agent,
                        result.status().lifecycle(),
                        result.status().last_turn_outcome(),
                        ResolvedExecutionLimitsSnapshot,
                    ),
                    tool_call_id: None,
                    result: None,
                    step_count: None,
                    estimated_tokens: None,
                    resolved_overrides: None,
                });
                entry.tool_call_id = tool_call_id.clone().or_else(|| entry.tool_call_id.clone());
                entry.handle.lifecycle = result.status().lifecycle();
                entry.handle.last_turn_outcome = result.status().last_turn_outcome();
                entry.result = Some(result.clone());
                entry.step_count = Some(*step_count);
                entry.estimated_tokens = Some(*estimated_tokens);
            },
            _ => {},
        }
    }

    projection.map(|projection| DurableSubRunStatusSummary {
        sub_run_id: projection.handle.sub_run_id.to_string(),
        tool_call_id: projection.tool_call_id,
        agent_id: projection.handle.agent_id.to_string(),
        agent_profile: projection.handle.agent_profile,
        session_id: projection.handle.session_id.to_string(),
        child_session_id: projection.handle.child_session_id.map(|id| id.to_string()),
        depth: projection.handle.depth,
        parent_agent_id: projection.handle.parent_agent_id.map(|id| id.to_string()),
        parent_sub_run_id: projection.handle.parent_sub_run_id.map(|id| id.to_string()),
        storage_mode: projection.handle.storage_mode,
        lifecycle: projection.handle.lifecycle,
        last_turn_outcome: projection.handle.last_turn_outcome,
        result: projection.result,
        step_count: projection.step_count,
        estimated_tokens: projection.estimated_tokens,
        resolved_overrides: projection.resolved_overrides,
        resolved_limits: projection.handle.resolved_limits,
    })
}

fn build_subrun_handle(
    parent_session_id: &str,
    child_session_id: &str,
    requested_subrun_id: &str,
    agent: &AgentEventContext,
    lifecycle: AgentLifecycleStatus,
    last_turn_outcome: Option<astrcode_core::AgentTurnOutcome>,
    resolved_limits: ResolvedExecutionLimitsSnapshot,
) -> SubRunHandle {
    SubRunHandle {
        sub_run_id: agent
            .sub_run_id
            .clone()
            .unwrap_or_else(|| requested_subrun_id.to_string().into()),
        agent_id: agent
            .agent_id
            .clone()
            .unwrap_or_else(|| requested_subrun_id.to_string().into()),
        session_id: parent_session_id.to_string().into(),
        child_session_id: Some(
            agent
                .child_session_id
                .clone()
                .unwrap_or_else(|| child_session_id.to_string().into()),
        ),
        depth: 1,
        parent_turn_id: agent.parent_turn_id.clone().unwrap_or_default(),
        parent_agent_id: None,
        parent_sub_run_id: agent.parent_sub_run_id.clone(),
        lineage_kind: astrcode_core::ChildSessionLineageKind::Spawn,
        agent_profile: agent
            .agent_profile
            .clone()
            .unwrap_or_else(|| "unknown".to_string()),
        storage_mode: agent
            .storage_mode
            .unwrap_or(SubRunStorageMode::IndependentSession),
        lifecycle,
        last_turn_outcome,
        resolved_limits,
        delegation: None,
    }
}

fn matches_requested_subrun(agent: &AgentEventContext, requested_subrun_id: &str) -> bool {
    if agent.invocation_kind != Some(InvocationKind::SubRun) {
        return false;
    }

    agent.sub_run_id.as_deref() == Some(requested_subrun_id)
        || agent.agent_id.as_deref() == Some(requested_subrun_id)
}

pub(crate) fn recoverable_parent_deliveries(
    events: &[StoredEvent],
) -> Vec<RecoverableParentDelivery> {
    let projection_index = replay_input_queue_projection_index(events);
    let mut recoverable_by_agent = HashMap::<String, HashSet<String>>::new();
    for (agent_id, projection) in projection_index {
        let active_ids = projection
            .active_delivery_ids
            .into_iter()
            .collect::<HashSet<_>>();
        let recoverable = projection
            .pending_delivery_ids
            .into_iter()
            .filter(|delivery_id| !active_ids.contains(delivery_id))
            .map(|delivery_id| delivery_id.to_string())
            .collect::<HashSet<_>>();
        if !recoverable.is_empty() {
            recoverable_by_agent.insert(agent_id, recoverable);
        }
    }

    let queued_at_by_delivery = events
        .iter()
        .filter_map(|stored| match &stored.event.payload {
            StorageEventPayload::AgentInputQueued { payload } => Some((
                payload.envelope.delivery_id.clone(),
                payload.envelope.queued_at,
            )),
            _ => None,
        })
        .collect::<HashMap<_, _>>();

    let mut recovered = Vec::new();
    let mut seen = HashSet::new();
    for stored in events {
        let StorageEventPayload::ChildSessionNotification { notification, .. } =
            &stored.event.payload
        else {
            continue;
        };
        let Some(parent_agent_id) = notification.child_ref.parent_agent_id() else {
            continue;
        };
        let Some(recoverable_ids) = recoverable_by_agent.get(parent_agent_id.as_str()) else {
            continue;
        };
        if !recoverable_ids.contains(notification.notification_id.as_str()) {
            continue;
        }
        if !seen.insert(notification.notification_id.clone()) {
            continue;
        }
        let Some(parent_turn_id) = stored.event.turn_id().map(ToString::to_string) else {
            continue;
        };
        recovered.push(RecoverableParentDelivery {
            delivery_id: notification.notification_id.to_string(),
            parent_session_id: notification.child_ref.session_id().to_string(),
            parent_turn_id,
            queued_at_ms: queued_at_by_delivery
                .get(&notification.notification_id)
                .map(|queued_at| queued_at.timestamp_millis())
                .unwrap_or_default(),
            notification: notification.clone(),
        });
    }

    recovered
}

fn replay_input_queue_projection_index(
    events: &[StoredEvent],
) -> HashMap<String, InputQueueProjection> {
    let mut index = HashMap::new();
    for stored in events {
        apply_input_queue_event_to_index(&mut index, stored);
    }
    index
}

fn apply_input_queue_event_to_index(
    index: &mut HashMap<String, InputQueueProjection>,
    stored: &StoredEvent,
) {
    let Some(target_agent_id) = input_queue_projection_target_agent_id(&stored.event.payload)
    else {
        return;
    };
    let projection = index.entry(target_agent_id.to_string()).or_default();
    apply_input_queue_event_for_agent(projection, stored, target_agent_id);
}

fn input_queue_projection_target_agent_id(payload: &StorageEventPayload) -> Option<&str> {
    match payload {
        StorageEventPayload::AgentInputQueued { payload } => Some(&payload.envelope.to_agent_id),
        StorageEventPayload::AgentInputBatchStarted { payload } => Some(&payload.target_agent_id),
        StorageEventPayload::AgentInputBatchAcked { payload } => Some(&payload.target_agent_id),
        StorageEventPayload::AgentInputDiscarded { payload } => Some(&payload.target_agent_id),
        _ => None,
    }
}

fn apply_input_queue_event_for_agent(
    projection: &mut InputQueueProjection,
    stored: &StoredEvent,
    target_agent_id: &str,
) {
    match &stored.event.payload {
        StorageEventPayload::AgentInputQueued { payload } => {
            if payload.envelope.to_agent_id != target_agent_id {
                return;
            }
            let delivery_id = &payload.envelope.delivery_id;
            if !projection.discarded_delivery_ids.contains(delivery_id)
                && !projection.pending_delivery_ids.contains(delivery_id)
            {
                projection.pending_delivery_ids.push(delivery_id.clone());
            }
        },
        StorageEventPayload::AgentInputBatchStarted { payload } => {
            if payload.target_agent_id != target_agent_id {
                return;
            }
            projection.active_batch_id = Some(payload.batch_id.clone());
            projection.active_delivery_ids = payload.delivery_ids.clone();
        },
        StorageEventPayload::AgentInputBatchAcked { payload } => {
            if payload.target_agent_id != target_agent_id {
                return;
            }
            let acked_set = payload.delivery_ids.iter().collect::<HashSet<_>>();
            projection.pending_delivery_ids.retain(|delivery_id| {
                !acked_set.contains(delivery_id)
                    && !projection.discarded_delivery_ids.contains(delivery_id)
            });
            if projection.active_batch_id.as_deref() == Some(payload.batch_id.as_str()) {
                projection.active_batch_id = None;
                projection.active_delivery_ids.clear();
            }
        },
        StorageEventPayload::AgentInputDiscarded { payload } => {
            if payload.target_agent_id != target_agent_id {
                return;
            }
            for delivery_id in &payload.delivery_ids {
                if !projection.discarded_delivery_ids.contains(delivery_id) {
                    projection.discarded_delivery_ids.push(delivery_id.clone());
                }
            }
            projection
                .pending_delivery_ids
                .retain(|delivery_id| !projection.discarded_delivery_ids.contains(delivery_id));
            let discarded_set = projection
                .discarded_delivery_ids
                .iter()
                .collect::<HashSet<_>>();
            if projection
                .active_delivery_ids
                .iter()
                .any(|delivery_id| discarded_set.contains(delivery_id))
            {
                projection.active_batch_id = None;
                projection.active_delivery_ids.clear();
            }
        },
        _ => {},
    }
}
