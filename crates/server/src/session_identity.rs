//! server-owned session 输入整形辅助。
//!
//! Why: `session-runtime` 的 session key 规范化规则非常窄，继续为了这一个
//! helper 保留正式依赖只会放大迁移尾巴；这里直接下沉同等规则，避免业务代码各自复制。

/// 规范化外部传入的 session 标识。
pub(crate) fn normalize_external_session_id(session_id: &str) -> String {
    let trimmed = session_id.trim();
    trimmed
        .strip_prefix("session-")
        .unwrap_or(trimmed)
        .to_string()
}
