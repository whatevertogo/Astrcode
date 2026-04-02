use std::fmt::{Display, Formatter};

use astrcode_core::{AstrError, StoreError};
pub use astrcode_core::{SessionEventRecord, SessionMessage};
use async_trait::async_trait;
use tokio::sync::broadcast;

#[derive(Debug, Clone)]
pub struct PromptAccepted {
    pub turn_id: String,
    pub session_id: String,
    pub branched_from_session_id: Option<String>,
}

pub struct SessionReplay {
    pub history: Vec<SessionEventRecord>,
    pub receiver: broadcast::Receiver<SessionEventRecord>,
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
            }
            Self::Internal(error) => Display::fmt(error, f),
        }
    }
}

impl std::error::Error for ServiceError {}

impl From<anyhow::Error> for ServiceError {
    fn from(value: anyhow::Error) -> Self {
        // 两级 downcast 链：spawn_blocking_service 将错误包装为 anyhow::Error
        // 传输（因为 tokio::task::spawn_blocking 返回 JoinError + 闭包返回值
        // 需要类型擦除）。此链尝试恢复原始错误变体以正确映射 HTTP 状态码：
        // 1. 先尝试还原为 ServiceError（跨越 spawn_blocking 边界的业务错误）
        // 2. 再尝试还原为 AstrError（底层领域错误）
        // 3. 都失败则包装为 Internal
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

/// 将领域错误映射为 HTTP 语义错误。
///
/// 每个 AstrError 变体被归类到对应的 HTTP 状态码类别：
/// - NotFound (404): SessionNotFound, ProjectNotFound
/// - Conflict (409): TurnInProgress
/// - InvalidInput (400): Validation, InvalidSessionId, MissingApiKey, MissingBaseUrl
/// - Internal (500): 其他所有错误（IO、LLM 失败等）
impl From<AstrError> for ServiceError {
    fn from(value: AstrError) -> Self {
        match &value {
            AstrError::SessionNotFound(id) => Self::NotFound(format!("session not found: {}", id)),
            AstrError::ProjectNotFound(id) => Self::NotFound(format!("project not found: {}", id)),
            AstrError::TurnInProgress(id) => {
                Self::Conflict(format!("turn already in progress: {}", id))
            }
            AstrError::Validation(msg) => Self::InvalidInput(msg.clone()),
            AstrError::InvalidSessionId(id) => {
                Self::InvalidInput(format!("invalid session id: {}", id))
            }
            AstrError::MissingApiKey(profile) => {
                Self::InvalidInput(format!("missing api key for profile: {}", profile))
            }
            AstrError::MissingBaseUrl(profile) => {
                Self::InvalidInput(format!("missing base url for profile: {}", profile))
            }
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
            }
            StoreError::Io { context, .. } => Self::Internal(AstrError::Internal(context)),
            StoreError::Parse { context, .. } => Self::Internal(AstrError::Internal(context)),
        }
    }
}

pub type ServiceResult<T> = std::result::Result<T, ServiceError>;
