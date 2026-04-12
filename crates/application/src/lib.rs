use std::{collections::HashMap, path::Path, sync::Arc};

use astrcode_core::{
    AgentEvent, AgentEventContext, AgentId, DeleteProjectResult, ExecutionAccepted, Phase,
    SessionEventRecord, SessionId, SessionMeta, TurnId, config::Config, event::generate_session_id,
};
use astrcode_kernel::Kernel;
use astrcode_session_runtime::SessionRuntime;
use tokio::sync::{RwLock, broadcast};

pub mod composer;
pub mod config;
pub mod errors;
pub mod lifecycle;
pub mod mcp;
pub mod observability;
pub mod watch;

pub use composer::{ComposerOption, ComposerOptionKind, ComposerOptionsRequest};
pub use config::{
    ConfigService, TestConnectionResult, is_env_var_name, list_model_options,
    resolve_active_selection, resolve_current_model,
};
pub use errors::ApplicationError;
pub use lifecycle::governance::{
    AppGovernance, ObservabilitySnapshotProvider, RuntimeReloader, SessionInfoProvider,
};
pub use mcp::{McpConfigScope, McpServerConfig, McpServerStatusSnapshot, McpTransportConfig};
pub use observability::{
    ExecutionDiagnosticsSnapshot, GovernanceSnapshot, OperationMetricsSnapshot, ReloadResult,
    ReplayMetricsSnapshot, ReplayPath, RuntimeObservabilitySnapshot,
    SubRunExecutionMetricsSnapshot,
};

#[derive(Debug)]
pub struct SessionReplay {
    pub history: Vec<SessionEventRecord>,
    pub receiver: broadcast::Receiver<SessionEventRecord>,
    pub live_receiver: broadcast::Receiver<AgentEvent>,
}

#[derive(Debug, Clone)]
pub struct SessionHistorySnapshot {
    pub history: Vec<SessionEventRecord>,
    pub cursor: Option<String>,
    pub phase: Phase,
}

#[derive(Debug, Clone)]
pub struct SessionViewSnapshot {
    pub focus_history: Vec<SessionEventRecord>,
    pub direct_children_history: Vec<SessionEventRecord>,
    pub cursor: Option<String>,
    pub phase: Phase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionCatalogEvent {
    SessionCreated {
        session_id: String,
    },
    SessionDeleted {
        session_id: String,
    },
    ProjectDeleted {
        working_dir: String,
    },
    SessionBranched {
        session_id: String,
        source_session_id: String,
    },
}

#[derive(Debug, Clone)]
struct SessionEntry {
    meta: SessionMeta,
    history: Vec<SessionEventRecord>,
    next_seq: u64,
    durable_tx: broadcast::Sender<SessionEventRecord>,
    live_tx: broadcast::Sender<AgentEvent>,
}

/// 唯一业务用例入口。
pub struct App {
    kernel: Arc<Kernel>,
    session_runtime: Arc<SessionRuntime>,
    sessions: Arc<RwLock<HashMap<String, SessionEntry>>>,
    catalog_events: broadcast::Sender<SessionCatalogEvent>,
    config_service: Arc<ConfigService>,
    composer_service: Arc<composer::ComposerService>,
    mcp_service: Arc<mcp::McpService>,
}

impl App {
    pub fn new(kernel: Arc<Kernel>, session_runtime: Arc<SessionRuntime>) -> Self {
        let (catalog_events, _) = broadcast::channel(256);
        Self {
            kernel,
            session_runtime,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            catalog_events,
            config_service: Arc::new(ConfigService::default()),
            composer_service: Arc::new(composer::ComposerService),
            mcp_service: Arc::new(mcp::McpService),
        }
    }

    pub fn kernel(&self) -> &Arc<Kernel> {
        &self.kernel
    }

    pub fn session_runtime(&self) -> &Arc<SessionRuntime> {
        &self.session_runtime
    }

    pub fn config(&self) -> &Arc<ConfigService> {
        &self.config_service
    }

    pub fn mcp(&self) -> &Arc<mcp::McpService> {
        &self.mcp_service
    }

    pub fn composer(&self) -> &Arc<composer::ComposerService> {
        &self.composer_service
    }

    pub fn subscribe_catalog(&self) -> broadcast::Receiver<SessionCatalogEvent> {
        self.catalog_events.subscribe()
    }

    pub async fn list_sessions(&self) -> Vec<SessionMeta> {
        let sessions = self.sessions.read().await;
        let mut metas = sessions
            .values()
            .map(|entry| entry.meta.clone())
            .collect::<Vec<_>>();
        metas.sort_by_key(|left| left.updated_at);
        metas
    }

