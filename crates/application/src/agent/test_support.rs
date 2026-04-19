//! Agent 编排子域的测试基础设施。
//!
//! 提供 `AgentTestHarness` 和 `AgentTestEnvGuard`，用于在隔离环境中测试
//! `AgentOrchestrationService` 的协作编排逻辑，无需启动真实 session-runtime。

use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
};

use astrcode_core::{
    AgentMode, AgentProfile, AstrError, Config, ConfigOverlay, DeleteProjectResult, EventStore,
    LlmEvent, LlmFinishReason, LlmOutput, LlmProvider, LlmRequest, ModelLimits, Phase,
    PromptBuildOutput, PromptBuildRequest, PromptFacts, PromptFactsProvider, PromptProvider,
    ReasoningContent, ResourceProvider, ResourceReadResult, ResourceRequestContext, Result,
    SessionId, SessionMeta, SessionTurnAcquireResult, SessionTurnBusy, SessionTurnLease,
    StorageEvent, StoredEvent,
    ports::{ConfigStore, McpConfigFileScope},
};
use astrcode_kernel::{CapabilityRouter, Kernel};
use astrcode_session_runtime::{SessionRuntime, display_name_from_working_dir};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;

use crate::{
    AgentKernelPort, AgentOrchestrationService, AgentSessionPort, ApplicationError, ConfigService,
    GovernanceSurfaceAssembler, ProfileResolutionService, RuntimeObservabilityCollector,
    execution::ProfileProvider, lifecycle::TaskRegistry,
};

pub(crate) struct AgentTestHarness {
    pub(crate) _guard: AgentTestEnvGuard,
    pub(crate) kernel: Arc<Kernel>,
    pub(crate) session_runtime: Arc<SessionRuntime>,
    pub(crate) service: AgentOrchestrationService,
    pub(crate) metrics: Arc<RuntimeObservabilityCollector>,
    pub(crate) event_store: Arc<InMemoryEventStore>,
    pub(crate) config_service: Arc<ConfigService>,
    pub(crate) profiles: Arc<ProfileResolutionService>,
}

pub(crate) fn build_agent_test_harness(llm_behavior: TestLlmBehavior) -> Result<AgentTestHarness> {
    build_agent_test_harness_with_agent_config(llm_behavior, None)
}

pub(crate) fn build_agent_test_harness_with_agent_config(
    llm_behavior: TestLlmBehavior,
    agent_config: Option<astrcode_core::AgentConfig>,
) -> Result<AgentTestHarness> {
    let guard = AgentTestEnvGuard::new();
    let kernel = Arc::new(
        Kernel::builder()
            .with_capabilities(CapabilityRouter::empty())
            .with_llm_provider(Arc::new(TestLlmProvider::new(llm_behavior)))
            .with_prompt_provider(Arc::new(TestPromptProvider))
            .with_resource_provider(Arc::new(TestResourceProvider))
            .build()
            .map_err(|error| AstrError::Internal(error.to_string()))?,
    );
    let metrics = Arc::new(RuntimeObservabilityCollector::new());
    let event_store = Arc::new(InMemoryEventStore::default());
    let session_runtime = Arc::new(SessionRuntime::new(
        Arc::clone(&kernel),
        Arc::new(TestPromptFactsProvider),
        event_store.clone(),
        metrics.clone(),
    ));
    let config_store = Arc::new(TestConfigStore::default());
    if let Some(agent_config) = agent_config {
        config_store
            .config
            .lock()
            .expect("config mutex")
            .runtime
            .agent = Some(agent_config);
    }
    let config_service = Arc::new(ConfigService::new(config_store));
    let profiles = Arc::new(ProfileResolutionService::new(Arc::new(
        StaticProfileProvider::default(),
    )));
    let task_registry = Arc::new(TaskRegistry::new());
    let kernel_port: Arc<dyn AgentKernelPort> = kernel.clone();
    let session_port: Arc<dyn AgentSessionPort> = session_runtime.clone();
    let service = AgentOrchestrationService::new(
        kernel_port,
        session_port,
        config_service.clone(),
        profiles.clone(),
        Arc::new(GovernanceSurfaceAssembler::default()),
        task_registry,
        metrics.clone(),
    );

    Ok(AgentTestHarness {
        _guard: guard,
        kernel,
        session_runtime,
        service,
        metrics,
        event_store,
        config_service,
        profiles,
    })
}

