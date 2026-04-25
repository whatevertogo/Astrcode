//! 应用层错误类型。
//!
//! `ServerApplicationError` 是 application 层唯一的错误枚举，
//! 通过 `From` 转换桥接 core / session-runtime 的底层错误。

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ServerApplicationError {
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

impl From<astrcode_core::AstrError> for ServerApplicationError {
    fn from(e: astrcode_core::AstrError) -> Self {
        match e {
            astrcode_core::AstrError::SessionNotFound(message)
            | astrcode_core::AstrError::ProjectNotFound(message) => {
                ServerApplicationError::NotFound(message)
            },
            astrcode_core::AstrError::TurnInProgress(message) => {
                ServerApplicationError::Conflict(message)
            },
            astrcode_core::AstrError::Validation(message)
            | astrcode_core::AstrError::InvalidSessionId(message) => {
                ServerApplicationError::InvalidArgument(message)
            },
            other => ServerApplicationError::Internal(other.to_string()),
        }
    }
}
