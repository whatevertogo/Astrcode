//! 面向外层输入的 session 标识桥接。
//!
//! Why: runtime 仍然是 session key 规范化语义的唯一 owner，
//! 但上层 blanket impl 偶尔需要把原始字符串转换成 runtime key。

/// 规范化外部传入的 session 标识。
pub fn normalize_external_session_id(session_id: &str) -> String {
    crate::state::normalize_session_id(session_id)
}
