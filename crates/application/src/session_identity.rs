//! application 层的 session 输入整形辅助。
//!
//! Why: 用例层仍然只处理原始字符串，但 session key 规范化真相属于
//! runtime；这里保留一个极窄的桥接函数，避免业务代码各自复制规则。

/// 规范化外部传入的 session 标识。
///
/// 真正的规范化规则由 `session-runtime` 持有，这里只做转发。
pub(crate) fn normalize_external_session_id(session_id: &str) -> String {
    astrcode_session_runtime::identity::normalize_external_session_id(session_id)
}
