//! 应用层测试桩。
//!
//! 提供 `StubSessionPort`，实现 `AppSessionPort` + `AgentSessionPort` 两个 trait，
//! 用于 `application` 内部单元测试，避免依赖真实 `SessionRuntime`。

use std::sync::{Arc, Mutex};

use astrcode_core::{
    AgentCollaborationFact, AgentEventContext, AgentLifecycleStatus, AstrError,
    DeleteProjectResult, ExecutionAccepted, InputBatchAckedPayload, InputBatchStartedPayload,
    InputDiscardedPayload, InputQueuedPayload, LlmMessage, ModeId, PromptDeclaration,
    ResolvedRuntimeConfig, SessionId, SessionMeta, StorageEvent, StorageEventPayload, StoredEvent,
    TaskSnapshot, TurnId,
};
use astrcode_session_runtime::{
    ConversationSnapshotFacts, ConversationStreamReplayFacts, SessionCatalogEvent,
    SessionControlStateSnapshot, SessionModeSnapshot, SessionReplay, SessionTranscriptSnapshot,
};
use async_trait::async_trait;
use chrono::Utc;
use tokio::sync::broadcast;

use crate::{
    AgentSessionPort, AppAgentPromptSubmission, AppSessionPort, RecoverableParentDelivery,
    SessionForkSelector, SessionObserveSnapshot, SessionTurnOutcomeSummary,
    SessionTurnTerminalState,
};

