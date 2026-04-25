//! HTTP/SSE 传输层。
//!
//! 这里只放 Axum route、认证、DTO 映射和 HTTP 状态对象。

pub(crate) mod agent_api;
pub(crate) mod auth;
pub(crate) mod composer_catalog;
pub(crate) mod error;
pub(crate) mod mapper;
pub(crate) mod routes;
pub(crate) mod state;

pub(crate) use error::ApiError;
pub(crate) use state::{AppState, FrontendBuild};

/// 认证请求头名称。
///
/// 所有 API 请求通过此请求头携带认证 token。
pub(crate) const AUTH_HEADER_NAME: &str = "x-astrcode-token";
