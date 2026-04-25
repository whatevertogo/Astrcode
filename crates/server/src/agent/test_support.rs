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
    AgentLifecycleStatus, AgentMode, AgentProfile, AstrError, Config, ConfigOverlay,
    DeleteProjectResult, Phase, Result, SessionId, SessionMeta, SessionTurnAcquireResult,
    SessionTurnBusy, SessionTurnLease, StorageEvent, StoredEvent, SubRunStorageMode,
    ports::{ConfigStore, McpConfigFileScope},
};
use astrcode_host_session::{
    EventStore, SessionCatalog, SubRunHandle, catalog::display_name_from_working_dir,
};
use astrcode_llm_contract::{
    LlmEventSink, LlmFinishReason, LlmOutput, LlmProvider, LlmRequest, ModelLimits,
};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;

use crate::{
    AgentKernelPort, AgentOrchestrationService, AgentSessionPort, ApplicationError, ConfigService,
    GovernanceSurfaceAssembler, ProfileResolutionService, RuntimeObservabilityCollector,
    agent_control_bridge::ServerLiveSubRunStatus,
    execution::ProfileProvider,
    lifecycle::TaskRegistry,
    mode::builtin_mode_specs,
    mode_catalog_service::ServerModeCatalog,
    session_runtime_owner_bridge::{
        ServerAgentControlLimits, ServerRuntimeTestSupport, ServerSessionRuntimeBootstrapInput,
        bootstrap_session_runtime,
    },
};

pub(crate) struct AgentTestHarness {
    pub(crate) _guard: AgentTestEnvGuard,
    pub(crate) session_runtime: AgentTestRuntimeHandle,
    pub(crate) service: AgentOrchestrationService,
    pub(crate) metrics: Arc<RuntimeObservabilityCollector>,
    _runtime_keepalive: Arc<dyn std::any::Any + Send + Sync>,
}

#[derive(Clone)]
pub(crate) struct AgentTestRuntimeHandle {
    session_catalog: Arc<SessionCatalog>,
    test_support: Arc<dyn ServerRuntimeTestSupport>,
    agent_kernel: Arc<dyn AgentKernelPort>,
}

impl AgentTestRuntimeHandle {
    pub(crate) fn agent(&self) -> &Self {
        self
    }

    pub(crate) fn agent_control(&self) -> &Self {
        self
    }

    pub(crate) async fn create_session(&self, working_dir: String) -> Result<SessionMeta> {
        self.session_catalog.create_session(working_dir).await
    }

    pub(crate) async fn replay_stored_events(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<StoredEvent>> {
        self.test_support
            .replay_stored_events(session_id.as_str())
            .await
    }

    pub(crate) async fn list_session_metas(&self) -> Result<Vec<SessionMeta>> {
        self.session_catalog.list_session_metas().await
    }

    pub(crate) async fn list_session_ids(&self) -> Result<Vec<SessionId>> {
        Ok(self
            .session_catalog
            .list_session_metas()
            .await?
            .into_iter()
            .map(|meta| SessionId::from(meta.session_id))
            .collect())
    }

    pub(crate) async fn get_lifecycle(
        &self,
        sub_run_or_agent_id: &str,
    ) -> Option<AgentLifecycleStatus> {
        self.agent_kernel.get_lifecycle(sub_run_or_agent_id).await
    }

    pub(crate) async fn get_agent_handle(&self, sub_run_or_agent_id: &str) -> Option<SubRunHandle> {
        self.agent_kernel.get_handle(sub_run_or_agent_id).await
    }

    pub(crate) async fn get_handle(&self, sub_run_or_agent_id: &str) -> Option<SubRunHandle> {
        self.get_agent_handle(sub_run_or_agent_id).await
    }

    pub(crate) async fn register_root_agent(
        &self,
        agent_id: impl Into<String>,
        session_id: impl Into<String>,
        profile_id: impl Into<String>,
    ) -> Result<SubRunHandle> {
        self.test_support
            .register_root_agent(agent_id.into(), session_id.into(), profile_id.into())
            .await
            .map_err(|error| AstrError::Validation(error.to_string()))
    }

    pub(crate) async fn spawn_independent_child(
        &self,
        profile: &AgentProfile,
        session_id: impl Into<String>,
        child_session_id: impl Into<String>,
        parent_turn_id: impl Into<String>,
        parent_agent_id: impl Into<String>,
    ) -> Result<SubRunHandle> {
        self.test_support
            .spawn_independent_child(
                profile,
                session_id.into(),
                child_session_id.into(),
                parent_turn_id.into(),
                parent_agent_id.into(),
            )
            .await
            .map_err(|error| AstrError::Validation(error.to_string()))
    }

    pub(crate) async fn spawn_with_storage(
        &self,
        profile: &AgentProfile,
        session_id: String,
        child_session_id: Option<String>,
        parent_turn_id: String,
        parent_agent_id: Option<String>,
        storage_mode: SubRunStorageMode,
    ) -> Result<SubRunHandle> {
        if !matches!(storage_mode, SubRunStorageMode::IndependentSession) {
            return Err(AstrError::Validation(format!(
                "agent test runtime only supports independent child storage, got {storage_mode:?}"
            )));
        }
        let child_session_id = child_session_id.ok_or_else(|| {
            AstrError::Validation(
                "agent test runtime requires an explicit child session id for spawn".to_string(),
            )
        })?;
        let parent_agent_id = parent_agent_id.ok_or_else(|| {
            AstrError::Validation(
                "agent test runtime requires an explicit parent agent id for spawn".to_string(),
            )
        })?;
        self.spawn_independent_child(
            profile,
            session_id,
            child_session_id,
            parent_turn_id,
            parent_agent_id,
        )
        .await
    }

    pub(crate) async fn set_lifecycle(
        &self,
        sub_run_or_agent_id: &str,
        lifecycle: AgentLifecycleStatus,
    ) -> Result<()> {
        self.test_support
            .set_lifecycle(sub_run_or_agent_id, lifecycle)
            .await
            .ok_or_else(|| {
                AstrError::Internal(format!(
                    "agent '{sub_run_or_agent_id}' disappeared before lifecycle update"
                ))
            })
    }

    pub(crate) async fn pending_parent_delivery_count(&self, parent_session_id: &str) -> usize {
        self.test_support
            .pending_parent_delivery_count(parent_session_id)
            .await
    }

    pub(crate) async fn query_root_status(
        &self,
        session_id: &str,
    ) -> Option<ServerLiveSubRunStatus> {
        self.test_support.query_root_status(session_id).await
    }
}

impl AgentTestHarness {
    pub(crate) async fn append_events_to_session(
        &self,
        session_id: &str,
        _phase: Phase,
        events: &[StorageEvent],
    ) -> Result<()> {
        for event in events {
            self.session_runtime
                .test_support
                .append_event(session_id, event.clone())
                .await?;
        }
        Ok(())
    }

