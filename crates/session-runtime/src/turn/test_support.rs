#![cfg(test)]

use std::{
    collections::{HashMap, VecDeque},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use astrcode_core::{
    AgentCollaborationFact, AgentId, AgentStateProjector, AstrError, EventLogWriter, EventStore,
    EventTranslator, LlmOutput, LlmProvider, LlmRequest, ModelLimits, Phase, PromptBuildOutput,
    PromptBuildRequest, PromptFacts, PromptFactsProvider, PromptFactsRequest, PromptProvider,
    ResourceProvider, ResourceReadResult, ResourceRequestContext, Result, RuntimeMetricsRecorder,
    SessionMeta, SessionTurnAcquireResult, StorageEvent, StorageEventPayload, StoreResult,
    StoredEvent, SubRunExecutionOutcome, Tool,
};
use astrcode_kernel::{Kernel, KernelGateway, ToolCapabilityInvoker};
use async_trait::async_trait;
use serde_json::Value;

use crate::{
    SessionRuntime, SessionState, SessionWriter,
    actor::SessionActor,
    state::append_and_broadcast,
    turn::events::{
        CompactAppliedStats, assistant_final_event, compact_applied_event, user_message_event,
    },
};

#[derive(Debug)]
struct NoopLlmProvider {
    limits: ModelLimits,
}

#[async_trait]
impl LlmProvider for NoopLlmProvider {
    async fn generate(
        &self,
        _request: LlmRequest,
        _sink: Option<astrcode_core::LlmEventSink>,
    ) -> Result<LlmOutput> {
        Err(AstrError::Validation(
            "turn test noop llm provider should not execute".to_string(),
        ))
    }

    fn model_limits(&self) -> ModelLimits {
        self.limits
    }
}

#[derive(Debug)]
struct NoopPromptProvider;

#[async_trait]
impl PromptProvider for NoopPromptProvider {
    async fn build_prompt(&self, _request: PromptBuildRequest) -> Result<PromptBuildOutput> {
        Ok(PromptBuildOutput {
            system_prompt: "noop".to_string(),
            system_prompt_blocks: Vec::new(),
            metadata: Value::Null,
        })
    }
}

#[derive(Debug)]
pub(crate) struct NoopPromptFactsProvider;

#[async_trait]
impl PromptFactsProvider for NoopPromptFactsProvider {
    async fn resolve_prompt_facts(&self, _request: &PromptFactsRequest) -> Result<PromptFacts> {
        Ok(PromptFacts::default())
    }
}

#[derive(Debug)]
struct NoopResourceProvider;

#[async_trait]
impl ResourceProvider for NoopResourceProvider {
    async fn read_resource(
        &self,
        _uri: &str,
        _context: &ResourceRequestContext,
    ) -> Result<ResourceReadResult> {
        Ok(ResourceReadResult {
            uri: "noop://resource".to_string(),
            content: Value::Null,
            metadata: Value::Null,
        })
    }
}

#[derive(Debug, Default)]
struct NoopEventLogWriter {
    next_seq: u64,
}

impl EventLogWriter for NoopEventLogWriter {
    fn append(&mut self, event: &StorageEvent) -> StoreResult<astrcode_core::StoredEvent> {
        self.next_seq += 1;
        Ok(astrcode_core::StoredEvent {
            storage_seq: self.next_seq,
            event: event.clone(),
        })
    }
}

pub(crate) fn test_gateway(context_window: usize) -> KernelGateway {
    KernelGateway::new(
        astrcode_kernel::CapabilityRouter::empty(),
        Arc::new(NoopLlmProvider {
            limits: ModelLimits {
                context_window,
                max_output_tokens: 4096,
            },
        }),
        Arc::new(NoopPromptProvider),
        Arc::new(NoopResourceProvider),
    )
}

pub(crate) fn test_kernel_with_tool(tool: Arc<dyn Tool>, context_window: usize) -> Kernel {
    let router = astrcode_kernel::CapabilityRouter::builder()
        .register_invoker(Arc::new(
            ToolCapabilityInvoker::new(tool).expect("tool invoker should build"),
        ))
        .build()
        .expect("router should build");
    Kernel::builder()
        .with_capabilities(router)
        .with_llm_provider(Arc::new(NoopLlmProvider {
            limits: ModelLimits {
                context_window,
                max_output_tokens: 4096,
            },
        }))
        .with_prompt_provider(Arc::new(NoopPromptProvider))
        .with_resource_provider(Arc::new(NoopResourceProvider))
        .build()
        .expect("kernel should build")
}

pub(crate) fn test_kernel(context_window: usize) -> Kernel {
    Kernel::builder()
        .with_capabilities(astrcode_kernel::CapabilityRouter::empty())
        .with_llm_provider(Arc::new(NoopLlmProvider {
            limits: ModelLimits {
                context_window,
                max_output_tokens: 4096,
            },
        }))
        .with_prompt_provider(Arc::new(NoopPromptProvider))
        .with_resource_provider(Arc::new(NoopResourceProvider))
        .build()
        .expect("kernel should build")
}

pub(crate) fn test_runtime(event_store: Arc<dyn EventStore>) -> SessionRuntime {
    SessionRuntime::new(
        Arc::new(test_kernel(8192)),
        Arc::new(NoopPromptFactsProvider),
        event_store,
        Arc::new(NoopMetrics),
    )
}

pub(crate) fn test_session_state() -> Arc<SessionState> {
    Arc::new(SessionState::new(
        Phase::Idle,
        Arc::new(SessionWriter::new(Box::new(NoopEventLogWriter::default()))),
        AgentStateProjector::default(),
        Vec::new(),
        Vec::new(),
    ))
}

#[derive(Debug, Default)]
pub(crate) struct StubEventStore {
    next_seq: AtomicU64,
}

pub(crate) struct StubTurnLease;

impl astrcode_core::SessionTurnLease for StubTurnLease {}

#[async_trait]
impl EventStore for StubEventStore {
    async fn ensure_session(
        &self,
        _session_id: &astrcode_core::SessionId,
        _working_dir: &Path,
    ) -> Result<()> {
        Ok(())
    }

    async fn append(
        &self,
        _session_id: &astrcode_core::SessionId,
        event: &StorageEvent,
    ) -> Result<StoredEvent> {
        Ok(StoredEvent {
            storage_seq: self.next_seq.fetch_add(1, Ordering::SeqCst) + 1,
            event: event.clone(),
        })
    }

    async fn replay(&self, _session_id: &astrcode_core::SessionId) -> Result<Vec<StoredEvent>> {
        Ok(Vec::new())
    }

    async fn try_acquire_turn(
        &self,
        _session_id: &astrcode_core::SessionId,
        _turn_id: &str,
    ) -> Result<SessionTurnAcquireResult> {
        Ok(SessionTurnAcquireResult::Acquired(Box::new(StubTurnLease)))
    }

    async fn list_sessions(&self) -> Result<Vec<astrcode_core::SessionId>> {
        Ok(Vec::new())
    }

    async fn list_session_metas(&self) -> Result<Vec<SessionMeta>> {
        Ok(Vec::new())
    }

    async fn delete_session(&self, _session_id: &astrcode_core::SessionId) -> Result<()> {
        Ok(())
    }

    async fn delete_sessions_by_working_dir(
        &self,
        _working_dir: &str,
    ) -> Result<astrcode_core::DeleteProjectResult> {
        Ok(astrcode_core::DeleteProjectResult {
            success_count: 0,
            failed_session_ids: Vec::new(),
        })
    }
}

pub(crate) struct NoopMetrics;

impl RuntimeMetricsRecorder for NoopMetrics {
    fn record_session_rehydrate(&self, _duration_ms: u64, _success: bool) {}

    fn record_sse_catch_up(
        &self,
        _duration_ms: u64,
        _success: bool,
        _used_disk_fallback: bool,
        _recovered_events: u64,
    ) {
    }

    fn record_turn_execution(&self, _duration_ms: u64, _success: bool) {}

    fn record_subrun_execution(
        &self,
        _duration_ms: u64,
        _outcome: SubRunExecutionOutcome,
        _step_count: Option<u32>,
        _estimated_tokens: Option<u64>,
        _storage_mode: Option<astrcode_core::SubRunStorageMode>,
    ) {
    }

    fn record_child_spawned(&self) {}
    fn record_parent_reactivation_requested(&self) {}
    fn record_parent_reactivation_succeeded(&self) {}
    fn record_parent_reactivation_failed(&self) {}
    fn record_delivery_buffer_queued(&self) {}
    fn record_delivery_buffer_dequeued(&self) {}
    fn record_delivery_buffer_wake_requested(&self) {}
    fn record_delivery_buffer_wake_succeeded(&self) {}
    fn record_delivery_buffer_wake_failed(&self) {}
    fn record_cache_reuse_hit(&self) {}
    fn record_cache_reuse_miss(&self) {}
    fn record_agent_collaboration_fact(&self, _fact: &AgentCollaborationFact) {}
}

pub(crate) async fn test_actor() -> Arc<SessionActor> {
    Arc::new(
        SessionActor::new_persistent(
            astrcode_core::SessionId::from("session-1".to_string()),
            ".",
            AgentId::from("root-agent".to_string()),
            Arc::new(StubEventStore::default()),
        )
        .await
        .expect("test actor should initialize"),
    )
}

pub(crate) fn root_turn_event(turn_id: Option<&str>, payload: StorageEventPayload) -> StorageEvent {
    StorageEvent {
        turn_id: turn_id.map(str::to_string),
        agent: astrcode_core::AgentEventContext::default(),
        payload,
    }
}

pub(crate) fn root_user_message_event(turn_id: &str, content: impl Into<String>) -> StorageEvent {
    user_message_event(
        turn_id,
        &astrcode_core::AgentEventContext::default(),
        content.into(),
        astrcode_core::UserMessageOrigin::User,
        chrono::Utc::now(),
    )
}

pub(crate) fn root_assistant_final_event(
    turn_id: &str,
    content: impl Into<String>,
) -> StorageEvent {
    assistant_final_event(
        turn_id,
        &astrcode_core::AgentEventContext::default(),
        content.into(),
        None,
        None,
        Some(chrono::Utc::now()),
    )
}

pub(crate) fn root_compact_applied_event(
    turn_id: &str,
    summary: impl Into<String>,
    preserved_recent_turns: usize,
    pre_tokens: usize,
    post_tokens_estimate: usize,
    messages_removed: usize,
    tokens_freed: usize,
) -> StorageEvent {
    compact_applied_event(
        Some(turn_id),
        &astrcode_core::AgentEventContext::default(),
        astrcode_core::CompactTrigger::Auto,
        summary.into(),
        CompactAppliedStats {
            preserved_recent_turns,
            pre_tokens,
            post_tokens_estimate,
            messages_removed,
            tokens_freed,
        },
        chrono::Utc::now(),
    )
}

pub(crate) fn assert_contains_error_message(events: &[StoredEvent], expected_message: &str) {
    assert!(
        events.iter().any(|stored| matches!(
            &stored.event.payload,
            StorageEventPayload::Error { message, .. } if message == expected_message
        )),
        "expected stored events to contain Error('{expected_message}')"
    );
}

pub(crate) fn assert_contains_compact_summary(events: &[StoredEvent], expected_summary: &str) {
    assert!(
        events.iter().any(|stored| matches!(
            &stored.event.payload,
            StorageEventPayload::CompactApplied { summary, .. } if summary == expected_summary
        )),
        "expected stored events to contain CompactApplied('{expected_summary}')"
    );
}

pub(crate) fn assert_has_turn_done(events: &[StorageEvent]) {
    assert!(
        events
            .iter()
            .any(|event| matches!(&event.payload, StorageEventPayload::TurnDone { .. })),
        "expected events to contain TurnDone"
    );
}

pub(crate) async fn append_root_turn_event_to_actor(
    actor: &Arc<SessionActor>,
    event: StorageEvent,
) {
    let mut translator = EventTranslator::new(actor.state().current_phase().expect("phase"));
    append_and_broadcast(actor.state(), &event, &mut translator)
        .await
        .expect("test event should append");
}

enum AcquireScript {
    Busy { turn_id: String },
    Acquired,
}

#[derive(Default)]
pub(crate) struct BranchingTestEventStore {
    next_seq: AtomicU64,
    events: Mutex<HashMap<String, Vec<StoredEvent>>>,
    metas: Mutex<HashMap<String, SessionMeta>>,
    acquire_scripts: Mutex<VecDeque<AcquireScript>>,
}

impl BranchingTestEventStore {
    pub(crate) fn push_busy(&self, turn_id: impl Into<String>) {
        self.acquire_scripts
            .lock()
            .expect("acquire_scripts lock should work")
            .push_back(AcquireScript::Busy {
                turn_id: turn_id.into(),
            });
    }

    pub(crate) fn stored_events_for(&self, session_id: &str) -> Vec<StoredEvent> {
        self.events
            .lock()
            .expect("events lock should work")
            .get(session_id)
            .cloned()
            .unwrap_or_default()
    }
}

#[async_trait]
impl EventStore for BranchingTestEventStore {
    async fn ensure_session(
        &self,
        session_id: &astrcode_core::SessionId,
        working_dir: &Path,
    ) -> Result<()> {
        let now = chrono::Utc::now();
        self.metas
            .lock()
            .expect("metas lock should work")
            .entry(session_id.to_string())
            .or_insert_with(|| SessionMeta {
                session_id: session_id.to_string(),
                working_dir: working_dir.display().to_string(),
                display_name: crate::display_name_from_working_dir(working_dir),
                title: "New Session".to_string(),
                created_at: now,
                updated_at: now,
                parent_session_id: None,
                parent_storage_seq: None,
                phase: Phase::Idle,
            });
        Ok(())
    }

    async fn append(
        &self,
        session_id: &astrcode_core::SessionId,
        event: &StorageEvent,
    ) -> Result<StoredEvent> {
        let storage_seq = self.next_seq.fetch_add(1, Ordering::SeqCst) + 1;
        let stored = StoredEvent {
            storage_seq,
            event: event.clone(),
        };
        self.events
            .lock()
            .expect("events lock should work")
            .entry(session_id.to_string())
            .or_default()
            .push(stored.clone());

        let mut metas = self.metas.lock().expect("metas lock should work");
        let meta = metas
            .entry(session_id.to_string())
            .or_insert_with(|| SessionMeta {
                session_id: session_id.to_string(),
                working_dir: ".".to_string(),
                display_name: ".".to_string(),
                title: "New Session".to_string(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                parent_session_id: None,
                parent_storage_seq: None,
                phase: Phase::Idle,
            });
        meta.updated_at = chrono::Utc::now();
        if let StorageEventPayload::SessionStart {
            working_dir,
            parent_session_id,
            parent_storage_seq,
            ..
        } = &event.payload
        {
            meta.working_dir = working_dir.clone();
            meta.display_name = crate::display_name_from_working_dir(Path::new(working_dir));
            meta.parent_session_id = parent_session_id.clone();
            meta.parent_storage_seq = *parent_storage_seq;
        }
        Ok(stored)
    }

    async fn replay(&self, session_id: &astrcode_core::SessionId) -> Result<Vec<StoredEvent>> {
        Ok(self.stored_events_for(session_id.as_str()))
    }

    async fn try_acquire_turn(
        &self,
        _session_id: &astrcode_core::SessionId,
        _turn_id: &str,
    ) -> Result<SessionTurnAcquireResult> {
        let scripted = self
            .acquire_scripts
            .lock()
            .expect("acquire_scripts lock should work")
            .pop_front();
        match scripted.unwrap_or(AcquireScript::Acquired) {
            AcquireScript::Busy { turn_id } => Ok(SessionTurnAcquireResult::Busy(
                astrcode_core::SessionTurnBusy {
                    turn_id,
                    owner_pid: std::process::id(),
                    acquired_at: chrono::Utc::now(),
                },
            )),
            AcquireScript::Acquired => {
                Ok(SessionTurnAcquireResult::Acquired(Box::new(StubTurnLease)))
            },
        }
    }

    async fn list_sessions(&self) -> Result<Vec<astrcode_core::SessionId>> {
        Ok(self
            .metas
            .lock()
            .expect("metas lock should work")
            .keys()
            .cloned()
            .map(astrcode_core::SessionId::from)
            .collect())
    }

    async fn list_session_metas(&self) -> Result<Vec<SessionMeta>> {
        Ok(self
            .metas
            .lock()
            .expect("metas lock should work")
            .values()
            .cloned()
            .collect())
    }

    async fn delete_session(&self, _session_id: &astrcode_core::SessionId) -> Result<()> {
        Ok(())
    }

    async fn delete_sessions_by_working_dir(
        &self,
        _working_dir: &str,
    ) -> Result<astrcode_core::DeleteProjectResult> {
        Ok(astrcode_core::DeleteProjectResult {
            success_count: 0,
            failed_session_ids: Vec::new(),
        })
    }
}