pub(crate) fn sample_profile(id: &str) -> AgentProfile {
    AgentProfile {
        id: id.to_string(),
        name: id.to_string(),
        description: format!("test profile {id}"),
        mode: AgentMode::SubAgent,
        system_prompt: Some(format!("你是 {id}")),
        allowed_tools: Vec::new(),
        disallowed_tools: Vec::new(),
        model_preference: None,
    }
}

pub(crate) struct AgentTestEnvGuard {
    _temp_home: tempfile::TempDir,
    previous_test_home: Option<std::ffi::OsString>,
}

#[derive(Default)]
struct TestConfigStore {
    config: Mutex<Config>,
}

impl ConfigStore for TestConfigStore {
    fn load(&self) -> Result<Config> {
        Ok(self.config.lock().expect("config mutex").clone())
    }

    fn save(&self, config: &Config) -> Result<()> {
        *self.config.lock().expect("config mutex") = config.clone();
        Ok(())
    }

    fn path(&self) -> std::path::PathBuf {
        std::path::PathBuf::from("agent-test-config.json")
    }

    fn load_overlay(&self, _working_dir: &Path) -> Result<Option<ConfigOverlay>> {
        Ok(None)
    }

    fn save_overlay(&self, _working_dir: &Path, _overlay: &ConfigOverlay) -> Result<()> {
        Ok(())
    }

    fn load_mcp(
        &self,
        _scope: McpConfigFileScope,
        _working_dir: Option<&Path>,
    ) -> Result<Option<Value>> {
        Ok(None)
    }

    fn save_mcp(
        &self,
        _scope: McpConfigFileScope,
        _working_dir: Option<&Path>,
        _mcp: Option<&Value>,
    ) -> Result<()> {
        Ok(())
    }
}

impl AgentTestEnvGuard {
    fn new() -> Self {
        let temp_home = tempfile::tempdir().expect("temp home should be created");
        let previous_test_home = std::env::var_os(astrcode_core::home::ASTRCODE_TEST_HOME_ENV);
        std::env::set_var(
            astrcode_core::home::ASTRCODE_TEST_HOME_ENV,
            temp_home.path(),
        );
        Self {
            _temp_home: temp_home,
            previous_test_home,
        }
    }
}