    pub async fn create_session(
        &self,
        working_dir: impl Into<String>,
    ) -> Result<SessionMeta, ApplicationError> {
        let working_dir = working_dir.into();
        self.validate_non_empty("workingDir", &working_dir)?;
        let session_id = generate_session_id();
        let created_at = chrono::Utc::now();
        let session_meta = SessionMeta {
            session_id: session_id.clone(),
            working_dir: working_dir.clone(),
            display_name: display_name_from_working_dir(&working_dir),
            title: "New Session".to_string(),
            created_at,
            updated_at: created_at,
            parent_session_id: None,
            parent_storage_seq: None,
            phase: Phase::Idle,
        };

        let (durable_tx, _) = broadcast::channel(2048);
        let (live_tx, _) = broadcast::channel(2048);
        let mut entry = SessionEntry {
            meta: session_meta.clone(),
            history: Vec::new(),
            next_seq: 1,
            durable_tx,
            live_tx,
        };
        entry.push_event(AgentEvent::SessionStarted {
            session_id: session_id.clone(),
        });

        let root_agent_id: AgentId = "root-agent".into();
        let session_runtime_id: SessionId = session_id.clone().into();
        let _ = self
            .session_runtime
            .create_session(session_runtime_id, working_dir, root_agent_id);

        self.sessions
            .write()
            .await
            .insert(session_id.clone(), entry);
        let _ = self
            .catalog_events
            .send(SessionCatalogEvent::SessionCreated { session_id });
        Ok(session_meta)
    }

    pub async fn delete_session(&self, session_id: &str) -> Result<(), ApplicationError> {
        let mut sessions = self.sessions.write().await;
        if sessions.remove(session_id).is_none() {
            return Err(ApplicationError::NotFound(format!(
                "session '{}' not found",
                session_id
            )));
        }
        let _ = self
            .catalog_events
            .send(SessionCatalogEvent::SessionDeleted {
                session_id: session_id.to_string(),
            });
        let runtime_id: SessionId = session_id.to_string().into();
        let _ = self.session_runtime.remove_session(&runtime_id);
        Ok(())
    }

    pub async fn delete_project(
        &self,
        working_dir: &str,
    ) -> Result<DeleteProjectResult, ApplicationError> {
        let mut sessions = self.sessions.write().await;
        let to_delete = sessions
            .iter()
            .filter_map(|(session_id, entry)| {
                (normalize_path(&entry.meta.working_dir) == normalize_path(working_dir))
                    .then_some(session_id.clone())
            })
            .collect::<Vec<_>>();

        let success_count = to_delete.len();
        for session_id in &to_delete {
            sessions.remove(session_id);
        }
        let _ = self
            .catalog_events
            .send(SessionCatalogEvent::ProjectDeleted {
                working_dir: working_dir.to_string(),
            });
        Ok(DeleteProjectResult {
            success_count,
            failed_session_ids: Vec::new(),
        })
    }

    pub async fn submit_prompt(
        &self,
        session_id: &str,
        text: String,
    ) -> Result<ExecutionAccepted, ApplicationError> {
        self.validate_non_empty("prompt", &text)?;
        let mut sessions = self.sessions.write().await;
        let entry = sessions.get_mut(session_id).ok_or_else(|| {
            ApplicationError::NotFound(format!("session '{}' not found", session_id))
        })?;
        let turn_id = format!("turn-{}", entry.next_seq);

        entry.meta.phase = Phase::Thinking;
        entry.meta.updated_at = chrono::Utc::now();
        entry.push_event(AgentEvent::UserMessage {
            turn_id: turn_id.clone(),
            agent: AgentEventContext::default(),
            content: text.clone(),
        });
        entry.push_live(AgentEvent::ThinkingDelta {
            turn_id: turn_id.clone(),
            agent: AgentEventContext::default(),
            delta: "thinking...".to_string(),
        });
        entry.push_event(AgentEvent::AssistantMessage {
            turn_id: turn_id.clone(),
            agent: AgentEventContext::default(),
            content: format!("Echo: {}", text),
            reasoning_content: None,
        });
        entry.push_event(AgentEvent::TurnDone {
            turn_id: turn_id.clone(),
            agent: AgentEventContext::default(),
        });
        entry.meta.phase = Phase::Idle;
        entry.meta.updated_at = chrono::Utc::now();

        Ok(ExecutionAccepted {
            session_id: SessionId::from(session_id.to_string()),
            turn_id: TurnId::from(turn_id),
            agent_id: None,
            branched_from_session_id: None,
        })
    }

