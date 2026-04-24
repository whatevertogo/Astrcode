//! server-owned application error bridge。
//!
//! 定义 server 自己的 route-facing 错误枚举。

use astrcode_core::AstrError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ServerRouteError {
    NotFound(String),
    Conflict(String),
    InvalidArgument(String),
    PermissionDenied(String),
    Internal(String),
}

impl ServerRouteError {
    pub(crate) fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }

    pub(crate) fn invalid_argument(message: impl Into<String>) -> Self {
        Self::InvalidArgument(message.into())
    }

    pub(crate) fn permission_denied(message: impl Into<String>) -> Self {
        Self::PermissionDenied(message.into())
    }

    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }
}

impl From<AstrError> for ServerRouteError {
    fn from(value: AstrError) -> Self {
        match value {
            AstrError::SessionNotFound(_)
            | AstrError::ProjectNotFound(_)
            | AstrError::ModelNotFound { .. } => Self::NotFound(value.to_string()),
            AstrError::TurnInProgress(_) | AstrError::Cancelled => {
                Self::Conflict(value.to_string())
            },
            AstrError::InvalidSessionId(_)
            | AstrError::ConfigError { .. }
            | AstrError::MissingApiKey(_)
            | AstrError::MissingBaseUrl(_)
            | AstrError::NoProfilesConfigured
            | AstrError::UnsupportedProvider(_)
            | AstrError::Validation(_) => Self::InvalidArgument(value.to_string()),
            _ => Self::Internal(value.to_string()),
        }
    }
}

impl std::fmt::Display for ServerRouteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServerRouteError::NotFound(message)
            | ServerRouteError::Conflict(message)
            | ServerRouteError::InvalidArgument(message)
            | ServerRouteError::PermissionDenied(message)
            | ServerRouteError::Internal(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ServerRouteError {}
