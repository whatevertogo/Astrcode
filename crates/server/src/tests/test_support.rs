use std::{
    path::Path,
    sync::{Arc, Mutex, MutexGuard, OnceLock},
};

use astrcode_core::{
    AgentEventContext, EventLogWriter, PluginRegistry, RuntimeCoordinator, RuntimeHandle,
    StorageEvent, StorageEventPayload,
};
use astrcode_runtime::{RuntimeGovernance, RuntimeService};
use astrcode_runtime_registry::CapabilityRouter;
use astrcode_storage::session::EventLog;
use chrono::Utc;

use crate::{
    AppState, FrontendBuild,
    auth::{AuthSessionManager, BootstrapAuth},
    bootstrap::APP_HOME_OVERRIDE_ENV,
};

pub(crate) fn server_test_env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub(crate) struct ServerTestEnvGuard {
    _lock: MutexGuard<'static, ()>,
    _temp_home: tempfile::TempDir,
    previous_home_override: Option<std::ffi::OsString>,
}

impl ServerTestEnvGuard {
    pub(crate) fn new() -> Self {
        let lock = match server_test_env_lock().lock() {
            Ok(lock) => lock,
            Err(poisoned) => poisoned.into_inner(),
        };
        let temp_home = tempfile::tempdir().expect("tempdir should be created");
        let previous_home_override = std::env::var_os(APP_HOME_OVERRIDE_ENV);
        std::env::set_var(APP_HOME_OVERRIDE_ENV, temp_home.path());

        Self {
            _lock: lock,
            _temp_home: temp_home,
            previous_home_override,
        }
    }

    pub(crate) fn path(&self) -> &std::path::Path {
        self._temp_home.path()
    }
}

impl Drop for ServerTestEnvGuard {
    fn drop(&mut self) {
        match &self.previous_home_override {
            Some(value) => std::env::set_var(APP_HOME_OVERRIDE_ENV, value),
            None => std::env::remove_var(APP_HOME_OVERRIDE_ENV),
        }
    }
}

pub(crate) fn test_state(frontend_build: Option<FrontendBuild>) -> (AppState, ServerTestEnvGuard) {
    let capabilities = CapabilityRouter::builder()
        .build()
        .expect("empty capability router should build");
    test_state_with_capabilities(capabilities, frontend_build)
}

pub(crate) fn test_state_with_capabilities(
    capabilities: CapabilityRouter,
    frontend_build: Option<FrontendBuild>,
) -> (AppState, ServerTestEnvGuard) {
    let guard = ServerTestEnvGuard::new();
    let service = Arc::new(
        RuntimeService::from_capabilities(capabilities).expect("runtime service should initialize"),
    );
    let runtime: Arc<dyn RuntimeHandle> = service.clone();
    let coordinator = Arc::new(RuntimeCoordinator::new(
        runtime,
        Arc::new(PluginRegistry::default()),
        Vec::new(),
    ));
    let runtime_governance = Arc::new(RuntimeGovernance::from_runtime(
        Arc::clone(&service),
        Arc::clone(&coordinator),
    ));
    let auth_sessions = Arc::new(AuthSessionManager::default());
    auth_sessions.issue_test_token("browser-token");
    (
        AppState {
            service,
            coordinator,
            runtime_governance,
            auth_sessions,
            bootstrap_auth: BootstrapAuth::new(
                "browser-token".to_string(),
                chrono::Utc::now().timestamp_millis() + 60_000,
            ),
            frontend_build,
        },
        guard,
    )
}

pub(crate) fn append_session_events(
    session_id: &str,
    working_dir: &Path,
    events: impl IntoIterator<Item = StorageEvent>,
) {
    let mut log =
        EventLog::create(session_id, working_dir).expect("session file should be created");
    for event in events {
        log.append(&event).expect("event should append");
    }
}

