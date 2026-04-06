//! 会话 HTTP 路由按交互类型拆分：
//! - `query`：只读查询接口
//! - `mutation`：写操作与状态改变
//! - `stream`：SSE / 订阅类接口

mod mutation;
mod query;
mod stream;

use axum::http::StatusCode;
pub(crate) use mutation::{
    compact_session, create_session, delete_project, delete_session, interrupt_session,
    submit_prompt,
};
pub(crate) use query::{list_sessions, session_history, session_messages};
pub(crate) use stream::{session_catalog_events, session_events};

use crate::ApiError;

/// 对 HTTP 路由参数中的 session_id 做前置格式校验，避免非法字符进入后端路径解析。
///
/// 这里和 storage 层保持同一套字符白名单，并允许外层带 `session-` 前缀，
/// 统一剥离后再向 runtime 传递 canonical id。
pub(crate) fn validate_session_path_id(raw_session_id: &str) -> Result<String, ApiError> {
    let trimmed = raw_session_id.trim();
    let canonical = trimmed.strip_prefix("session-").unwrap_or(trimmed);
    let is_valid = !canonical.is_empty()
        && canonical
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == 'T');

    if is_valid {
        Ok(canonical.to_string())
    } else {
        Err(ApiError {
            status: StatusCode::BAD_REQUEST,
            message: format!("invalid session id: {raw_session_id}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::validate_session_path_id;

    #[test]
    fn validate_session_path_id_accepts_canonical_and_prefixed_values() {
        assert_eq!(
            validate_session_path_id("session-2026-04-06T10-00-00-abc_1")
                .expect("prefixed id should be accepted"),
            "2026-04-06T10-00-00-abc_1"
        );
        assert_eq!(
            validate_session_path_id("2026-04-06T10-00-00-abc_1")
                .expect("canonical id should be accepted"),
            "2026-04-06T10-00-00-abc_1"
        );
    }

    #[test]
    fn validate_session_path_id_rejects_unsafe_characters() {
        let err =
            validate_session_path_id("../../etc/passwd").expect_err("path traversal should fail");
        assert!(err.message.contains("invalid session id"));
    }
}
