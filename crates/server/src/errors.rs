//! 应用层错误类型。
//!
//! `ApplicationError` 是 application 层唯一的错误枚举，
//! 通过 `From` 转换桥接 core / session-runtime 的底层错误。

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
