//! # 认证模块
//!
//! 本模块实现两层认证机制：
//!
//! 1. **Bootstrap Auth**：server 启动时生成的短期 token（24 小时）， 用于前端 Vite dev server 与
//!    server 进行初始鉴权交换。
//! 2. **API Session Auth**：通过 `/api/auth/exchange` 交换获得的长期 token（8 小时）， 用于所有后续
//!    API 请求的认证。
//!
//! Token 通过 `x-astrcode-token` 请求头或 `token` 查询参数传递，
//! 比较使用常量时间比较函数防止时序攻击。

use std::{collections::HashMap, sync::Mutex};

use axum::http::HeaderMap;
use chrono::{Duration, Utc};

use crate::{AUTH_HEADER_NAME, ApiError, AppState, bootstrap::random_hex_token};

/// API 会话 token 有效期（小时）。
///
/// 通过 `/api/auth/exchange` 交换获得的 token 有效期为 8 小时，
/// 过期后需要重新用 bootstrap token 交换。
const API_SESSION_TTL_HOURS: i64 = 8;

/// Bootstrap 认证凭证。
///
/// 包含 server 启动时生成的短期 token 和过期时间戳。
/// 仅用于 `/api/auth/exchange` 端点的初始验证，
/// 不用于常规 API 请求的认证。
#[derive(Debug, Clone)]
pub(crate) struct BootstrapAuth {
    token: String,
    expires_at_ms: i64,
}

impl BootstrapAuth {
    /// 创建新的 bootstrap 认证凭证。
    pub(crate) fn new(token: String, expires_at_ms: i64) -> Self {
        Self {
            token,
            expires_at_ms,
        }
    }

    /// 获取 bootstrap token 字符串引用。
    pub(crate) fn token(&self) -> &str {
        &self.token
    }

    /// 获取 token 过期时间戳（毫秒）。
    pub(crate) fn expires_at_ms(&self) -> i64 {
        self.expires_at_ms
    }

    /// 验证候选 token 是否匹配且未过期。
    ///
    /// 使用常量时间比较防止时序攻击，
    /// 同时检查当前时间是否超过过期时间戳。
    pub(crate) fn validate(&self, candidate: &str) -> bool {
        Utc::now().timestamp_millis() <= self.expires_at_ms
            && secure_token_eq(&self.token, candidate)
    }
}

/// 已签发的 API 认证 token。
///
/// 通过 `/api/auth/exchange` 端点返回给前端，
/// 用于后续所有 API 请求的认证。
#[derive(Debug, Clone)]
pub(crate) struct IssuedAuthToken {
    pub token: String,
    pub expires_at_ms: i64,
}

/// auth exchange 的共享摘要输入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AuthExchangeSummary {
    pub ok: bool,
    pub token: String,
    pub expires_at_ms: i64,
}

/// API 会话 token 管理器。
///
/// 维护一个线程安全的 token 映射，支持签发和验证。
/// 验证时会自动清理过期 token，防止内存泄漏。
#[derive(Debug, Default)]
pub(crate) struct AuthSessionManager {
    tokens: Mutex<HashMap<String, i64>>,
}

impl AuthSessionManager {
    /// 签发一个新的 API 会话 token。
    ///
    /// 使用随机生成的 token，有效期为 `API_SESSION_TTL_HOURS`（8 小时）。
    pub(crate) fn issue_token(&self) -> IssuedAuthToken {
        self.issue_named_token(random_hex_token(), API_SESSION_TTL_HOURS)
    }

    pub(crate) fn issue_exchange_summary(&self) -> AuthExchangeSummary {
        let issued = self.issue_token();
        AuthExchangeSummary {
            ok: true,
            token: issued.token,
            expires_at_ms: issued.expires_at_ms,
        }
    }

    /// 验证 token 是否有效且未过期。
    ///
    /// 验证前会先清理所有过期 token。
    /// 使用常量时间比较防止时序攻击。
    pub(crate) fn validate(&self, token: &str) -> bool {
        let now = Utc::now().timestamp_millis();
        let mut tokens = self.tokens.lock().expect("auth token lock poisoned");
        tokens.retain(|_, expires_at_ms| *expires_at_ms > now);
        tokens
            .iter()
            .any(|(known, expires_at_ms)| *expires_at_ms > now && secure_token_eq(known, token))
    }

    #[cfg(test)]
    pub(crate) fn issue_test_token(&self, token: impl Into<String>) -> IssuedAuthToken {
        self.issue_named_token(token.into(), API_SESSION_TTL_HOURS)
    }

    /// 签发指定名称和有效期的 token。
    ///
    /// 内部方法，被 `issue_token` 和 `issue_test_token` 共用。
    fn issue_named_token(&self, token: String, ttl_hours: i64) -> IssuedAuthToken {
        let expires_at_ms = (Utc::now() + Duration::hours(ttl_hours)).timestamp_millis();
        self.tokens
            .lock()
            .expect("auth token lock poisoned")
            .insert(token.clone(), expires_at_ms);
        IssuedAuthToken {
            token,
            expires_at_ms,
        }
    }
}

/// 检查请求是否携带有效的认证 token。
///
/// 优先从 `x-astrcode-token` 请求头读取，其次从 `token` 查询参数读取。
/// 未通过认证时返回 `ApiError::unauthorized()`（401）。
///
/// 此函数是路由处理器的认证守卫，所有需要认证的端点都应调用它。
pub(crate) fn require_auth(
    state: &AppState,
    headers: &HeaderMap,
    query_token: Option<&str>,
) -> Result<(), ApiError> {
    if is_authorized(state, headers, query_token) {
        Ok(())
    } else {
        Err(ApiError::unauthorized())
    }
}

pub(crate) fn is_authorized(
    state: &AppState,
    headers: &HeaderMap,
    query_token: Option<&str>,
) -> bool {
    headers
        .get(AUTH_HEADER_NAME)
        .and_then(|value| value.to_str().ok())
        .or(query_token)
        .map(|token| state.auth_sessions.validate(token))
        .unwrap_or(false)
}

/// 常量时间字符串比较，防止时序攻击。
///
/// 通过异或累积差异值，确保比较时间与输入内容无关。
/// 长度不同也会返回 false，但比较过程仍然遍历最大长度，
/// 避免通过响应时间推断 token 长度。
pub(crate) fn secure_token_eq(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    let mut diff = left.len() ^ right.len();

    for i in 0..left.len().max(right.len()) {
        let left_byte = left.get(i).copied().unwrap_or(0);
        let right_byte = right.get(i).copied().unwrap_or(0);
        diff |= usize::from(left_byte ^ right_byte);
    }

    diff == 0
}
