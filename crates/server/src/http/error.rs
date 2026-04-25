use astrcode_core::AstrError;
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

use crate::application_error_bridge::ServerRouteError;

/// 错误响应载荷。
///
/// 所有 API 错误统一返回此结构，包含 `error` 字段描述错误信息。
#[derive(Debug, Serialize)]
struct ErrorPayload {
    error: String,
}

/// API 错误。
///
/// 将业务逻辑错误映射为 HTTP 状态码和错误消息。
#[derive(Debug)]
pub(crate) struct ApiError {
    pub(crate) status: StatusCode,
    pub(crate) message: String,
}

impl ApiError {
    pub(crate) fn unauthorized() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: "unauthorized".to_string(),
        }
    }

    pub(crate) fn bad_request(message: String) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message,
        }
    }

    pub(crate) fn internal_server_error(message: String) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorPayload {
                error: self.message,
            }),
        )
            .into_response()
    }
}

impl From<AstrError> for ApiError {
    fn from(value: AstrError) -> Self {
        let message = value.to_string();
        match value {
            AstrError::SessionNotFound(_) | AstrError::ProjectNotFound(_) => Self {
                status: StatusCode::NOT_FOUND,
                message,
            },
            AstrError::TurnInProgress(_) => Self {
                status: StatusCode::CONFLICT,
                message,
            },
            AstrError::InvalidSessionId(_)
            | AstrError::ConfigError { .. }
            | AstrError::MissingApiKey(_)
            | AstrError::MissingBaseUrl(_)
            | AstrError::NoProfilesConfigured
            | AstrError::ModelNotFound { .. }
            | AstrError::UnsupportedProvider(_)
            | AstrError::Validation(_) => Self {
                status: StatusCode::BAD_REQUEST,
                message,
            },
            AstrError::Cancelled => Self {
                status: StatusCode::CONFLICT,
                message,
            },
            _ => Self {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                message,
            },
        }
    }
}

impl From<ServerRouteError> for ApiError {
    fn from(value: ServerRouteError) -> Self {
        match value {
            ServerRouteError::NotFound(message) => Self {
                status: StatusCode::NOT_FOUND,
                message,
            },
            ServerRouteError::Conflict(message) => Self {
                status: StatusCode::CONFLICT,
                message,
            },
            ServerRouteError::InvalidArgument(message) => Self {
                status: StatusCode::BAD_REQUEST,
                message,
            },
            ServerRouteError::PermissionDenied(message) => Self {
                status: StatusCode::FORBIDDEN,
                message,
            },
            ServerRouteError::Internal(message) => Self {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                message,
            },
        }
    }
}
