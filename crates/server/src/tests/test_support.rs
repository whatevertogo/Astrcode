use std::{
    collections::HashSet,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use astrcode_adapter_storage::session::EventLog;
use astrcode_application::{ApplicationError, WatchEvent, WatchPort, WatchService, WatchSource};
use astrcode_core::{
    AgentEventContext, AgentLifecycleStatus, ChildAgentRef, ChildSessionNotification,
    ChildSessionNotificationKind, EventLogWriter, ResolvedExecutionLimitsSnapshot,
    ResolvedSubagentContextOverrides, StorageEvent, StorageEventPayload, SubRunResult,
    SubRunStorageMode,
};
use chrono::Utc;
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
    let auth_sessions = Arc::new(AuthSessionManager::default());
    auth_sessions.issue_test_token("browser-token");

    (
        AppState {
            app: runtime.app,
            governance: runtime.governance,
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

// ─── 契约测试辅助函数 ──────────────────────────────────────

/// 植入一个 subrun 状态契约测试所需的会话事件日志。
/// 创建一个已完成的子执行，包含 step_count 和 estimated_tokens。
#[allow(dead_code)]
pub(crate) fn seed_subrun_status_contract_session(session_id: &str, working_dir: &std::path::Path) {
    let mut log =
        EventLog::create(session_id, working_dir).expect("session file should be created");

    let agent_ctx = AgentEventContext::sub_run(
        "agent-contract",
        "turn-contract",
        "explore",
        "subrun-contract",
        None,
        SubRunStorageMode::IndependentSession,
        None,
    );

    let events = [
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
            agent: agent_ctx.clone(),
            payload: StorageEventPayload::SubRunStarted {
                tool_call_id: Some("call-contract".to_string()),
                resolved_overrides: ResolvedSubagentContextOverrides::default(),
                resolved_limits: ResolvedExecutionLimitsSnapshot::default(),
                timestamp: Some(Utc::now()),
            },
        },
        StorageEvent {
            turn_id: Some("turn-contract".to_string()),
            agent: agent_ctx,
            payload: StorageEventPayload::SubRunFinished {
                tool_call_id: Some("call-contract".to_string()),
                result: SubRunResult {
                    lifecycle: AgentLifecycleStatus::Terminated,
                    last_turn_outcome: Some(astrcode_core::AgentTurnOutcome::Completed),
                    handoff: None,
                    failure: None,
                },
                step_count: 1,
                estimated_tokens: 42,
                timestamp: Some(Utc::now()),
            },
        },
    ];

    for event in events {
        log.append(&event).expect("event should append");
    }
}

/// 植入一个 child delivery 契约测试所需的会话事件日志。
/// 创建一个已交付的子会话通知，包含 final_reply_excerpt。
#[allow(dead_code)]
pub(crate) fn seed_child_delivery_contract_session(
    session_id: &str,
    working_dir: &std::path::Path,
) {
    let mut log =
        EventLog::create(session_id, working_dir).expect("session file should be created");

    let child_ref = ChildAgentRef {
        agent_id: "agent-child-contract".to_string(),
        session_id: session_id.to_string(),
        sub_run_id: "subrun-delivery-contract".to_string(),
        parent_agent_id: Some("agent-parent".to_string()),
        parent_sub_run_id: Some("subrun-parent".to_string()),
        lineage_kind: astrcode_core::ChildSessionLineageKind::Spawn,
        status: AgentLifecycleStatus::Terminated,
        open_session_id: "session-child-contract".to_string(),
    };

    let agent_ctx = AgentEventContext::sub_run(
        "agent-child-contract",
        "turn-parent",
        "explore",
        "subrun-delivery-contract",
        Some("subrun-parent".to_string()),
        SubRunStorageMode::IndependentSession,
        Some("session-child-contract".to_string()),
    );

    let events = [
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
            turn_id: Some("turn-parent".to_string()),
            agent: agent_ctx.clone(),
            payload: StorageEventPayload::SubRunStarted {
                tool_call_id: Some("call-delivery".to_string()),
                resolved_overrides: ResolvedSubagentContextOverrides {
                    storage_mode: SubRunStorageMode::IndependentSession,
                    ..Default::default()
                },
                resolved_limits: ResolvedExecutionLimitsSnapshot::default(),
                timestamp: Some(Utc::now()),
            },
        },
        StorageEvent {
            turn_id: Some("turn-parent".to_string()),
            agent: agent_ctx.clone(),
            payload: StorageEventPayload::SubRunFinished {
                tool_call_id: Some("call-delivery".to_string()),
                result: SubRunResult {
                    lifecycle: AgentLifecycleStatus::Terminated,
                    last_turn_outcome: Some(astrcode_core::AgentTurnOutcome::Completed),
                    handoff: None,
                    failure: None,
                },
                step_count: 1,
                estimated_tokens: 10,
                timestamp: Some(Utc::now()),
            },
        },
        StorageEvent {
            turn_id: Some("turn-parent".to_string()),
            agent: agent_ctx,
            payload: StorageEventPayload::ChildSessionNotification {
                notification: ChildSessionNotification {
                    notification_id: "child-terminal:subrun-delivery-contract:delivered"
                        .to_string(),
                    child_ref,
                    kind: ChildSessionNotificationKind::Delivered,
                    summary: "子 Agent 任务完成".to_string(),
                    status: AgentLifecycleStatus::Terminated,
                    source_tool_call_id: Some("call-delivery".to_string()),
                    final_reply_excerpt: Some("final answer excerpt".to_string()),
                },
                timestamp: Some(Utc::now()),
            },
        },
    ];

    for event in events {
        log.append(&event).expect("event should append");
    }
}
