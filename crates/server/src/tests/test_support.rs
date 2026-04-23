use std::{
    collections::HashSet,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use astrcode_application::{ApplicationError, WatchEvent, WatchPort, WatchService, WatchSource};
use astrcode_core::{
    AgentEventContext, EventTranslator, SessionId, StorageEvent, StorageEventPayload,
    TurnTerminalKind, UserMessageOrigin,
};
use tokio::sync::broadcast;

use crate::{
    AppState, FrontendBuild,
    auth::{AuthSessionManager, BootstrapAuth},
    bootstrap::{ServerBootstrapOptions, bootstrap_server_runtime_with_options},
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
    ) -> Result<(), ApplicationError> {
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

    fn stop_all(&self) -> Result<(), ApplicationError> {
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

    fn add_source(&self, source: WatchSource) -> Result<(), ApplicationError> {
        self.sources
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(source);
        Ok(())
    }

    fn remove_source(&self, source: &WatchSource) -> Result<(), ApplicationError> {
        self.sources
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(source);
        Ok(())
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
    let app = Arc::clone(&runtime.app);
    let governance = Arc::clone(&runtime.governance);
    let auth_sessions = Arc::new(AuthSessionManager::default());
    auth_sessions.issue_test_token("browser-token");

    (
        AppState {
            app,
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
    let session_state = state
        ._runtime_handles
        .session_runtime
        .get_session_state(&SessionId::from(session_id.to_string()))
        .await
        .expect("session state should load");
    let mut translator = EventTranslator::new(
        session_state
            .current_phase()
            .expect("session phase should be readable"),
    );
    let stored = session_state
        .writer
        .clone()
        .append(event)
        .await
        .expect("event should append");
    let records = session_state
        .translate_store_and_cache(&stored, &mut translator)
        .expect("event should translate");
    for record in records {
        let _ = session_state.broadcaster.send(record);
    }
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
        .session_runtime
        .prepare_test_turn_runtime(session_id, "test-running-turn")
        .await
        .expect("session should enter running state");
}

pub(crate) async fn stored_events_for_session(
    state: &crate::AppState,
    session_id: &str,
) -> Vec<astrcode_core::StoredEvent> {
    state
        ._runtime_handles
        .session_runtime
        .replay_stored_events(&SessionId::from(session_id.to_string()))
        .await
        .expect("events should replay")
}