impl Drop for AgentTestEnvGuard {
    fn drop(&mut self) {
        match &self.previous_test_home {
            Some(value) => std::env::set_var(astrcode_core::home::ASTRCODE_TEST_HOME_ENV, value),
            None => std::env::remove_var(astrcode_core::home::ASTRCODE_TEST_HOME_ENV),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum TestLlmBehavior {
    Succeed {
        content: String,
    },
    Stream {
        reasoning_chunks: Vec<String>,
        text_chunks: Vec<String>,
        final_content: String,
        final_reasoning: Option<String>,
    },
    Fail {
        message: String,
    },
}

#[derive(Debug)]
struct TestLlmProvider {
    behavior: TestLlmBehavior,
}

impl TestLlmProvider {
    fn new(behavior: TestLlmBehavior) -> Self {
        Self { behavior }
    }
}

#[async_trait]
impl LlmProvider for TestLlmProvider {
    async fn generate(
        &self,
        _request: LlmRequest,
        sink: Option<astrcode_core::LlmEventSink>,
    ) -> Result<LlmOutput> {
        match &self.behavior {
            TestLlmBehavior::Succeed { content } => Ok(LlmOutput {
                content: content.clone(),
                tool_calls: Vec::new(),
                reasoning: None,
                usage: None,
                finish_reason: LlmFinishReason::Stop,
            }),
            TestLlmBehavior::Stream {
                reasoning_chunks,
                text_chunks,
                final_content,
                final_reasoning,
            } => {
                if let Some(sink) = sink {
                    for chunk in reasoning_chunks {
                        sink(LlmEvent::ThinkingDelta(chunk.clone()));
                    }
                    for chunk in text_chunks {
                        sink(LlmEvent::TextDelta(chunk.clone()));
                    }
                }
                Ok(LlmOutput {
                    content: final_content.clone(),
                    tool_calls: Vec::new(),
                    reasoning: final_reasoning.clone().map(|content| ReasoningContent {
                        content,
                        signature: None,
                    }),
                    usage: None,
                    finish_reason: LlmFinishReason::Stop,
                })
            },
            TestLlmBehavior::Fail { message } => {
                Err(AstrError::Internal(format!("test llm failure: {message}")))
            },
        }
    }

    fn model_limits(&self) -> ModelLimits {
        ModelLimits {
            context_window: 32_000,
            max_output_tokens: 4_096,
        }
    }
}

#[derive(Debug)]
struct TestPromptProvider;

#[async_trait]
impl PromptProvider for TestPromptProvider {
    async fn build_prompt(&self, _request: PromptBuildRequest) -> Result<PromptBuildOutput> {
        Ok(PromptBuildOutput {
            system_prompt: "test".to_string(),
            system_prompt_blocks: Vec::new(),
            cache_metrics: Default::default(),
            metadata: Value::Null,
        })
    }
}

#[derive(Debug)]
struct TestPromptFactsProvider;

#[async_trait]
impl PromptFactsProvider for TestPromptFactsProvider {
    async fn resolve_prompt_facts(
        &self,
        _request: &astrcode_core::PromptFactsRequest,
    ) -> Result<PromptFacts> {
        Ok(PromptFacts::default())
    }
}

#[derive(Debug)]
struct TestResourceProvider;

#[async_trait]
impl ResourceProvider for TestResourceProvider {
    async fn read_resource(
        &self,
        uri: &str,
        _context: &ResourceRequestContext,
    ) -> Result<ResourceReadResult> {
        Ok(ResourceReadResult {
            uri: uri.to_string(),
            content: Value::Null,
            metadata: Value::Null,
        })
    }
}

struct StaticProfileProvider {
    profiles: Vec<AgentProfile>,
}

impl Default for StaticProfileProvider {
    fn default() -> Self {
        Self {
            profiles: vec![sample_profile("reviewer"), sample_profile("explore")],
        }
    }
}

impl ProfileProvider for StaticProfileProvider {
    fn load_for_working_dir(
        &self,
        _working_dir: &Path,
    ) -> std::result::Result<Vec<AgentProfile>, ApplicationError> {
        Ok(self.profiles.clone())
    }

    fn load_global(&self) -> std::result::Result<Vec<AgentProfile>, ApplicationError> {
        Ok(self.profiles.clone())
    }
}

#[derive(Default)]
pub(crate) struct InMemoryEventStore {
    sessions: Arc<Mutex<HashMap<String, InMemorySession>>>,
}

#[derive(Debug, Clone)]
struct InMemorySession {
    working_dir: String,
    created_at: chrono::DateTime<Utc>,
    updated_at: chrono::DateTime<Utc>,
    parent_session_id: Option<String>,
    events: Vec<StoredEvent>,
    active_turn: Option<ActiveTurnLeaseState>,
}

#[derive(Debug, Clone)]
struct ActiveTurnLeaseState {
    turn_id: String,
    acquired_at: chrono::DateTime<Utc>,
}

#[derive(Debug)]
struct InMemoryTurnLease {
    sessions: Arc<Mutex<HashMap<String, InMemorySession>>>,
    session_id: String,
    turn_id: String,
}

impl Drop for InMemoryTurnLease {
    fn drop(&mut self) {
        if let Ok(mut sessions) = self.sessions.lock() {
            if let Some(session) = sessions.get_mut(&self.session_id) {
                if session
                    .active_turn
                    .as_ref()
                    .is_some_and(|lease| lease.turn_id == self.turn_id)
                {
                    session.active_turn = None;
                }
            }
        }
    }
}

impl SessionTurnLease for InMemoryTurnLease {}

#[async_trait]
impl EventStore for InMemoryEventStore {
    async fn ensure_session(&self, session_id: &SessionId, working_dir: &Path) -> Result<()> {
        let mut sessions = self.sessions.lock().expect("event store lock should work");
        sessions
            .entry(session_id.to_string())
            .or_insert_with(|| InMemorySession {
                working_dir: working_dir.display().to_string(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
                parent_session_id: None,
                events: Vec::new(),
                active_turn: None,
            });
        Ok(())
    }

    async fn append(&self, session_id: &SessionId, event: &StorageEvent) -> Result<StoredEvent> {
        let mut sessions = self.sessions.lock().expect("event store lock should work");
        let session = sessions
            .get_mut(session_id.as_str())
            .ok_or_else(|| AstrError::SessionNotFound(session_id.to_string()))?;
        let stored = StoredEvent {
            storage_seq: session.events.len() as u64 + 1,
            event: event.clone(),
        };
        if let astrcode_core::StorageEventPayload::SessionStart {
            parent_session_id, ..
        } = &event.payload
        {
            session.parent_session_id = parent_session_id.clone();
        }
        session.updated_at = Utc::now();
        session.events.push(stored.clone());
        Ok(stored)
    }

    async fn replay(&self, session_id: &SessionId) -> Result<Vec<StoredEvent>> {
        let sessions = self.sessions.lock().expect("event store lock should work");
        let session = sessions
            .get(session_id.as_str())
            .ok_or_else(|| AstrError::SessionNotFound(session_id.to_string()))?;
        Ok(session.events.clone())
    }

    async fn try_acquire_turn(
        &self,
        session_id: &SessionId,
        turn_id: &str,
    ) -> Result<SessionTurnAcquireResult> {
        let mut sessions = self.sessions.lock().expect("event store lock should work");
        let session = sessions
            .get_mut(session_id.as_str())
            .ok_or_else(|| AstrError::SessionNotFound(session_id.to_string()))?;
        if let Some(active) = &session.active_turn {
            return Ok(SessionTurnAcquireResult::Busy(SessionTurnBusy {
                turn_id: active.turn_id.clone(),
                owner_pid: std::process::id(),
                acquired_at: active.acquired_at,
            }));
        }
        session.active_turn = Some(ActiveTurnLeaseState {
            turn_id: turn_id.to_string(),
            acquired_at: Utc::now(),
        });
        Ok(SessionTurnAcquireResult::Acquired(Box::new(
            InMemoryTurnLease {
                sessions: Arc::clone(&self.sessions),
                session_id: session_id.to_string(),
                turn_id: turn_id.to_string(),
            },
        )))
    }

    async fn list_sessions(&self) -> Result<Vec<SessionId>> {
        let sessions = self.sessions.lock().expect("event store lock should work");
        let mut ids = sessions
            .keys()
            .cloned()
            .map(SessionId::from)
            .collect::<Vec<_>>();
        ids.sort();
        Ok(ids)
    }

    async fn list_session_metas(&self) -> Result<Vec<SessionMeta>> {
        let sessions = self.sessions.lock().expect("event store lock should work");
        let mut metas = sessions
            .iter()
            .map(|(session_id, session)| SessionMeta {
                session_id: session_id.clone(),
                working_dir: session.working_dir.clone(),
                display_name: display_name_from_working_dir(Path::new(&session.working_dir)),
                title: "New Session".to_string(),
                created_at: session.created_at,
                updated_at: session.updated_at,
                parent_session_id: session.parent_session_id.clone(),
                parent_storage_seq: None,
                phase: if session.active_turn.is_some() {
                    Phase::Thinking
                } else {
                    Phase::Idle
                },
            })
            .collect::<Vec<_>>();
        metas.sort_by_key(|meta| meta.updated_at);
        Ok(metas)
    }

    async fn delete_session(&self, session_id: &SessionId) -> Result<()> {
        let mut sessions = self.sessions.lock().expect("event store lock should work");
        sessions.remove(session_id.as_str());
        Ok(())
    }

    async fn delete_sessions_by_working_dir(
        &self,
        working_dir: &str,
    ) -> Result<DeleteProjectResult> {
        let mut sessions = self.sessions.lock().expect("event store lock should work");
        let target = working_dir.replace('\\', "/");
        let to_remove = sessions
            .iter()
            .filter_map(|(session_id, session)| {
                (session.working_dir.replace('\\', "/") == target).then_some(session_id.clone())
            })
            .collect::<Vec<_>>();
        for session_id in &to_remove {
            sessions.remove(session_id);
        }
        Ok(DeleteProjectResult {
            success_count: to_remove.len(),
            failed_session_ids: Vec::new(),
        })
    }
}
