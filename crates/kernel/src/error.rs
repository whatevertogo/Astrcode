use astrcode_core::AstrError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum KernelError {
    #[error("{0}")]
    Validation(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Invoke(String),
}

impl From<AstrError> for KernelError {
    fn from(value: AstrError) -> Self {
        match value {
            AstrError::Validation(message) => Self::Validation(message),
            AstrError::SessionNotFound(id) => Self::NotFound(format!("session '{}' not found", id)),
            AstrError::ProjectNotFound(id) => Self::NotFound(format!("project '{}' not found", id)),
            other => Self::Invoke(other.to_string()),
        }
    }
}
