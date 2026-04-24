//! turn mutation owner 合同。
//!
//! `host-session` 最终拥有 submit / compact / interrupt 的 durable truth。
//! server 或 application 只能提供治理、workflow、skill invocation 的输入材料，
//! 不能拥有 turn lease、事件持久化或投影真相。

use std::sync::{Arc, Mutex as StdMutex};

use astrcode_core::{
    AgentEventContext, AstrError, CancelToken, ExecutionControl, Result, SessionId, StorageEvent,
    StorageEventPayload, StoredEvent, TurnId, TurnTerminalKind, UserMessageOrigin,
    generate_turn_id,
};
use astrcode_runtime_contract::{ExecutionAccepted, RuntimeTurnEvent};
use async_trait::async_trait;
use chrono::Utc;

use crate::{EventTranslator, SessionCatalog, SubmitTarget, state::checkpoint_if_compacted};

/// turn mutation 预处理来源。
///
/// Durable mutation owner 固定为 `host-session`；这里仅描述治理、workflow、skill
/// invocation 等准备材料当前由谁提供，避免把 bridge 误认为 owner。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnMutationPreparationOwner {
    HostSession,
    ExternalPreparation { owner: String },
}

impl TurnMutationPreparationOwner {
    pub fn external_preparation(owner: impl Into<String>) -> Self {
        Self::ExternalPreparation {
            owner: owner.into(),
        }
    }

    pub fn is_external_preparation(&self) -> bool {
        matches!(self, Self::ExternalPreparation { .. })
    }
}

/// submit / compact 进入 durable mutation 前的准备归属快照。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnMutationPreparation {
    pub governance: TurnMutationPreparationOwner,
    pub workflow: TurnMutationPreparationOwner,
    pub skill_invocation: TurnMutationPreparationOwner,
}

impl TurnMutationPreparation {
    pub fn host_session_owned() -> Self {
        Self {
            governance: TurnMutationPreparationOwner::HostSession,
            workflow: TurnMutationPreparationOwner::HostSession,
            skill_invocation: TurnMutationPreparationOwner::HostSession,
        }
    }

    pub fn external_preparation(owner: impl Into<String>) -> Self {
        let owner = owner.into();
        Self {
            governance: TurnMutationPreparationOwner::external_preparation(owner.clone()),
            workflow: TurnMutationPreparationOwner::external_preparation(owner.clone()),
            skill_invocation: TurnMutationPreparationOwner::external_preparation(owner),
        }
    }

    pub fn has_external_preparation(&self) -> bool {
        self.governance.is_external_preparation()
            || self.workflow.is_external_preparation()
            || self.skill_invocation.is_external_preparation()
    }
}

impl Default for TurnMutationPreparation {
    fn default() -> Self {
        Self::host_session_owned()
    }
}

/// Prompt submit 的 owner 输入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmitPromptMutationInput {
    pub requested_session_id: SessionId,
    pub requested_turn_id: Option<TurnId>,
    pub prompt_text: String,
    pub queued_inputs: Vec<String>,
    pub control: Option<ExecutionControl>,
    pub preparation: TurnMutationPreparation,
}

/// Submit 遇到 busy session 时的 owner 策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitTurnBusyPolicy {
    BranchOnBusy { max_branch_depth: usize },
    RejectOnBusy,
}

/// Prompt submit 接受摘要。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptAcceptedSummary {
    pub turn_id: TurnId,
    pub session_id: SessionId,
    pub branched_from_session_id: Option<String>,
    pub accepted_control: Option<ExecutionControl>,
}

impl PromptAcceptedSummary {
    pub fn from_execution_accepted(
        accepted: ExecutionAccepted,
        accepted_control: Option<ExecutionControl>,
    ) -> Self {
        Self {
            turn_id: accepted.turn_id,
            session_id: accepted.session_id,
            branched_from_session_id: accepted.branched_from_session_id,
            accepted_control,
        }
    }
}

/// 已被 `host-session` 接受的 prompt submit。
///
/// 持有 [`SubmitTarget`] 会同时持有 turn lease，因此后续执行 bridge 必须在 turn
/// 生命周期内保留该值，不能只复制摘要后丢弃 lease。
pub struct AcceptedSubmitPrompt {
    pub summary: PromptAcceptedSummary,
    pub target: SubmitTarget,
    pub live_user_input: Option<String>,
    pub queued_inputs: Vec<String>,
    pub preparation: TurnMutationPreparation,
}

/// 已进入 running 状态、并由 `host-session` 持有 lease/cancel owner 的 accepted turn。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BegunAcceptedTurn {
    pub summary: PromptAcceptedSummary,
    pub live_user_input: Option<String>,
    pub queued_inputs: Vec<String>,
    pub preparation: TurnMutationPreparation,
}

/// `agent-runtime` turn output 写入 `host-session` durable log 的输入。
pub struct RuntimeTurnPersistenceInput {
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub agent: AgentEventContext,
    pub runtime_events: Vec<RuntimeTurnEvent>,
}

/// 单个 runtime event 写入 durable log 的输入。
pub struct RuntimeTurnEventPersistenceInput {
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub agent: AgentEventContext,
    pub runtime_event: RuntimeTurnEvent,
}

