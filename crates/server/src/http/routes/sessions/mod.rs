//! 会话 HTTP 路由按交互类型拆分：
//! - `query`：只读查询接口
//! - `mutation`：写操作与状态改变
//! - `stream`：SSE / 订阅类接口

mod filter;
mod mutation;
mod query;
mod stream;

use axum::http::StatusCode;
pub(crate) use mutation::{
    compact_session, create_session, delete_project, delete_session, interrupt_session,
    submit_prompt,
};
pub(crate) use query::{list_sessions, session_history};
pub(crate) use stream::{session_catalog_events, session_events};

use crate::ApiError;

/// 通用的路径 ID 验证函数，支持可选前缀和字符白名单。
///
/// # 参数
/// - `raw_id`: 原始 ID 字符串
/// - `prefix`: 可选前缀（如 "session-"），如果存在会被剥离
/// - `allow_timestamp`: 是否允许时间戳字符 'T'（用于 session ID）
/// - `field_name`: 字段名称，用于错误消息
pub(crate) fn validate_path_id(
    raw_id: &str,
    prefix: Option<&str>,
    allow_timestamp: bool,
    field_name: &str,
) -> Result<String, ApiError> {
    let trimmed = raw_id.trim();
    let canonical = if let Some(p) = prefix {
        trimmed.strip_prefix(p).unwrap_or(trimmed)
    } else {
        trimmed
    };

    let is_valid = !canonical.is_empty()
        && canonical.chars().all(|c| {
            c.is_ascii_alphanumeric() || c == '-' || c == '_' || (allow_timestamp && c == 'T')
        });

    if is_valid {
        Ok(canonical.to_string())
    } else {
        Err(ApiError {
            status: StatusCode::BAD_REQUEST,
            message: format!("invalid {field_name} id: {raw_id}"),
        })
    }
}

/// 对 HTTP 路由参数中的 session_id 做前置格式校验，避免非法字符进入后端路径解析。
///
/// 这里和 storage 层保持同一套字符白名单，并允许外层带 `session-` 前缀，
/// 统一剥离后再向 runtime 传递 canonical id。
pub(crate) fn validate_session_path_id(raw_session_id: &str) -> Result<String, ApiError> {
    validate_path_id(raw_session_id, Some("session-"), true, "session")
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
