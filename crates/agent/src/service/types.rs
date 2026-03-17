use std::fmt::{Display, Formatter};

use astrcode_core::{AgentEvent, AstrError};
use async_trait::async_trait;
use tokio::sync::broadcast;

#[derive(Debug, Clone)]
pub struct PromptAccepted {
    pub turn_id: String,
}

#[derive(Clone, Debug)]
pub enum SessionMessage {
    User {
        content: String,
        timestamp: String,
    },
    Assistant {
        content: String,
        timestamp: String,
        reasoning_content: Option<String>,
    },
    ToolCall {
        tool_call_id: String,
        tool_name: String,
        args: serde_json::Value,
        output: Option<String>,
        ok: Option<bool>,
        duration_ms: Option<u64>,
    },
}

#[derive(Clone, Debug)]
pub struct SessionEventRecord {
    pub event_id: String,
    pub event: AgentEvent,
}

pub struct SessionReplay {
    pub history: Vec<SessionEventRecord>,
    pub receiver: broadcast::Receiver<SessionEventRecord>,
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

pub type ServiceResult<T> = std::result::Result<T, ServiceError>;
