//! # Compact 摘要协议
//!
//! 统一管理 compact 摘要在消息流中的封装格式和解析规则，
//! 避免不同模块手写字符串拼接后逐步漂移。

/// compact 摘要消息的人类可读前缀。
pub const COMPACT_SUMMARY_PREFIX: &str = "[Auto-compact summary]\n";

/// compact 摘要消息尾部的继续提示。
pub const COMPACT_SUMMARY_CONTINUATION: &str =
    "\n\nContinue from this summary without repeating it to the user.";

/// 被封装到消息流中的 compact 摘要。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactSummaryEnvelope {
    pub summary: String,
}

impl CompactSummaryEnvelope {
    pub fn new(summary: impl Into<String>) -> Self {
        Self {
            summary: summary.into().trim().to_string(),
        }
    }

    /// 统一生成 compact 摘要消息。
    ///
    /// 这里保留稳定的前后缀协议，确保 projector / compact parser / replay
    /// 都基于同一份格式工作，避免不同模块各自拼接字符串。
    pub fn render(&self) -> String {
        format!(
            "{COMPACT_SUMMARY_PREFIX}{}{COMPACT_SUMMARY_CONTINUATION}",
            self.summary
        )
    }
}

/// 生成 compact 摘要消息。
pub fn format_compact_summary(summary: &str) -> String {
    CompactSummaryEnvelope::new(summary).render()
}

/// 解析 compact 摘要消息。
///
/// 只接受 Astrcode 自己写入的固定封装格式；若消息不是 compact 摘要消息，
/// 返回 `None`，让调用方自行决定是否继续走其他分支。
pub fn parse_compact_summary_message(content: &str) -> Option<CompactSummaryEnvelope> {
    let summary_with_suffix = content.strip_prefix(COMPACT_SUMMARY_PREFIX)?;
    let summary = summary_with_suffix
        .strip_suffix(COMPACT_SUMMARY_CONTINUATION)
        .unwrap_or(summary_with_suffix)
        .trim();
    (!summary.is_empty()).then(|| CompactSummaryEnvelope::new(summary))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_summary_round_trip_preserves_summary_text() {
        let rendered = format_compact_summary("summary body");
        let parsed = parse_compact_summary_message(&rendered).expect("summary should parse");

        assert_eq!(parsed.summary, "summary body");
    }

    #[test]
    fn compact_summary_parser_rejects_non_protocol_text() {
        assert!(parse_compact_summary_message("plain text").is_none());
    }
}