impl RuntimeTurnPersistenceInput {
    pub fn from_accepted_prompt(
        accepted: &AcceptedSubmitPrompt,
        agent: AgentEventContext,
        runtime_events: Vec<RuntimeTurnEvent>,
    ) -> Self {
        Self {
            session_id: accepted.summary.session_id.clone(),
            turn_id: accepted.summary.turn_id.clone(),
            agent,
            runtime_events,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingManualCompactRequest {
    pub control: Option<ExecutionControl>,
    pub instructions: Option<String>,
    pub preparation: TurnMutationPreparation,
}

struct ActiveTurnMutationState {
    turn_id: TurnId,
    agent: AgentEventContext,
    cancel: CancelToken,
    _target: SubmitTarget,
}

#[derive(Default)]
pub(crate) struct TurnMutationState {
    active_turn: StdMutex<Option<ActiveTurnMutationState>>,
    pending_manual_compact: StdMutex<Option<PendingManualCompactRequest>>,
}

impl TurnMutationState {
    pub(crate) fn active_turn_id_snapshot(&self) -> Result<Option<TurnId>> {
        Ok(astrcode_core::support::lock_anyhow(
            &self.active_turn,
            "host-session active turn mutation",
        )?
        .as_ref()
        .map(|active| active.turn_id.clone()))
    }

    pub(crate) fn has_pending_manual_compact(&self) -> Result<bool> {
        Ok(astrcode_core::support::lock_anyhow(
            &self.pending_manual_compact,
            "host-session pending manual compact",
        )?
        .is_some())
    }
}

impl SessionCatalog {
    pub async fn accept_submit_prompt(
        &self,
        input: SubmitPromptMutationInput,
        busy_policy: SubmitTurnBusyPolicy,
    ) -> Result<Option<AcceptedSubmitPrompt>> {
        let SubmitPromptMutationInput {
            requested_session_id,
            requested_turn_id,
            prompt_text,
            queued_inputs,
            control,
            preparation,
        } = input;
        let live_user_input = normalize_prompt_text(prompt_text);
        let queued_inputs = normalize_queued_inputs(queued_inputs);
        if live_user_input.is_none() && queued_inputs.is_empty() {
            return Err(AstrError::Validation(
                "turn submission must include live user input or queued inputs".to_string(),
            ));
        }

        let requested_session_id = SessionId::from(requested_session_id.as_str().trim());
        let turn_id = requested_turn_id.unwrap_or_else(|| TurnId::from(generate_turn_id()));
        let target = match busy_policy {
            SubmitTurnBusyPolicy::BranchOnBusy { max_branch_depth } => Some(
                self.resolve_submit_target(
                    &requested_session_id,
                    turn_id.as_str(),
                    max_branch_depth,
                )
                .await?,
            ),
            SubmitTurnBusyPolicy::RejectOnBusy => {
                self.try_resolve_submit_target_without_branch(
                    &requested_session_id,
                    turn_id.as_str(),
                )
                .await?
            },
        };

        let Some(target) = target else {
            return Ok(None);
        };
        let summary = PromptAcceptedSummary {
            turn_id,
            session_id: target.session_id.clone(),
            branched_from_session_id: target.branched_from_session_id.clone(),
            accepted_control: control,
        };

        Ok(Some(AcceptedSubmitPrompt {
            summary,
            target,
            live_user_input,
            queued_inputs,
            preparation,
        }))
    }

    pub fn begin_accepted_turn(
        &self,
        accepted: AcceptedSubmitPrompt,
        agent: AgentEventContext,
        cancel: CancelToken,
    ) -> Result<BegunAcceptedTurn> {
        let AcceptedSubmitPrompt {
            summary,
            target,
            live_user_input,
            queued_inputs,
            preparation,
        } = accepted;
        let state = self.turn_mutation_state(&summary.session_id);
        let mut active_turn = astrcode_core::support::lock_anyhow(
            &state.active_turn,
            "host-session active turn mutation",
        )?;
        if active_turn.is_some() {
            return Err(AstrError::Validation(format!(
                "session '{}' already has an active turn",
                summary.session_id
            )));
        }
        *active_turn = Some(ActiveTurnMutationState {
            turn_id: summary.turn_id.clone(),
            agent,
            cancel,
            _target: target,
        });
        Ok(BegunAcceptedTurn {
            summary,
            live_user_input,
            queued_inputs,
            preparation,
        })
    }

    pub async fn persist_begun_turn_inputs(
        &self,
        begun: &BegunAcceptedTurn,
        agent: AgentEventContext,
    ) -> Result<Vec<StoredEvent>> {
        let mut storage_events = Vec::new();
        let now = Utc::now();
        for queued_input in &begun.queued_inputs {
            storage_events.push(user_message_storage_event(
                begun.summary.turn_id.as_str(),
                &agent,
                queued_input.clone(),
                UserMessageOrigin::QueuedInput,
                now,
            ));
        }
        if let Some(live_user_input) = &begun.live_user_input {
            storage_events.push(user_message_storage_event(
                begun.summary.turn_id.as_str(),
                &agent,
                live_user_input.clone(),
                UserMessageOrigin::User,
                now,
            ));
        }
        self.append_turn_storage_events(&begun.summary.session_id, storage_events)
            .await
    }

    pub async fn request_manual_compact(
        &self,
        input: CompactSessionMutationInput,
    ) -> Result<CompactSessionSummary> {
        if matches!(
            input
                .control
                .as_ref()
                .and_then(|control| control.manual_compact),
            Some(false)
        ) {
            return Err(AstrError::Validation(
                "manualCompact must be true for manual compact requests".to_string(),
            ));
        }

        self.ensure_session_exists(&input.session_id).await?;
        let state = self.turn_mutation_state(&input.session_id);
        let is_running = astrcode_core::support::lock_anyhow(
            &state.active_turn,
            "host-session active turn mutation",
        )?
        .is_some();
        if is_running {
            *astrcode_core::support::lock_anyhow(
                &state.pending_manual_compact,
                "host-session pending manual compact",
            )? = Some(PendingManualCompactRequest {
                control: input.control,
                instructions: input.instructions,
                preparation: input.preparation,
            });
            return Ok(CompactSessionSummary::manual_compact_accepted(true));
        }

        Ok(CompactSessionSummary::manual_compact_accepted(false))
    }

    pub fn complete_running_turn(
        &self,
        session_id: &SessionId,
        turn_id: &TurnId,
    ) -> Result<Option<PendingManualCompactRequest>> {
        let Some(state) = self.turn_mutations.get(session_id) else {
            return Ok(None);
        };
        let mut active_turn = astrcode_core::support::lock_anyhow(
            &state.active_turn,
            "host-session active turn mutation",
        )?;
        if active_turn
            .as_ref()
            .map(|active| &active.turn_id)
            .filter(|active_turn_id| *active_turn_id == turn_id)
            .is_none()
        {
            return Ok(None);
        }
        *active_turn = None;
        let pending_manual_compact = astrcode_core::support::lock_anyhow(
            &state.pending_manual_compact,
            "host-session pending manual compact",
        )?
        .take();
        Ok(pending_manual_compact)
    }

    pub async fn interrupt_running_turn(
        &self,
        input: InterruptSessionMutationInput,
    ) -> Result<InterruptSessionSummary> {
        let loaded = self.ensure_loaded_session(&input.session_id).await?;
        let Some(state) = self.turn_mutations.get(&input.session_id) else {
            return Ok(InterruptSessionSummary::not_running(input.session_id));
        };
        let active = astrcode_core::support::lock_anyhow(
            &state.active_turn,
            "host-session active turn mutation",
        )?
        .take();
        let Some(active) = active else {
            return Ok(InterruptSessionSummary::not_running(input.session_id));
        };
        active.cancel.cancel();

        let mut translator = EventTranslator::new(loaded.state.current_phase()?);
        let event = StorageEvent {
            turn_id: Some(active.turn_id.to_string()),
            agent: active.agent,
            payload: StorageEventPayload::TurnDone {
                timestamp: Utc::now(),
                terminal_kind: Some(TurnTerminalKind::Cancelled),
                reason: None,
            },
        };
        loaded
            .state
            .append_and_broadcast(&event, &mut translator)
            .await?;
        let pending_manual_compact = astrcode_core::support::lock_anyhow(
            &state.pending_manual_compact,
            "host-session pending manual compact",
        )?
        .take();
        Ok(InterruptSessionSummary {
            session_id: input.session_id,
            accepted: true,
            interrupted_turn_id: Some(active.turn_id),
            pending_manual_compact,
        })
    }

    fn turn_mutation_state(&self, session_id: &SessionId) -> Arc<TurnMutationState> {
        Arc::clone(
            self.turn_mutations
                .entry(session_id.clone())
                .or_insert_with(|| Arc::new(TurnMutationState::default()))
                .value(),
        )
    }

    pub async fn persist_runtime_turn_events(
        &self,
        input: RuntimeTurnPersistenceInput,
    ) -> Result<Vec<StoredEvent>> {
        let RuntimeTurnPersistenceInput {
            session_id,
            turn_id,
            agent,
            runtime_events,
        } = input;
        let storage_events = runtime_turn_storage_events(turn_id.as_str(), &agent, runtime_events);
        self.append_turn_storage_events(&session_id, storage_events)
            .await
    }

    pub async fn persist_runtime_turn_event(
        &self,
        input: RuntimeTurnEventPersistenceInput,
    ) -> Result<Vec<StoredEvent>> {
        let storage_events = runtime_turn_storage_events(
            input.turn_id.as_str(),
            &input.agent,
            vec![input.runtime_event],
        );
        if storage_events.is_empty() {
            return Ok(Vec::new());
        }
        self.append_turn_storage_events(&input.session_id, storage_events)
            .await
    }

    async fn append_turn_storage_events(
        &self,
        session_id: &SessionId,
        storage_events: Vec<StorageEvent>,
    ) -> Result<Vec<StoredEvent>> {
        let loaded = self.ensure_loaded_session(session_id).await?;
        let mut translator = EventTranslator::new(loaded.state.current_phase()?);
        let mut persisted_events = Vec::with_capacity(storage_events.len());
        for event in storage_events {
            persisted_events.push(
                loaded
                    .state
                    .append_and_broadcast(&event, &mut translator)
                    .await?,
            );
        }
        checkpoint_if_compacted(
            &self.event_store,
            session_id,
            &loaded.state,
            &persisted_events,
        )
        .await;
        Ok(persisted_events)
    }
}

fn normalize_prompt_text(text: String) -> Option<String> {
    let text = text.trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn normalize_queued_inputs(inputs: Vec<String>) -> Vec<String> {
    inputs
        .into_iter()
        .filter_map(normalize_prompt_text)
        .collect()
}

fn runtime_turn_storage_events(
    turn_id: &str,
    agent: &AgentEventContext,
    runtime_events: Vec<RuntimeTurnEvent>,
) -> Vec<StorageEvent> {
    let mut events = Vec::new();
    for runtime_event in runtime_events {
        match runtime_event {
            RuntimeTurnEvent::StorageEvent { event } => {
                events.push(*event);
            },
            RuntimeTurnEvent::AssistantFinal {
                content, reasoning, ..
            } => {
                events.push(StorageEvent {
                    turn_id: Some(turn_id.to_string()),
                    agent: agent.clone(),
                    payload: StorageEventPayload::AssistantFinal {
                        content,
                        reasoning_content: reasoning.as_ref().map(|value| value.content.clone()),
                        reasoning_signature: reasoning.and_then(|value| value.signature),
                        step_index: None,
                        timestamp: Some(Utc::now()),
                    },
                });
            },
            RuntimeTurnEvent::TurnCompleted { terminal_kind, .. } => {
                events.push(StorageEvent {
                    turn_id: Some(turn_id.to_string()),
                    agent: agent.clone(),
                    payload: StorageEventPayload::TurnDone {
                        timestamp: Utc::now(),
                        terminal_kind: Some(terminal_kind),
                        reason: None,
                    },
                });
            },
            RuntimeTurnEvent::TurnErrored { message, .. } => {
                events.push(StorageEvent {
                    turn_id: Some(turn_id.to_string()),
                    agent: agent.clone(),
                    payload: StorageEventPayload::Error {
                        message,
                        timestamp: Some(Utc::now()),
                    },
                });
            },
            RuntimeTurnEvent::HookPromptAugmented { content, .. } => {
                events.push(user_message_storage_event(
                    turn_id,
                    agent,
                    content,
                    UserMessageOrigin::ReactivationPrompt,
                    Utc::now(),
                ));
            },
            RuntimeTurnEvent::TurnStarted { .. }
            | RuntimeTurnEvent::ProviderStream { .. }
            | RuntimeTurnEvent::ToolUseRequested { .. }
            | RuntimeTurnEvent::ToolCallStarted { .. }
            | RuntimeTurnEvent::ToolResultReady { .. }
            | RuntimeTurnEvent::HookDispatched { .. }
            | RuntimeTurnEvent::StepContinued { .. } => {},
        }
    }
    events
}

fn user_message_storage_event(
    turn_id: &str,
    agent: &AgentEventContext,
    content: String,
    origin: UserMessageOrigin,
    timestamp: chrono::DateTime<Utc>,
) -> StorageEvent {
    StorageEvent {
        turn_id: Some(turn_id.to_string()),
        agent: agent.clone(),
        payload: StorageEventPayload::UserMessage {
            content,
            timestamp,
            origin,
        },
    }
}

/// Manual compact 的 owner 输入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactSessionMutationInput {
    pub session_id: SessionId,
    pub control: Option<ExecutionControl>,
    pub instructions: Option<String>,
    pub preparation: TurnMutationPreparation,
}

/// Manual compact 接受摘要。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactSessionSummary {
    pub accepted: bool,
    pub deferred: bool,
    pub message: String,
}

impl CompactSessionSummary {
    pub fn manual_compact_accepted(deferred: bool) -> Self {
        Self {
            accepted: true,
            deferred,
            message: if deferred {
                "手动 compact 已登记，会在当前 turn 完成后执行。".to_string()
            } else {
                "手动 compact 已执行。".to_string()
            },
        }
    }
}

/// Interrupt 的 owner 输入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterruptSessionMutationInput {
    pub session_id: SessionId,
}

/// Interrupt 接受摘要。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterruptSessionSummary {
    pub session_id: SessionId,
    pub accepted: bool,
    pub interrupted_turn_id: Option<TurnId>,
    pub pending_manual_compact: Option<PendingManualCompactRequest>,
}