pub(crate) fn seed_subrun_status_contract_session(session_id: &str, working_dir: &Path) {
    let sub = AgentEventContext::sub_run(
        "agent-contract",
        "turn-contract",
        "review",
        "subrun-contract",
        astrcode_core::SubRunStorageMode::IndependentSession,
        Some("child-contract".to_string()),
    );

    append_session_events(
        session_id,
        working_dir,
        [
            StorageEvent {
                turn_id: None,
                agent: AgentEventContext::default(),
                payload: StorageEventPayload::SessionStart {
                    session_id: session_id.to_string(),
                    timestamp: Utc::now(),
                    working_dir: working_dir.display().to_string(),
                    parent_session_id: None,
                    parent_storage_seq: None,
                },
            },
            StorageEvent {
                turn_id: Some("turn-contract".to_string()),
                agent: sub.clone(),
                payload: StorageEventPayload::SubRunStarted {
                    tool_call_id: Some("call-contract".to_string()),
                    resolved_overrides: astrcode_core::ResolvedSubagentContextOverrides::default(),
                    resolved_limits: astrcode_core::ResolvedExecutionLimitsSnapshot::default(),
                    timestamp: Some(Utc::now()),
                },
            },
            StorageEvent {
                turn_id: Some("turn-contract".to_string()),
                agent: sub,
                payload: StorageEventPayload::SubRunFinished {
                    tool_call_id: Some("call-contract".to_string()),
                    result: astrcode_core::SubRunResult {
                        lifecycle: astrcode_core::AgentLifecycleStatus::Terminated,
                        last_turn_outcome: Some(astrcode_core::AgentTurnOutcome::Completed),
                        handoff: Some(astrcode_core::SubRunHandoff {
                            summary: "contract done".to_string(),
                            findings: Vec::new(),
                            artifacts: Vec::new(),
                        }),
                        failure: None,
                    },
                    step_count: 1,
                    estimated_tokens: 42,
                    timestamp: Some(Utc::now()),
                },
            },
        ],
    );
}

pub(crate) fn seed_child_delivery_contract_session(session_id: &str, working_dir: &Path) {
    let agent = AgentEventContext::sub_run(
        "agent-child-contract",
        "turn-delivery-contract",
        "review",
        "subrun-delivery-contract",
        astrcode_core::SubRunStorageMode::IndependentSession,
        Some("session-child-contract".to_string()),
    );
    let child_ref = astrcode_core::ChildAgentRef {
        agent_id: "agent-child-contract".to_string(),
        session_id: session_id.to_string(),
        sub_run_id: "subrun-delivery-contract".to_string(),
        parent_agent_id: Some("agent-parent".to_string()),
        lineage_kind: astrcode_core::ChildSessionLineageKind::Spawn,
        status: astrcode_core::AgentLifecycleStatus::Terminated,
        open_session_id: "session-child-contract".to_string(),
    };

    append_session_events(
        session_id,
        working_dir,
        [
            StorageEvent {
                turn_id: None,
                agent: AgentEventContext::default(),
                payload: StorageEventPayload::SessionStart {
                    session_id: session_id.to_string(),
                    timestamp: Utc::now(),
                    working_dir: working_dir.display().to_string(),
                    parent_session_id: None,
                    parent_storage_seq: None,
                },
            },
            StorageEvent {
                turn_id: Some("turn-delivery-contract".to_string()),
                agent: agent.clone(),
                payload: StorageEventPayload::SubRunStarted {
                    tool_call_id: Some("call-delivery-contract".to_string()),
                    resolved_overrides: astrcode_core::ResolvedSubagentContextOverrides {
                        storage_mode: astrcode_core::SubRunStorageMode::IndependentSession,
                        ..Default::default()
                    },
                    resolved_limits: astrcode_core::ResolvedExecutionLimitsSnapshot::default(),
                    timestamp: Some(Utc::now()),
                },
            },
            StorageEvent {
                turn_id: Some("turn-delivery-contract".to_string()),
                agent: agent.clone(),
                payload: StorageEventPayload::SubRunFinished {
                    tool_call_id: Some("call-delivery-contract".to_string()),
                    result: astrcode_core::SubRunResult {
                        lifecycle: astrcode_core::AgentLifecycleStatus::Terminated,
                        last_turn_outcome: Some(astrcode_core::AgentTurnOutcome::Completed),
                        handoff: Some(astrcode_core::SubRunHandoff {
                            summary: "child final reply summary".to_string(),
                            findings: Vec::new(),
                            artifacts: Vec::new(),
                        }),
                        failure: None,
                    },
                    step_count: 2,
                    estimated_tokens: 88,
                    timestamp: Some(Utc::now()),
                },
            },
            StorageEvent {
                turn_id: Some("turn-delivery-contract".to_string()),
                agent,
                payload: StorageEventPayload::ChildSessionNotification {
                    notification: astrcode_core::ChildSessionNotification {
                        notification_id: "child-terminal:subrun-delivery-contract:completed"
                            .to_string(),
                        child_ref,
                        kind: astrcode_core::ChildSessionNotificationKind::Delivered,
                        summary: "child final reply summary".to_string(),
                        status: astrcode_core::AgentLifecycleStatus::Terminated,
                        source_tool_call_id: Some("call-delivery-contract".to_string()),
                        final_reply_excerpt: Some("final answer excerpt".to_string()),
                    },
                    timestamp: Some(Utc::now()),
                },
            },
        ],
    );
}
