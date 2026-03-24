use axum::http::HeaderMap;

use crate::{ApiError, AppState, AUTH_HEADER_NAME};

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
        .map(|token| secure_token_eq(token, &state.bootstrap_token))
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
