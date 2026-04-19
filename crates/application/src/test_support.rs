//! 应用层测试桩。
//!
//! 提供 `StubSessionPort`，实现 `AppSessionPort` + `AgentSessionPort` 两个 trait，
//! 用于 `application` 内部单元测试，避免依赖真实 `SessionRuntime`。

use astrcode_core::{
    AgentCollaborationFact, AgentEventContext, AgentLifecycleStatus, DeleteProjectResult,
    ExecutionAccepted, InputBatchAckedPayload, InputBatchStartedPayload, InputDiscardedPayload,
    InputQueuedPayload, ModeId, ResolvedRuntimeConfig, SessionId, SessionMeta, StoredEvent, TurnId,
};
use astrcode_kernel::PendingParentDelivery;
use astrcode_session_runtime::{
    AgentObserveSnapshot, ConversationSnapshotFacts, ConversationStreamReplayFacts, ForkPoint,
    ForkResult, ProjectedTurnOutcome, SessionCatalogEvent, SessionControlStateSnapshot,
    SessionModeSnapshot, SessionReplay, SessionTranscriptSnapshot, TurnTerminalSnapshot,
};
use async_trait::async_trait;
use tokio::sync::broadcast;

use crate::{AgentSessionPort, AppAgentPromptSubmission, AppSessionPort};

fn unimplemented_for_test(area: &str) -> ! {
    panic!("not used in {area}")
}

#[derive(Debug, Default)]
pub(crate) struct StubSessionPort {
    stored_events: Vec<StoredEvent>,
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
        _session_id: &SessionId,
        _fork_point: ForkPoint,
    ) -> astrcode_core::Result<ForkResult> {
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
        unimplemented_for_test("application test stub")
    }

    async fn submit_prompt_for_agent(
        &self,
        _session_id: &str,
        _text: String,
        _runtime: ResolvedRuntimeConfig,
        _submission: AppAgentPromptSubmission,
    ) -> astrcode_core::Result<ExecutionAccepted> {
        unimplemented_for_test("application test stub")
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
        unimplemented_for_test("application test stub")
    }

    async fn session_mode_state(
        &self,
        _session_id: &str,
    ) -> astrcode_core::Result<SessionModeSnapshot> {
        Ok(SessionModeSnapshot {
            current_mode_id: ModeId::code(),
            last_mode_changed_at: None,
        })
    }

    async fn switch_mode(
        &self,
        _session_id: &str,
        _from: ModeId,
        _to: ModeId,
    ) -> astrcode_core::Result<StoredEvent> {
        unimplemented_for_test("application test stub")
    }

    async fn session_child_nodes(
        &self,
        _session_id: &str,
    ) -> astrcode_core::Result<Vec<astrcode_core::ChildSessionNode>> {
        unimplemented_for_test("application test stub")
    }

    async fn session_stored_events(
        &self,
        _session_id: &SessionId,
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
    ) -> astrcode_core::Result<Vec<PendingParentDelivery>> {
        unimplemented_for_test("application test stub")
    }

    async fn observe_agent_session(
        &self,
        _open_session_id: &str,
        _target_agent_id: &str,
        _lifecycle_status: AgentLifecycleStatus,
    ) -> astrcode_core::Result<AgentObserveSnapshot> {
        unimplemented_for_test("application test stub")
    }

    async fn project_turn_outcome(
        &self,
        _session_id: &str,
        _turn_id: &str,
    ) -> astrcode_core::Result<ProjectedTurnOutcome> {
        unimplemented_for_test("application test stub")
    }

    async fn wait_for_turn_terminal_snapshot(
        &self,
        _session_id: &str,
        _turn_id: &str,
    ) -> astrcode_core::Result<TurnTerminalSnapshot> {
        unimplemented_for_test("application test stub")
    }
}
