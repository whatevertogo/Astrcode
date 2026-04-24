use std::{
    collections::HashSet,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use astrcode_core::{
    AgentCollaborationFact, AgentEventContext, AgentLifecycleStatus, AstrError,
    DeleteProjectResult, ExecutionAccepted, InputBatchAckedPayload, InputBatchStartedPayload,
    InputDiscardedPayload, InputQueuedPayload, LlmMessage, ModeId, PromptDeclaration,
    ResolvedRuntimeConfig, SessionId, SessionMeta, SkillCatalog, StorageEvent, StorageEventPayload,
    StoredEvent, TaskSnapshot, TurnId, TurnTerminalKind, UserMessageOrigin,
};
use astrcode_host_session::{SessionCatalogEvent, SessionControlStateSnapshot, SessionModeState};
use async_trait::async_trait;
use tokio::sync::broadcast;

use crate::{
    AgentSessionPort, AppAgentPromptSubmission, AppState, FrontendBuild, RecoverableParentDelivery,
    SessionTurnOutcomeSummary,
    application_error_bridge::ServerRouteError,
    auth::{AuthSessionManager, BootstrapAuth},
    bootstrap::{ServerBootstrapOptions, bootstrap_server_runtime_with_options},
    conversation_read_model::{
        ConversationSnapshotFacts, ConversationStreamReplayFacts, SessionReplay,
        SessionTranscriptSnapshot,
    },
    ports::{
        AppSessionPort, DurableSubRunStatusSummary, SessionObserveSnapshot,
        SessionTurnTerminalState,
    },
    session_use_cases::SessionForkSelector,
    watch_service::{WatchEvent, WatchPort, WatchService, WatchSource},
};

pub(crate) struct ServerTestContext {
    temp_home: tempfile::TempDir,
}

impl ServerTestContext {
    pub(crate) fn new() -> Self {
        Self {
            temp_home: tempfile::tempdir().expect("tempdir should be created"),
        }
    }

    pub(crate) fn home_dir(&self) -> &Path {
        self.temp_home.path()
    }
}

pub(crate) struct ManualWatchHarness {
    port: Arc<ManualWatchPort>,
    service: Arc<WatchService>,
}

impl ManualWatchHarness {
    pub(crate) fn new() -> Self {
        let port = Arc::new(ManualWatchPort::default());
        let service = Arc::new(WatchService::new(port.clone()));
        Self { port, service }
    }

    pub(crate) fn service(&self) -> Arc<WatchService> {
        Arc::clone(&self.service)
    }

    pub(crate) fn emit(&self, source: WatchSource, affected_paths: Vec<String>) {
        self.port.emit(source, affected_paths);
    }

    pub(crate) async fn wait_for_source(
        &self,
        source: &WatchSource,
        timeout: Duration,
    ) -> Result<(), String> {
        let deadline = tokio::time::Instant::now() + timeout;
        while tokio::time::Instant::now() < deadline {
            if self.port.has_source(source) {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Err(format!(
            "watch source '{source:?}' was not registered before timeout"
        ))
    }
}

#[derive(Default)]
struct ManualWatchPort {
    tx: Mutex<Option<broadcast::Sender<WatchEvent>>>,
    sources: Mutex<HashSet<WatchSource>>,
}

impl ManualWatchPort {
    fn emit(&self, source: WatchSource, affected_paths: Vec<String>) {
        let registered = self
            .sources
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(&source);
        if !registered {
            return;
        }
        let tx = self
            .tx
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(tx) = tx {
            let _ = tx.send(WatchEvent {
                source,
                affected_paths,
            });
        }
    }

    fn has_source(&self, source: &WatchSource) -> bool {
        self.sources
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(source)
    }
}

impl WatchPort for ManualWatchPort {
    fn start_watch(
        &self,
        sources: Vec<WatchSource>,
        tx: broadcast::Sender<WatchEvent>,
    ) -> Result<(), ServerRouteError> {
        *self
            .tx
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(tx);
        let mut registered = self
            .sources
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        registered.extend(sources);
        Ok(())
    }

    fn stop_all(&self) -> Result<(), ServerRouteError> {
        *self
            .tx
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        self.sources
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        Ok(())
    }

    fn add_source(&self, source: WatchSource) -> Result<(), ServerRouteError> {
        self.sources
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(source);
        Ok(())
    }

    fn remove_source(&self, source: &WatchSource) -> Result<(), ServerRouteError> {
        self.sources
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(source);
        Ok(())
    }
}

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
#[allow(dead_code)]
pub(crate) struct StubSessionPort {
    pub(crate) stored_events: Vec<StoredEvent>,
    pub(crate) working_dir: Option<String>,
    pub(crate) control_state: Option<SessionControlStateSnapshot>,
    pub(crate) active_task_snapshot: Arc<Mutex<Option<TaskSnapshot>>>,
    pub(crate) mode_state: Arc<Mutex<Option<SessionModeState>>>,
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
        unimplemented_for_test("server test stub")
    }

    async fn create_session(&self, _working_dir: String) -> astrcode_core::Result<SessionMeta> {
        unimplemented_for_test("server test stub")
    }

    async fn fork_session(
        &self,
        _session_id: &str,
        _selector: SessionForkSelector,
    ) -> astrcode_core::Result<SessionMeta> {
        unimplemented_for_test("server test stub")
    }

    async fn delete_session(&self, _session_id: &str) -> astrcode_core::Result<()> {
        unimplemented_for_test("server test stub")
    }

    async fn delete_project(
        &self,
        _working_dir: &str,
    ) -> astrcode_core::Result<DeleteProjectResult> {
        unimplemented_for_test("server test stub")
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
        unimplemented_for_test("server test stub")
    }

    async fn compact_session(
        &self,
        _session_id: &str,
        _runtime: ResolvedRuntimeConfig,
        _instructions: Option<String>,
    ) -> astrcode_core::Result<bool> {
        unimplemented_for_test("server test stub")
    }

    async fn session_transcript_snapshot(
        &self,
        _session_id: &str,
    ) -> astrcode_core::Result<SessionTranscriptSnapshot> {
        unimplemented_for_test("server test stub")
    }

    async fn conversation_snapshot(
        &self,
        _session_id: &str,
    ) -> astrcode_core::Result<ConversationSnapshotFacts> {
        unimplemented_for_test("server test stub")
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
    ) -> astrcode_core::Result<SessionModeState> {
        Ok(self
            .mode_state
            .lock()
            .expect("mode state lock should work")
            .clone()
            .unwrap_or(SessionModeState {
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
        *self.mode_state.lock().expect("mode state lock should work") = Some(SessionModeState {
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
                    timestamp: chrono::Utc::now(),
                },
            },
        })
    }

    async fn session_child_nodes(
        &self,
        _session_id: &str,
    ) -> astrcode_core::Result<Vec<astrcode_core::ChildSessionNode>> {
        unimplemented_for_test("server test stub")
    }

    async fn session_stored_events(
        &self,
        _session_id: &str,
    ) -> astrcode_core::Result<Vec<StoredEvent>> {
        Ok(self.stored_events.clone())
    }

    async fn durable_subrun_status_snapshot(
        &self,
        _parent_session_id: &str,
        _requested_subrun_id: &str,
    ) -> astrcode_core::Result<Option<DurableSubRunStatusSummary>> {
        unimplemented_for_test("server test stub")
    }

    async fn session_replay(
        &self,
        _session_id: &str,
        _last_event_id: Option<&str>,
    ) -> astrcode_core::Result<SessionReplay> {
        unimplemented_for_test("server test stub")
    }

    async fn conversation_stream_replay(
        &self,
        _session_id: &str,
        _last_event_id: Option<&str>,
    ) -> astrcode_core::Result<ConversationStreamReplayFacts> {
        unimplemented_for_test("server test stub")
    }
}

#[async_trait]
impl AgentSessionPort for StubSessionPort {
    async fn create_child_session(
        &self,
        _working_dir: &str,
        _parent_session_id: &str,
    ) -> astrcode_core::Result<SessionMeta> {
        unimplemented_for_test("server test stub")
    }

    async fn submit_prompt_for_agent_with_submission(
        &self,
        _session_id: &str,
        _text: String,
        _runtime: ResolvedRuntimeConfig,
        _submission: AppAgentPromptSubmission,
    ) -> astrcode_core::Result<ExecutionAccepted> {
        unimplemented_for_test("server test stub")
    }

    async fn try_submit_prompt_for_agent_with_turn_id(
        &self,
        _session_id: &str,
        _turn_id: TurnId,
        _text: String,
        _runtime: ResolvedRuntimeConfig,
        _submission: AppAgentPromptSubmission,
    ) -> astrcode_core::Result<Option<ExecutionAccepted>> {
        unimplemented_for_test("server test stub")
    }

    async fn submit_queued_inputs_for_agent_with_turn_id(
        &self,
        _session_id: &str,
        _turn_id: TurnId,
        _queued_inputs: Vec<String>,
        _runtime: ResolvedRuntimeConfig,
        _submission: AppAgentPromptSubmission,
    ) -> astrcode_core::Result<Option<ExecutionAccepted>> {
        unimplemented_for_test("server test stub")
    }

    async fn append_agent_input_queued(
        &self,
        _session_id: &str,
        _turn_id: &str,
        _agent: AgentEventContext,
        _payload: InputQueuedPayload,
    ) -> astrcode_core::Result<StoredEvent> {
        unimplemented_for_test("server test stub")
    }

    async fn append_agent_input_discarded(
        &self,
        _session_id: &str,
        _turn_id: &str,
        _agent: AgentEventContext,
        _payload: InputDiscardedPayload,
    ) -> astrcode_core::Result<StoredEvent> {
        unimplemented_for_test("server test stub")
    }

    async fn append_agent_input_batch_started(
        &self,
        _session_id: &str,
        _turn_id: &str,
        _agent: AgentEventContext,
        _payload: InputBatchStartedPayload,
    ) -> astrcode_core::Result<StoredEvent> {
        unimplemented_for_test("server test stub")
    }

    async fn append_agent_input_batch_acked(
        &self,
        _session_id: &str,
        _turn_id: &str,
        _agent: AgentEventContext,
        _payload: InputBatchAckedPayload,
    ) -> astrcode_core::Result<StoredEvent> {
        unimplemented_for_test("server test stub")
    }

    async fn append_child_session_notification(
        &self,
        _session_id: &str,
        _turn_id: &str,
        _agent: AgentEventContext,
        _notification: astrcode_core::ChildSessionNotification,
    ) -> astrcode_core::Result<StoredEvent> {
        unimplemented_for_test("server test stub")
    }

    async fn append_agent_collaboration_fact(
        &self,
        _session_id: &str,
        _turn_id: &str,
        _agent: AgentEventContext,
        _fact: AgentCollaborationFact,
    ) -> astrcode_core::Result<StoredEvent> {
        unimplemented_for_test("server test stub")
    }

    async fn pending_delivery_ids_for_agent(
        &self,
        _session_id: &str,
        _agent_id: &str,
    ) -> astrcode_core::Result<Vec<String>> {
        unimplemented_for_test("server test stub")
    }

    async fn recoverable_parent_deliveries(
        &self,
        _parent_session_id: &str,
    ) -> astrcode_core::Result<Vec<RecoverableParentDelivery>> {
        unimplemented_for_test("server test stub")
    }

    async fn observe_agent_session(
        &self,
        _open_session_id: &str,
        _target_agent_id: &str,
        _lifecycle_status: AgentLifecycleStatus,
    ) -> astrcode_core::Result<SessionObserveSnapshot> {
        unimplemented_for_test("server test stub")
    }

    async fn project_turn_outcome(
        &self,
        _session_id: &str,
        _turn_id: &str,
    ) -> astrcode_core::Result<SessionTurnOutcomeSummary> {
        unimplemented_for_test("server test stub")
    }

    async fn wait_for_turn_terminal_snapshot(
        &self,
        _session_id: &str,
        _turn_id: &str,
    ) -> astrcode_core::Result<SessionTurnTerminalState> {
        unimplemented_for_test("server test stub")
    }
}

pub(crate) async fn test_state(
    frontend_build: Option<FrontendBuild>,
) -> (AppState, ServerTestContext) {
    test_state_with_options(
        frontend_build,
        ServerBootstrapOptions {
            enable_profile_watch: false,
            ..ServerBootstrapOptions::default()
        },
    )
    .await
}

pub(crate) async fn test_state_with_options(
    frontend_build: Option<FrontendBuild>,
    mut options: ServerBootstrapOptions,
) -> (AppState, ServerTestContext) {
    let context = ServerTestContext::new();
    options.home_dir = Some(context.home_dir().to_path_buf());
    let runtime = bootstrap_server_runtime_with_options(options)
        .await
        .expect("server runtime should bootstrap in tests");
    let agent_api = Arc::clone(&runtime.agent_api);
    let agent_control = Arc::clone(&runtime.agent_control);
    let config = Arc::clone(&runtime.config);
    let session_catalog = Arc::clone(&runtime.session_catalog);
    let profiles = Arc::clone(&runtime.profiles);
    let subagent_executor = Arc::clone(&runtime.subagent_executor);
    let mcp_service = Arc::clone(&runtime.mcp_service);
    let skill_catalog: Arc<dyn SkillCatalog> = Arc::clone(&runtime.skill_catalog);
    let resource_catalog = Arc::clone(&runtime.resource_catalog);
    let mode_catalog = Arc::clone(&runtime.mode_catalog);
    let governance = Arc::clone(&runtime.governance);
    let auth_sessions = Arc::new(AuthSessionManager::default());
    auth_sessions.issue_test_token("browser-token");

    (
        AppState {
            agent_api,
            agent_control,
            config,
            session_catalog,
            profiles,
            subagent_executor,
            mcp_service,
            skill_catalog,
            resource_catalog,
            mode_catalog,
            governance,
            auth_sessions,
            bootstrap_auth: BootstrapAuth::new(
                "browser-token".to_string(),
                chrono::Utc::now()
                    .checked_add_signed(
                        chrono::Duration::from_std(Duration::from_secs(60))
                            .expect("duration should convert"),
                    )
                    .expect("expiry should be valid")
                    .timestamp_millis(),
            ),
            frontend_build,
            _runtime_handles: runtime.handles,
        },
        context,
    )
}

async fn append_root_event(state: &crate::AppState, session_id: &str, event: StorageEvent) {
    state
        ._runtime_handles
        .session_runtime_test_support
        .append_event(session_id, event)
        .await
        .expect("event should append");
}

pub(crate) async fn seed_completed_root_turn(
    state: &crate::AppState,
    session_id: &str,
    turn_id: &str,
) {
    let agent = AgentEventContext::root_execution("root-agent", "test-profile");
    append_root_event(
        state,
        session_id,
        StorageEvent {
            turn_id: Some(turn_id.to_string()),
            agent: agent.clone(),
            payload: StorageEventPayload::UserMessage {
                content: "hello".to_string(),
                origin: UserMessageOrigin::User,
                timestamp: chrono::Utc::now(),
            },
        },
    )
    .await;
    append_root_event(
        state,
        session_id,
        StorageEvent {
            turn_id: Some(turn_id.to_string()),
            agent: agent.clone(),
            payload: StorageEventPayload::AssistantFinal {
                content: "world".to_string(),
                reasoning_content: None,
                reasoning_signature: None,
                step_index: None,
                timestamp: Some(chrono::Utc::now()),
            },
        },
    )
    .await;
    append_root_event(
        state,
        session_id,
        StorageEvent {
            turn_id: Some(turn_id.to_string()),
            agent,
            payload: StorageEventPayload::TurnDone {
                timestamp: chrono::Utc::now(),
                terminal_kind: Some(TurnTerminalKind::Completed),
                reason: Some("completed".to_string()),
            },
        },
    )
    .await;
}

pub(crate) async fn seed_unfinished_root_turn(
    state: &crate::AppState,
    session_id: &str,
    turn_id: &str,
) {
    append_root_event(
        state,
        session_id,
        StorageEvent {
            turn_id: Some(turn_id.to_string()),
            agent: AgentEventContext::root_execution("root-agent", "test-profile"),
            payload: StorageEventPayload::UserMessage {
                content: "still running".to_string(),
                origin: UserMessageOrigin::User,
                timestamp: chrono::Utc::now(),
            },
        },
    )
    .await;
}

pub(crate) async fn mark_session_running(state: &crate::AppState, session_id: &str) {
    state
        ._runtime_handles
        .session_runtime_test_support
        .prepare_test_turn_runtime(session_id, "test-running-turn")
        .await
        .expect("session should enter running state");
}