fn unimplemented_for_test(area: &str) -> ! {
    panic!("not used in {area}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecordedPromptSubmission {
    pub(crate) session_id: String,
    pub(crate) text: String,
    pub(crate) prompt_declarations: Vec<PromptDeclaration>,
    pub(crate) injected_messages: Vec<LlmMessage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecordedModeSwitch {
    pub(crate) session_id: String,
    pub(crate) from: ModeId,
    pub(crate) to: ModeId,
}

#[derive(Debug)]
pub(crate) struct StubSessionPort {
    pub(crate) stored_events: Vec<StoredEvent>,
    pub(crate) working_dir: Option<String>,
    pub(crate) control_state: Option<SessionControlStateSnapshot>,
    pub(crate) active_task_snapshot: Arc<Mutex<Option<TaskSnapshot>>>,
    pub(crate) mode_state: Arc<Mutex<Option<SessionModeSnapshot>>>,
    pub(crate) switch_mode_error: Arc<Mutex<Option<String>>>,
    pub(crate) recorded_submissions: Arc<Mutex<Vec<RecordedPromptSubmission>>>,
    pub(crate) recorded_mode_switches: Arc<Mutex<Vec<RecordedModeSwitch>>>,
}

impl Default for StubSessionPort {
    fn default() -> Self {
        Self {
            stored_events: Vec::new(),
            working_dir: None,
            control_state: None,
            active_task_snapshot: Arc::new(Mutex::new(None)),
            mode_state: Arc::new(Mutex::new(None)),
            switch_mode_error: Arc::new(Mutex::new(None)),
            recorded_submissions: Arc::new(Mutex::new(Vec::new())),
            recorded_mode_switches: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait]
impl AppSessionPort for StubSessionPort {
    fn subscribe_catalog_events(&self) -> broadcast::Receiver<SessionCatalogEvent> {
        let (_tx, rx) = broadcast::channel(1);
        rx
    }

    async fn list_session_metas(&self) -> astrcode_core::Result<Vec<SessionMeta>> {
        unimplemented_for_test("application test stub")
    }

    async fn create_session(&self, _working_dir: String) -> astrcode_core::Result<SessionMeta> {
        unimplemented_for_test("application test stub")
    }

    async fn fork_session(
        &self,
        _session_id: &str,
        _selector: SessionForkSelector,
    ) -> astrcode_core::Result<SessionMeta> {
        unimplemented_for_test("application test stub")
    }

    async fn delete_session(&self, _session_id: &str) -> astrcode_core::Result<()> {
        unimplemented_for_test("application test stub")
    }

    async fn delete_project(
        &self,
        _working_dir: &str,
    ) -> astrcode_core::Result<DeleteProjectResult> {
        unimplemented_for_test("application test stub")
    }

    async fn get_session_working_dir(&self, _session_id: &str) -> astrcode_core::Result<String> {
        Ok(self.working_dir.clone().unwrap_or_else(|| ".".to_string()))
    }

    async fn submit_prompt_for_agent(
        &self,
        session_id: &str,
        text: String,
        _runtime: ResolvedRuntimeConfig,
        submission: AppAgentPromptSubmission,
    ) -> astrcode_core::Result<ExecutionAccepted> {
        self.recorded_submissions
            .lock()
            .expect("submission record lock should work")
            .push(RecordedPromptSubmission {
                session_id: session_id.to_string(),
                text,
                prompt_declarations: submission.prompt_declarations,
                injected_messages: submission.injected_messages,
            });
        Ok(ExecutionAccepted {
            session_id: SessionId::from(session_id.to_string()),
            turn_id: TurnId::from("turn-stub".to_string()),
            agent_id: None,
            branched_from_session_id: None,
        })
    }

    async fn interrupt_session(&self, _session_id: &str) -> astrcode_core::Result<()> {
        unimplemented_for_test("application test stub")
    }

    async fn compact_session(
        &self,
        _session_id: &str,
        _runtime: ResolvedRuntimeConfig,
        _instructions: Option<String>,
    ) -> astrcode_core::Result<bool> {
        unimplemented_for_test("application test stub")
    }

    async fn session_transcript_snapshot(
        &self,
        _session_id: &str,
    ) -> astrcode_core::Result<SessionTranscriptSnapshot> {
        unimplemented_for_test("application test stub")
    }

    async fn conversation_snapshot(
        &self,
        _session_id: &str,
    ) -> astrcode_core::Result<ConversationSnapshotFacts> {
        unimplemented_for_test("application test stub")
    }

    async fn session_control_state(
        &self,
        _session_id: &str,
    ) -> astrcode_core::Result<SessionControlStateSnapshot> {
        Ok(self
            .control_state
            .clone()
            .unwrap_or(SessionControlStateSnapshot {
                phase: astrcode_core::Phase::Idle,
                active_turn_id: None,
                manual_compact_pending: false,
                compacting: false,
                last_compact_meta: None,
                current_mode_id: ModeId::code(),
                last_mode_changed_at: None,
            }))
    }

    async fn active_task_snapshot(
        &self,
        _session_id: &str,
        _owner: &str,
    ) -> astrcode_core::Result<Option<TaskSnapshot>> {
        Ok(self
            .active_task_snapshot
            .lock()
            .expect("active task snapshot lock should work")
            .clone())
    }

    async fn session_mode_state(
        &self,
        _session_id: &str,
    ) -> astrcode_core::Result<SessionModeSnapshot> {
        Ok(self
            .mode_state
            .lock()
            .expect("mode state lock should work")
            .clone()
            .unwrap_or(SessionModeSnapshot {
                current_mode_id: ModeId::code(),
                last_mode_changed_at: None,
            }))
    }

    async fn switch_mode(
        &self,
        session_id: &str,
        from: ModeId,
        to: ModeId,
    ) -> astrcode_core::Result<StoredEvent> {
        if let Some(message) = self
            .switch_mode_error
            .lock()
            .expect("mode switch error lock should work")
            .clone()
        {
            return Err(AstrError::Internal(message));
        }
        self.recorded_mode_switches
            .lock()
            .expect("mode switch record lock should work")
            .push(RecordedModeSwitch {
                session_id: session_id.to_string(),
                from: from.clone(),
                to: to.clone(),
            });
        *self.mode_state.lock().expect("mode state lock should work") = Some(SessionModeSnapshot {
            current_mode_id: to.clone(),
            last_mode_changed_at: None,
        });
        Ok(StoredEvent {
            storage_seq: 1,
            event: StorageEvent {
                turn_id: None,
                agent: AgentEventContext::default(),
                payload: StorageEventPayload::ModeChanged {
                    from,
                    to,
                    timestamp: Utc::now(),
                },
            },
        })
    }

    async fn session_child_nodes(
        &self,
        _session_id: &str,
    ) -> astrcode_core::Result<Vec<astrcode_core::ChildSessionNode>> {
        unimplemented_for_test("application test stub")
    }

    async fn session_stored_events(
        &self,
        _session_id: &str,
    ) -> astrcode_core::Result<Vec<StoredEvent>> {
        Ok(self.stored_events.clone())
    }

    async fn session_replay(
        &self,
        _session_id: &str,
        _last_event_id: Option<&str>,
    ) -> astrcode_core::Result<SessionReplay> {
        unimplemented_for_test("application test stub")
    }

    async fn conversation_stream_replay(
        &self,
        _session_id: &str,
        _last_event_id: Option<&str>,
    ) -> astrcode_core::Result<ConversationStreamReplayFacts> {
        unimplemented_for_test("application test stub")
    }
}

#[async_trait]
impl AgentSessionPort for StubSessionPort {
    async fn create_child_session(
        &self,
        _working_dir: &str,
        _parent_session_id: &str,
    ) -> astrcode_core::Result<SessionMeta> {
        unimplemented_for_test("application test stub")
    }

    async fn submit_prompt_for_agent_with_submission(
        &self,
        _session_id: &str,
        _text: String,
        _runtime: ResolvedRuntimeConfig,
        _submission: AppAgentPromptSubmission,
    ) -> astrcode_core::Result<ExecutionAccepted> {
        unimplemented_for_test("application test stub")
    }

    async fn try_submit_prompt_for_agent_with_turn_id(
        &self,
        _session_id: &str,
        _turn_id: TurnId,
        _text: String,
        _runtime: ResolvedRuntimeConfig,
        _submission: AppAgentPromptSubmission,
    ) -> astrcode_core::Result<Option<ExecutionAccepted>> {
        unimplemented_for_test("application test stub")
    }

    async fn submit_queued_inputs_for_agent_with_turn_id(
        &self,
        _session_id: &str,
        _turn_id: TurnId,
        _queued_inputs: Vec<String>,
        _runtime: ResolvedRuntimeConfig,
        _submission: AppAgentPromptSubmission,
    ) -> astrcode_core::Result<Option<ExecutionAccepted>> {
        unimplemented_for_test("application test stub")
    }

    async fn append_agent_input_queued(
        &self,
        _session_id: &str,
        _turn_id: &str,
        _agent: AgentEventContext,
        _payload: InputQueuedPayload,
    ) -> astrcode_core::Result<StoredEvent> {
        unimplemented_for_test("application test stub")
    }

    async fn append_agent_input_discarded(
        &self,
        _session_id: &str,
        _turn_id: &str,
        _agent: AgentEventContext,
        _payload: InputDiscardedPayload,
    ) -> astrcode_core::Result<StoredEvent> {
        unimplemented_for_test("application test stub")
    }

    async fn append_agent_input_batch_started(
        &self,
        _session_id: &str,
        _turn_id: &str,
        _agent: AgentEventContext,
        _payload: InputBatchStartedPayload,
    ) -> astrcode_core::Result<StoredEvent> {
        unimplemented_for_test("application test stub")
    }

    async fn append_agent_input_batch_acked(
        &self,
        _session_id: &str,
        _turn_id: &str,
        _agent: AgentEventContext,
        _payload: InputBatchAckedPayload,
    ) -> astrcode_core::Result<StoredEvent> {
        unimplemented_for_test("application test stub")
    }

    async fn append_child_session_notification(
        &self,
        _session_id: &str,
        _turn_id: &str,
        _agent: AgentEventContext,
        _notification: astrcode_core::ChildSessionNotification,
    ) -> astrcode_core::Result<StoredEvent> {
        unimplemented_for_test("application test stub")
    }

    async fn append_agent_collaboration_fact(
        &self,
        _session_id: &str,
        _turn_id: &str,
        _agent: AgentEventContext,
        _fact: AgentCollaborationFact,
    ) -> astrcode_core::Result<StoredEvent> {
        unimplemented_for_test("application test stub")
    }

    async fn pending_delivery_ids_for_agent(
        &self,
        _session_id: &str,
        _agent_id: &str,
    ) -> astrcode_core::Result<Vec<String>> {
        unimplemented_for_test("application test stub")
    }

    async fn recoverable_parent_deliveries(
        &self,
        _parent_session_id: &str,
    ) -> astrcode_core::Result<Vec<RecoverableParentDelivery>> {
        unimplemented_for_test("application test stub")
    }

    async fn observe_agent_session(
        &self,
        _open_session_id: &str,
        _target_agent_id: &str,
        _lifecycle_status: AgentLifecycleStatus,
    ) -> astrcode_core::Result<SessionObserveSnapshot> {
        unimplemented_for_test("application test stub")
    }

    async fn project_turn_outcome(
        &self,
        _session_id: &str,
        _turn_id: &str,
    ) -> astrcode_core::Result<SessionTurnOutcomeSummary> {
        unimplemented_for_test("application test stub")
    }

    async fn wait_for_turn_terminal_snapshot(
        &self,
        _session_id: &str,
        _turn_id: &str,
    ) -> astrcode_core::Result<SessionTurnTerminalState> {
        unimplemented_for_test("application test stub")
    }
}