impl InterruptSessionSummary {
    pub fn accepted(session_id: SessionId, interrupted_turn_id: Option<TurnId>) -> Self {
        Self {
            session_id,
            accepted: true,
            interrupted_turn_id,
            pending_manual_compact: None,
        }
    }

    pub fn not_running(session_id: SessionId) -> Self {
        Self {
            session_id,
            accepted: false,
            interrupted_turn_id: None,
            pending_manual_compact: None,
        }
    }
}

/// `host-session` 暴露给 server 的 turn mutation facade。
#[async_trait]
pub trait TurnMutationFacade: Send + Sync {
    async fn submit_prompt(
        &self,
        input: SubmitPromptMutationInput,
    ) -> Result<PromptAcceptedSummary>;

    async fn compact_session(
        &self,
        input: CompactSessionMutationInput,
    ) -> Result<CompactSessionSummary>;

    async fn interrupt_session(
        &self,
        input: InterruptSessionMutationInput,
    ) -> Result<InterruptSessionSummary>;
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
    };

    use astrcode_agent_runtime::TurnIdentity;
    use astrcode_core::{
        AgentId, DeleteProjectResult, Phase, SessionMeta, SessionTurnAcquireResult,
        SessionTurnBusy, SessionTurnLease, StorageEvent, StorageEventPayload, StoredEvent,
        TurnTerminalKind,
    };
    use astrcode_runtime_contract::ExecutionAccepted;
    use async_trait::async_trait;
    use chrono::Utc;

    use super::*;
    use crate::{EventStore, catalog::display_name_from_working_dir};

    #[derive(Debug)]
    struct TestLease;

    impl SessionTurnLease for TestLease {}

    #[derive(Default)]
    struct TurnMutationEventStore {
        sessions: Mutex<HashMap<SessionId, (PathBuf, Vec<StoredEvent>)>>,
        busy_sessions: Mutex<HashMap<SessionId, String>>,
    }

    impl TurnMutationEventStore {
        fn mark_busy(&self, session_id: &SessionId, active_turn_id: impl Into<String>) {
            self.busy_sessions
                .lock()
                .expect("busy sessions lock poisoned")
                .insert(session_id.clone(), active_turn_id.into());
        }
    }

    #[async_trait]
    impl EventStore for TurnMutationEventStore {
        async fn ensure_session(&self, session_id: &SessionId, working_dir: &Path) -> Result<()> {
            self.sessions
                .lock()
                .expect("sessions lock poisoned")
                .entry(session_id.clone())
                .or_insert_with(|| (working_dir.to_path_buf(), Vec::new()));
            Ok(())
        }

        async fn append(
            &self,
            session_id: &SessionId,
            event: &StorageEvent,
        ) -> Result<StoredEvent> {
            let mut sessions = self.sessions.lock().expect("sessions lock poisoned");
            let (_, events) = sessions
                .get_mut(session_id)
                .ok_or_else(|| AstrError::SessionNotFound(session_id.to_string()))?;
            let stored = StoredEvent {
                storage_seq: events.len() as u64 + 1,
                event: event.clone(),
            };
            events.push(stored.clone());
            Ok(stored)
        }

        async fn replay(&self, session_id: &SessionId) -> Result<Vec<StoredEvent>> {
            self.sessions
                .lock()
                .expect("sessions lock poisoned")
                .get(session_id)
                .map(|(_, events)| events.clone())
                .ok_or_else(|| AstrError::SessionNotFound(session_id.to_string()))
        }

        async fn try_acquire_turn(
            &self,
            session_id: &SessionId,
            _turn_id: &str,
        ) -> Result<SessionTurnAcquireResult> {
            if let Some(active_turn_id) = self
                .busy_sessions
                .lock()
                .expect("busy sessions lock poisoned")
                .get(session_id)
                .cloned()
            {
                return Ok(SessionTurnAcquireResult::Busy(SessionTurnBusy {
                    turn_id: active_turn_id,
                    owner_pid: 42,
                    acquired_at: Utc::now(),
                }));
            }

            if !self
                .sessions
                .lock()
                .expect("sessions lock poisoned")
                .contains_key(session_id)
            {
                return Err(AstrError::SessionNotFound(session_id.to_string()));
            }

            Ok(SessionTurnAcquireResult::Acquired(Box::new(TestLease)))
        }

        async fn list_sessions(&self) -> Result<Vec<SessionId>> {
            Ok(self
                .sessions
                .lock()
                .expect("sessions lock poisoned")
                .keys()
                .cloned()
                .collect())
        }

        async fn list_session_metas(&self) -> Result<Vec<SessionMeta>> {
            let sessions = self.sessions.lock().expect("sessions lock poisoned");
            Ok(sessions
                .iter()
                .map(|(session_id, (working_dir, events))| {
                    let session_start = events.iter().find_map(|stored| {
                        if let StorageEventPayload::SessionStart {
                            timestamp,
                            working_dir,
                            parent_session_id,
                            parent_storage_seq,
                            ..
                        } = &stored.event.payload
                        {
                            return Some((
                                *timestamp,
                                working_dir.clone(),
                                parent_session_id.clone(),
                                *parent_storage_seq,
                            ));
                        }
                        None
                    });
                    let (created_at, stored_working_dir, parent_session_id, parent_storage_seq) =
                        session_start.unwrap_or_else(|| {
                            (Utc::now(), working_dir.display().to_string(), None, None)
                        });
                    SessionMeta {
                        session_id: session_id.to_string(),
                        working_dir: stored_working_dir.clone(),
                        display_name: display_name_from_working_dir(Path::new(&stored_working_dir)),
                        title: "New Session".to_string(),
                        created_at,
                        updated_at: created_at,
                        parent_session_id,
                        parent_storage_seq,
                        phase: Phase::Idle,
                    }
                })
                .collect())
        }

        async fn delete_session(&self, session_id: &SessionId) -> Result<()> {
            self.sessions
                .lock()
                .expect("sessions lock poisoned")
                .remove(session_id);
            Ok(())
        }

        async fn delete_sessions_by_working_dir(
            &self,
            working_dir: &str,
        ) -> Result<DeleteProjectResult> {
            let mut sessions = self.sessions.lock().expect("sessions lock poisoned");
            let before = sessions.len();
            sessions.retain(|_, (path, _)| path.display().to_string() != working_dir);
            Ok(DeleteProjectResult {
                success_count: before.saturating_sub(sessions.len()),
                failed_session_ids: Vec::new(),
            })
        }
    }

    fn submit_input(
        session_id: SessionId,
        turn_id: impl Into<String>,
        prompt_text: impl Into<String>,
    ) -> SubmitPromptMutationInput {
        SubmitPromptMutationInput {
            requested_session_id: session_id,
            requested_turn_id: Some(TurnId::from(turn_id.into())),
            prompt_text: prompt_text.into(),
            queued_inputs: Vec::new(),
            control: None,
            preparation: TurnMutationPreparation::external_preparation("application"),
        }
    }

    fn compact_input(session_id: SessionId) -> CompactSessionMutationInput {
        CompactSessionMutationInput {
            session_id,
            control: Some(ExecutionControl {
                manual_compact: Some(true),
            }),
            instructions: Some("keep latest facts".to_string()),
            preparation: TurnMutationPreparation::external_preparation("application"),
        }
    }

    #[test]
    fn preparation_marks_external_preparation_without_changing_owner_contract() {
        let preparation = TurnMutationPreparation::external_preparation("application");

        assert!(preparation.has_external_preparation());
        assert_eq!(
            preparation.governance,
            TurnMutationPreparationOwner::ExternalPreparation {
                owner: "application".to_string()
            }
        );
    }

    #[test]
    fn prompt_summary_preserves_accepted_control_and_branch_source() {
        let accepted = ExecutionAccepted {
            session_id: SessionId::from("session-branch"),
            turn_id: TurnId::from("turn-1"),
            agent_id: Some(AgentId::from("agent-root")),
            branched_from_session_id: Some("session-source".to_string()),
        };
        let control = Some(ExecutionControl {
            manual_compact: Some(false),
        });

        let summary = PromptAcceptedSummary::from_execution_accepted(accepted, control.clone());

        assert_eq!(summary.session_id.as_str(), "session-branch");
        assert_eq!(summary.turn_id.as_str(), "turn-1");
        assert_eq!(
            summary.branched_from_session_id,
            Some("session-source".to_string())
        );
        assert_eq!(summary.accepted_control, control);
    }

    #[test]
    fn compact_summary_keeps_existing_response_messages() {
        let immediate = CompactSessionSummary::manual_compact_accepted(false);
        let deferred = CompactSessionSummary::manual_compact_accepted(true);

        assert!(immediate.accepted);
        assert!(!immediate.deferred);
        assert_eq!(immediate.message, "手动 compact 已执行。");
        assert!(deferred.deferred);
        assert_eq!(
            deferred.message,
            "手动 compact 已登记，会在当前 turn 完成后执行。"
        );
    }

    #[test]
    fn interrupt_summary_accepts_optional_interrupted_turn() {
        let summary = InterruptSessionSummary::accepted(
            SessionId::from("session-1"),
            Some(TurnId::from("turn-1")),
        );

        assert!(summary.accepted);
        assert_eq!(summary.session_id.as_str(), "session-1");
        assert_eq!(
            summary
                .interrupted_turn_id
                .as_ref()
                .map(|turn_id| turn_id.as_str()),
            Some("turn-1")
        );
    }

    #[tokio::test]
    async fn accept_submit_prompt_rejects_empty_input() {
        let catalog = SessionCatalog::new(Arc::new(TurnMutationEventStore::default()));

        let result = catalog
            .accept_submit_prompt(
                submit_input(SessionId::from("session-1"), "turn-1", "   "),
                SubmitTurnBusyPolicy::RejectOnBusy,
            )
            .await;

        assert!(matches!(
            result,
            Err(AstrError::Validation(message))
                if message == "turn submission must include live user input or queued inputs"
        ));
    }

    #[tokio::test]
    async fn accept_submit_prompt_keeps_accepted_response_shape() {
        let catalog = SessionCatalog::new(Arc::new(TurnMutationEventStore::default()));
        let meta = catalog
            .create_session("D:/workspace/project")
            .await
            .expect("session should be created");
        let session_id = SessionId::from(meta.session_id);
        let control = Some(ExecutionControl {
            manual_compact: Some(true),
        });
        let mut input = submit_input(session_id.clone(), "turn-shape", "  hello  ");
        input.queued_inputs = vec!["  queued  ".to_string(), " ".to_string()];
        input.control = control.clone();

        let accepted = catalog
            .accept_submit_prompt(input, SubmitTurnBusyPolicy::RejectOnBusy)
            .await
            .expect("submit should be accepted")
            .expect("reject-on-busy should accept idle session");

        assert_eq!(accepted.summary.session_id, session_id);
        assert_eq!(accepted.summary.turn_id.as_str(), "turn-shape");
        assert_eq!(accepted.summary.branched_from_session_id, None);
        assert_eq!(accepted.summary.accepted_control, control);
        assert_eq!(accepted.target.session_id, accepted.summary.session_id);
        assert_eq!(accepted.live_user_input.as_deref(), Some("hello"));
        assert_eq!(accepted.queued_inputs, vec!["queued".to_string()]);
        assert!(accepted.preparation.has_external_preparation());
    }

    #[tokio::test]
    async fn accept_submit_prompt_returns_none_when_reject_on_busy() {
        let store = Arc::new(TurnMutationEventStore::default());
        let catalog = SessionCatalog::new(store.clone());
        let meta = catalog
            .create_session("D:/workspace/project")
            .await
            .expect("session should be created");
        let session_id = SessionId::from(meta.session_id);
        store.mark_busy(&session_id, "turn-active");

        let accepted = catalog
            .accept_submit_prompt(
                submit_input(session_id, "turn-rejected", "hello"),
                SubmitTurnBusyPolicy::RejectOnBusy,
            )
            .await
            .expect("busy rejection should be non-error");

        assert!(accepted.is_none());
    }

    #[tokio::test]
    async fn accept_submit_prompt_branches_when_source_is_busy() {
        let store = Arc::new(TurnMutationEventStore::default());
        let catalog = SessionCatalog::new(store.clone());
        let meta = catalog
            .create_session("D:/workspace/project")
            .await
            .expect("session should be created");
        let source_session_id = SessionId::from(meta.session_id);
        store.mark_busy(&source_session_id, "turn-active");

        let accepted = catalog
            .accept_submit_prompt(
                submit_input(source_session_id.clone(), "turn-branch", "hello"),
                SubmitTurnBusyPolicy::BranchOnBusy {
                    max_branch_depth: 2,
                },
            )
            .await
            .expect("branch submit should be accepted")
            .expect("branch-on-busy should create a target");

        assert_ne!(accepted.summary.session_id, source_session_id);
        assert_eq!(
            accepted.summary.branched_from_session_id,
            Some(source_session_id.to_string())
        );
        assert_eq!(accepted.summary.turn_id.as_str(), "turn-branch");
        assert_eq!(accepted.target.session_id, accepted.summary.session_id);
    }

    #[tokio::test]
    async fn persist_runtime_turn_events_writes_and_recovers_read_model() {
        let store = Arc::new(TurnMutationEventStore::default());
        let catalog = SessionCatalog::new(store.clone());
        let meta = catalog
            .create_session("D:/workspace/project")
            .await
            .expect("session should be created");
        let session_id = SessionId::from(meta.session_id);
        let mut input = submit_input(session_id.clone(), "turn-runtime", " live prompt ");
        input.queued_inputs = vec![" queued prompt ".to_string()];
        let accepted = catalog
            .accept_submit_prompt(input, SubmitTurnBusyPolicy::RejectOnBusy)
            .await
            .expect("submit should be accepted")
            .expect("idle submit should have target");
        let agent = AgentEventContext::root_execution("root-agent", "planner");
        let begun = catalog
            .begin_accepted_turn(accepted, agent.clone(), CancelToken::new())
            .expect("turn should begin");

        let input_events = catalog
            .persist_begun_turn_inputs(&begun, agent.clone())
            .await
            .expect("turn inputs should persist");
        assert_eq!(input_events.len(), 2);

        let persisted = catalog
            .persist_runtime_turn_events(RuntimeTurnPersistenceInput {
                session_id: begun.summary.session_id.clone(),
                turn_id: begun.summary.turn_id.clone(),
                agent,
                runtime_events: vec![
                    RuntimeTurnEvent::AssistantFinal {
                        identity: TurnIdentity::new(
                            session_id.to_string(),
                            "turn-runtime".to_string(),
                            "root-agent".to_string(),
                        ),
                        content: "assistant answer".to_string(),
                        reasoning: Some(astrcode_core::ReasoningContent {
                            content: "assistant thinking".to_string(),
                            signature: Some("sig-1".to_string()),
                        }),
                        tool_call_count: 0,
                    },
                    RuntimeTurnEvent::TurnCompleted {
                        identity: TurnIdentity::new(
                            session_id.to_string(),
                            "turn-runtime".to_string(),
                            "root-agent".to_string(),
                        ),
                        stop_cause: astrcode_agent_runtime::TurnStopCause::Completed,
                        terminal_kind: TurnTerminalKind::Completed,
                    },
                ],
            })
            .await
            .expect("runtime events should persist");

        assert_eq!(persisted.len(), 2);
        assert!(matches!(
            &input_events[0].event.payload,
            StorageEventPayload::UserMessage { content, origin, .. }
                if content == "queued prompt" && *origin == UserMessageOrigin::QueuedInput
        ));
        assert!(matches!(
            &input_events[1].event.payload,
            StorageEventPayload::UserMessage { content, origin, .. }
                if content == "live prompt" && *origin == UserMessageOrigin::User
        ));
        assert!(matches!(
            &persisted[0].event.payload,
            StorageEventPayload::AssistantFinal {
                content,
                reasoning_content,
                reasoning_signature,
                ..
            } if content == "assistant answer"
                && reasoning_content.as_deref() == Some("assistant thinking")
                && reasoning_signature.as_deref() == Some("sig-1")
        ));
        assert!(matches!(
            &persisted[1].event.payload,
            StorageEventPayload::TurnDone { terminal_kind, .. }
                if *terminal_kind == Some(TurnTerminalKind::Completed)
        ));

        let recovered_catalog = SessionCatalog::new(store);
        let loaded = recovered_catalog
            .ensure_loaded_session(&begun.summary.session_id)
            .await
            .expect("session should recover");
        let recovered_turn = loaded
            .state
            .turn_projection("turn-runtime")
            .expect("turn projection should be readable")
            .expect("turn projection should exist");
        assert_eq!(
            recovered_turn.terminal_kind,
            Some(TurnTerminalKind::Completed)
        );
    }

    #[tokio::test]
    async fn request_manual_compact_is_immediate_when_no_turn_is_running() {
        let catalog = SessionCatalog::new(Arc::new(TurnMutationEventStore::default()));
        let meta = catalog
            .create_session("D:/workspace/project")
            .await
            .expect("session should be created");
        let session_id = SessionId::from(meta.session_id);

        let summary = catalog
            .request_manual_compact(compact_input(session_id))
            .await
            .expect("manual compact request should be accepted");

        assert!(summary.accepted);
        assert!(!summary.deferred);
        assert_eq!(summary.message, "手动 compact 已执行。");
    }

    #[tokio::test]
    async fn request_manual_compact_defers_until_running_turn_completes() {
        let catalog = SessionCatalog::new(Arc::new(TurnMutationEventStore::default()));
        let meta = catalog
            .create_session("D:/workspace/project")
            .await
            .expect("session should be created");
        let session_id = SessionId::from(meta.session_id);
        let accepted = catalog
            .accept_submit_prompt(
                submit_input(session_id.clone(), "turn-deferred", "hello"),
                SubmitTurnBusyPolicy::RejectOnBusy,
            )
            .await
            .expect("submit should be accepted")
            .expect("idle submit should have target");
        let turn_id = accepted.summary.turn_id.clone();
        catalog
            .begin_accepted_turn(
                accepted,
                AgentEventContext::root_execution("root-agent", "planner"),
                CancelToken::new(),
            )
            .expect("turn should begin");

        let summary = catalog
            .request_manual_compact(compact_input(session_id.clone()))
            .await
            .expect("manual compact request should be accepted");
        let pending = catalog
            .complete_running_turn(&session_id, &turn_id)
            .expect("turn should complete")
            .expect("pending compact should flush");

        assert!(summary.accepted);
        assert!(summary.deferred);
        assert_eq!(
            summary.message,
            "手动 compact 已登记，会在当前 turn 完成后执行。"
        );
        assert_eq!(pending.instructions.as_deref(), Some("keep latest facts"));
        assert!(pending.preparation.has_external_preparation());
    }

    #[tokio::test]
    async fn interrupt_running_turn_cancels_and_persists_cancelled_terminal_event() {
        let catalog = SessionCatalog::new(Arc::new(TurnMutationEventStore::default()));
        let meta = catalog
            .create_session("D:/workspace/project")
            .await
            .expect("session should be created");
        let session_id = SessionId::from(meta.session_id);
        let accepted = catalog
            .accept_submit_prompt(
                submit_input(session_id.clone(), "turn-cancelled", "hello"),
                SubmitTurnBusyPolicy::RejectOnBusy,
            )
            .await
            .expect("submit should be accepted")
            .expect("idle submit should have target");
        let cancel = CancelToken::new();
        let cancel_probe = cancel.clone();
        catalog
            .begin_accepted_turn(
                accepted,
                AgentEventContext::root_execution("root-agent", "planner"),
                cancel,
            )
            .expect("turn should begin");
        catalog
            .request_manual_compact(compact_input(session_id.clone()))
            .await
            .expect("manual compact should defer");

        let summary = catalog
            .interrupt_running_turn(InterruptSessionMutationInput {
                session_id: session_id.clone(),
            })
            .await
            .expect("interrupt should succeed");

        assert!(summary.accepted);
        assert!(cancel_probe.is_cancelled());
        assert_eq!(
            summary
                .interrupted_turn_id
                .as_ref()
                .map(|turn_id| turn_id.as_str()),
            Some("turn-cancelled")
        );
        assert!(summary.pending_manual_compact.is_some());
        let loaded = catalog
            .ensure_loaded_session(&session_id)
            .await
            .expect("session should remain loaded");
        assert_eq!(
            loaded.state.current_phase().expect("phase should read"),
            Phase::Interrupted
        );
        let stored = loaded
            .state
            .snapshot_recent_stored_events()
            .expect("stored events should read");
        assert!(stored.iter().any(|stored| matches!(
            &stored.event.payload,
            StorageEventPayload::TurnDone { terminal_kind, .. }
                if *terminal_kind == Some(TurnTerminalKind::Cancelled)
        )));
    }
}
