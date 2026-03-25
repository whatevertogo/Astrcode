use std::collections::HashMap;
use std::sync::Mutex;

use axum::http::HeaderMap;
use chrono::{Duration, Utc};

use crate::bootstrap::random_hex_token;
use crate::{ApiError, AppState, AUTH_HEADER_NAME};

const API_SESSION_TTL_HOURS: i64 = 8;

#[derive(Debug, Clone)]
pub(crate) struct BootstrapAuth {
    token: String,
    expires_at_ms: i64,
}

impl BootstrapAuth {
    pub(crate) fn new(token: String, expires_at_ms: i64) -> Self {
        Self {
            token,
            expires_at_ms,
        }
    }

    pub(crate) fn token(&self) -> &str {
        &self.token
    }

    pub(crate) fn expires_at_ms(&self) -> i64 {
        self.expires_at_ms
    }

    pub(crate) fn validate(&self, candidate: &str) -> bool {
        Utc::now().timestamp_millis() <= self.expires_at_ms
            && secure_token_eq(&self.token, candidate)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct IssuedAuthToken {
    pub token: String,
    pub expires_at_ms: i64,
}

#[derive(Debug, Default)]
pub(crate) struct AuthSessionManager {
    tokens: Mutex<HashMap<String, i64>>,
}

impl AuthSessionManager {
    pub(crate) fn issue_token(&self) -> IssuedAuthToken {
        self.issue_named_token(random_hex_token(), API_SESSION_TTL_HOURS)
    }

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

pub(crate) fn require_auth(
    state: &AppState,
    headers: &HeaderMap,
    query_token: Option<&str>,
) -> Result<(), ApiError> {
    let header_token = headers
        .get(AUTH_HEADER_NAME)
        .and_then(|value| value.to_str().ok());
    let authorized = header_token
        .or(query_token)
        .map(|token| state.auth_sessions.validate(token))
        .unwrap_or(false);
    if authorized {
        Ok(())
    } else {
        Err(ApiError::unauthorized())
    }
}

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