    pub async fn interrupt_session(&self, session_id: &str) -> Result<(), ApplicationError> {
        let mut sessions = self.sessions.write().await;
        let entry = sessions.get_mut(session_id).ok_or_else(|| {
            ApplicationError::NotFound(format!("session '{}' not found", session_id))
        })?;
        entry.push_event(AgentEvent::Error {
            turn_id: None,
            agent: AgentEventContext::default(),
            code: "interrupted".to_string(),
            message: "session interrupted".to_string(),
        });
        entry.meta.phase = Phase::Interrupted;
        entry.meta.updated_at = chrono::Utc::now();
        Ok(())
    }

    pub async fn compact_session(&self, session_id: &str) -> Result<(), ApplicationError> {
        let mut sessions = self.sessions.write().await;
        let entry = sessions.get_mut(session_id).ok_or_else(|| {
            ApplicationError::NotFound(format!("session '{}' not found", session_id))
        })?;
        entry.push_event(AgentEvent::CompactApplied {
            turn_id: None,
            agent: AgentEventContext::default(),
            trigger: astrcode_core::CompactTrigger::Manual,
            summary: "compacted".to_string(),
            preserved_recent_turns: 1,
        });
        entry.meta.updated_at = chrono::Utc::now();
        Ok(())
    }

    pub async fn session_history(
        &self,
        session_id: &str,
    ) -> Result<SessionHistorySnapshot, ApplicationError> {
        let sessions = self.sessions.read().await;
        let entry = sessions.get(session_id).ok_or_else(|| {
            ApplicationError::NotFound(format!("session '{}' not found", session_id))
        })?;
        Ok(SessionHistorySnapshot {
            history: entry.history.clone(),
            cursor: entry.history.last().map(|event| event.event_id.clone()),
            phase: entry.meta.phase,
        })
    }

    pub async fn session_view(
        &self,
        session_id: &str,
    ) -> Result<SessionViewSnapshot, ApplicationError> {
        let history = self.session_history(session_id).await?;
        Ok(SessionViewSnapshot {
            focus_history: history.history.clone(),
            direct_children_history: Vec::new(),
            cursor: history.cursor,
            phase: history.phase,
        })
    }

    pub async fn session_replay(
        &self,
        session_id: &str,
        last_event_id: Option<&str>,
    ) -> Result<SessionReplay, ApplicationError> {
        let sessions = self.sessions.read().await;
        let entry = sessions.get(session_id).ok_or_else(|| {
            ApplicationError::NotFound(format!("session '{}' not found", session_id))
        })?;
        let history = entry
            .history
            .iter()
            .filter(|record| {
                match (
                    parse_event_id(&record.event_id),
                    last_event_id.and_then(parse_event_id),
                ) {
                    (Some(current), Some(last)) => current > last,
                    (Some(_), None) => true,
                    _ => false,
                }
            })
            .cloned()
            .collect::<Vec<_>>();
        Ok(SessionReplay {
            history,
            receiver: entry.durable_tx.subscribe(),
            live_receiver: entry.live_tx.subscribe(),
        })
    }

    pub async fn list_composer_options(
        &self,
        request: ComposerOptionsRequest,
    ) -> Vec<ComposerOption> {
        self.composer_service.list_options(request)
    }

    pub async fn get_config(&self) -> Config {
        self.config_service.get_config().await
    }

    pub fn validate_non_empty(
        &self,
        field: &'static str,
        value: &str,
    ) -> Result<(), ApplicationError> {
        if value.trim().is_empty() {
            return Err(ApplicationError::InvalidArgument(format!(
                "field '{}' must not be empty",
                field
            )));
        }
        Ok(())
    }

    pub fn require_permission(
        &self,
        allowed: bool,
        reason: impl Into<String>,
    ) -> Result<(), ApplicationError> {
        if allowed {
            return Ok(());
        }
        Err(ApplicationError::PermissionDenied(reason.into()))
    }
}

impl SessionEntry {
    fn push_event(&mut self, event: AgentEvent) {
        let event_id = format!("{}.0", self.next_seq);
        self.next_seq += 1;
        let record = SessionEventRecord { event_id, event };
        self.history.push(record.clone());
        let _ = self.durable_tx.send(record);
    }

    fn push_live(&self, event: AgentEvent) {
        let _ = self.live_tx.send(event);
    }
}

fn parse_event_id(raw: &str) -> Option<(u64, u32)> {
    let (storage_seq, subindex) = raw.split_once('.')?;
    Some((storage_seq.parse().ok()?, subindex.parse().ok()?))
}

fn normalize_path(value: &str) -> String {
    value.replace('\\', "/").trim_end_matches('/').to_string()
}

fn display_name_from_working_dir(working_dir: &str) -> String {
    Path::new(working_dir)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("workspace")
        .to_string()
}
