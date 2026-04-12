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