    pub(crate) async fn prepare_busy_turn(&self, session_id: &str, turn_id: &str) -> Result<u64> {
        self.session_runtime
            .test_support
            .prepare_test_turn_runtime(session_id, turn_id)
            .await
    }

    pub(crate) async fn complete_turn_state(
        &self,
        session_id: &str,
        generation: u64,
        _phase: Phase,
    ) -> Result<()> {
        self.session_runtime
            .test_support
            .complete_test_turn_runtime(session_id, generation)
            .await
    }
}

pub(crate) fn build_agent_test_harness(llm_behavior: TestLlmBehavior) -> Result<AgentTestHarness> {
    build_agent_test_harness_with_agent_config(llm_behavior, None)
}

pub(crate) fn build_agent_test_harness_with_agent_config(
    llm_behavior: TestLlmBehavior,
    agent_config: Option<astrcode_core::AgentConfig>,
) -> Result<AgentTestHarness> {
    let guard = AgentTestEnvGuard::new();
    let metrics = Arc::new(RuntimeObservabilityCollector::new());
    let event_store = Arc::new(InMemoryEventStore::default());
    let session_catalog = Arc::new(SessionCatalog::new(event_store.clone()));
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
    let builtin_mode_specs = builtin_mode_specs();
    let bootstrapped_runtime = bootstrap_session_runtime(ServerSessionRuntimeBootstrapInput {
        capability_invokers: Vec::new(),
        llm_provider: Arc::new(TestLlmProvider::new(llm_behavior)),
        session_catalog: Arc::clone(&session_catalog),
        mode_catalog: ServerModeCatalog::from_mode_specs(builtin_mode_specs, Vec::new())?,
        agent_limits: ServerAgentControlLimits {
            max_depth: 8,
            max_concurrent: 8,
            finalized_retain_limit: 64,
            inbox_capacity: 64,
            parent_delivery_capacity: 64,
        },
        hook_dispatcher: None,
        hook_snapshot_id: "test-snapshot".to_string(),
    })?;
    let kernel_port: Arc<dyn AgentKernelPort> = bootstrapped_runtime.agent_kernel.clone();
    let session_port: Arc<dyn AgentSessionPort> = bootstrapped_runtime.agent_sessions.clone();
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
        session_runtime: AgentTestRuntimeHandle {
            session_catalog,
            test_support: bootstrapped_runtime.test_support,
            agent_kernel: bootstrapped_runtime.agent_kernel.clone(),
        },
        service,
        metrics,
        _runtime_keepalive: bootstrapped_runtime.keepalive,
    })
}

pub(crate) fn sample_profile(id: &str) -> AgentProfile {
    AgentProfile {
        id: id.to_string(),
        name: id.to_string(),
        description: format!("test profile {id}"),
        mode: AgentMode::SubAgent,
        system_prompt: Some(format!("你是 {id}")),
        model_preference: None,
    }
}

pub(crate) struct AgentTestEnvGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
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
        let lock = astrcode_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp_home = tempfile::tempdir().expect("temp home should be created");
        let previous_test_home = std::env::var_os(astrcode_core::env::ASTRCODE_TEST_HOME_ENV);
        std::env::set_var(astrcode_core::env::ASTRCODE_TEST_HOME_ENV, temp_home.path());
        Self {
            _lock: lock,
            _temp_home: temp_home,
            previous_test_home,
        }
    }
}

impl Drop for AgentTestEnvGuard {
    fn drop(&mut self) {
        match &self.previous_test_home {
            Some(value) => std::env::set_var(astrcode_core::env::ASTRCODE_TEST_HOME_ENV, value),
            None => std::env::remove_var(astrcode_core::env::ASTRCODE_TEST_HOME_ENV),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum TestLlmBehavior {
    Succeed { content: String },
    Fail { message: String },
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
        _sink: Option<LlmEventSink>,
    ) -> Result<LlmOutput> {
        match &self.behavior {
            TestLlmBehavior::Succeed { content } => Ok(LlmOutput {
                content: content.clone(),
                tool_calls: Vec::new(),
                reasoning: None,
                usage: None,
                finish_reason: LlmFinishReason::Stop,
                prompt_cache_diagnostics: None,
            }),
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
