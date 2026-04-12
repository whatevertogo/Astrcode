//! # 服务契约类型 (Service Contracts)
//!
//! 这里集中放置 runtime service 对外暴露的稳定契约：
//! - 错误类型与 HTTP 语义映射
//! - replay / session catalog / prompt accepted 等服务返回值
//! - composer / subrun 查询等上层协议会直接消费的结构

use std::fmt::{Display, Formatter};

pub use astrcode_core::SessionEventRecord;
use astrcode_core::{
    AgentEvent, AstrError, Phase, ResolvedExecutionLimitsSnapshot,
    ResolvedSubagentContextOverrides, StoreError, SubRunHandle, SubRunResult,
};
use async_trait::async_trait;
use tokio::sync::broadcast;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubRunEventScope {
    SelfOnly,
    Subtree,
    DirectChildren,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEventFilterSpec {
    pub target_sub_run_id: String,
    pub scope: SubRunEventScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubRunStatusSource {
    Live,
    Durable,
}

#[derive(Debug, Clone)]
pub struct SubRunStatusSnapshot {
    pub handle: SubRunHandle,
    pub tool_call_id: Option<String>,
    pub source: SubRunStatusSource,
    pub result: Option<SubRunResult>,
    pub step_count: Option<u32>,
    pub estimated_tokens: Option<u64>,
    pub resolved_overrides: Option<ResolvedSubagentContextOverrides>,
    pub resolved_limits: Option<ResolvedExecutionLimitsSnapshot>,
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

#[async_trait]
pub trait SessionReplaySource {
    async fn replay(
        &self,
        session_id: &str,
        last_event_id: Option<&str>,
    ) -> ServiceResult<SessionReplay>;
}

#[derive(Debug)]
pub enum ServiceError {
    NotFound(String),
    Conflict(String),
    InvalidInput(String),
    Internal(AstrError),
}

impl Display for ServiceError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(message) | Self::Conflict(message) | Self::InvalidInput(message) => {
                f.write_str(message)
            },
            Self::Internal(error) => Display::fmt(error, f),
        }
    }
}

impl std::error::Error for ServiceError {}

impl From<anyhow::Error> for ServiceError {
    fn from(value: anyhow::Error) -> Self {
        let value = match value.downcast::<ServiceError>() {
            Ok(service_error) => return service_error,
            Err(value) => value,
        };
        match value.downcast::<AstrError>() {
            Ok(astr_error) => Self::from(astr_error),
            Err(other) => Self::Internal(AstrError::Internal(other.to_string())),
        }
    }
}

impl From<AstrError> for ServiceError {
    fn from(value: AstrError) -> Self {
        match &value {
            AstrError::SessionNotFound(id) => Self::NotFound(format!("session not found: {}", id)),
            AstrError::ProjectNotFound(id) => Self::NotFound(format!("project not found: {}", id)),
            AstrError::TurnInProgress(id) => {
                Self::Conflict(format!("turn already in progress: {}", id))
            },
            AstrError::Validation(msg) => Self::InvalidInput(msg.clone()),
            AstrError::InvalidSessionId(id) => {
                Self::InvalidInput(format!("invalid session id: {}", id))
            },
            AstrError::MissingApiKey(profile) => {
                Self::InvalidInput(format!("missing api key for profile: {}", profile))
            },
            AstrError::MissingBaseUrl(profile) => {
                Self::InvalidInput(format!("missing base url for profile: {}", profile))
            },
            _ => Self::Internal(value),
        }
    }
}

impl From<StoreError> for ServiceError {
    fn from(value: StoreError) -> Self {
        match value {
            StoreError::SessionNotFound(id) => Self::NotFound(format!("session not found: {}", id)),
            StoreError::InvalidSessionId(id) => {
                Self::InvalidInput(format!("invalid session id: {}", id))
            },
            StoreError::Io { context, .. } => Self::Internal(AstrError::Internal(context)),
            StoreError::Parse { context, .. } => Self::Internal(AstrError::Internal(context)),
        }
    }
}

pub type ServiceResult<T> = std::result::Result<T, ServiceError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComposerOptionKind {
    Command,
    Skill,
    Capability,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerOptionsRequest {
    pub query: Option<String>,
    pub kinds: Vec<ComposerOptionKind>,
    pub limit: usize,
}

impl Default for ComposerOptionsRequest {
    fn default() -> Self {
        Self {
            query: None,
            kinds: Vec::new(),
            limit: 50,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerOption {
    pub kind: ComposerOptionKind,
    pub id: String,
    pub title: String,
    pub description: String,
    pub insert_text: String,
    pub badges: Vec<String>,
    pub keywords: Vec<String>,
}
