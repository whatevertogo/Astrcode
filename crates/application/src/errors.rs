use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApplicationError {
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<astrcode_core::AstrError> for ApplicationError {
    fn from(e: astrcode_core::AstrError) -> Self {
        match e {
            astrcode_core::AstrError::SessionNotFound(message)
            | astrcode_core::AstrError::ProjectNotFound(message) => {
                ApplicationError::NotFound(message)
            },
            astrcode_core::AstrError::TurnInProgress(message) => {
                ApplicationError::Conflict(message)
            },
            astrcode_core::AstrError::Validation(message)
            | astrcode_core::AstrError::InvalidSessionId(message) => {
                ApplicationError::InvalidArgument(message)
            },
            other => ApplicationError::Internal(other.to_string()),
        }
    }
}
