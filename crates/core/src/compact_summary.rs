//! # Compact 摘要协议
//!
//! 统一管理 compact 摘要在消息流中的封装格式和解析规则，
//! 避免不同模块手写字符串拼接后逐步漂移。

/// compact 摘要消息的人类可读前缀。
pub const COMPACT_SUMMARY_PREFIX: &str = "[Auto-compact summary]\n";

/// compact 摘要正文里的旧历史回读提示。
pub const COMPACT_SUMMARY_HISTORY_NOTE_PREFIX: &str =
    "\n\nIf more detail from before compaction is needed, read the earlier session event log at:\n";

/// compact 摘要消息尾部的继续提示。
pub const COMPACT_SUMMARY_CONTINUATION: &str =
    "\n\nContinue from this summary without repeating it to the user.";

/// 被封装到消息流中的 compact 摘要。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactSummaryEnvelope {
    pub summary: String,
    pub history_path: Option<String>,
}

impl CompactSummaryEnvelope {
    pub fn new(summary: impl Into<String>) -> Self {
        Self {
            summary: summary.into().trim().to_string(),
            history_path: None,
        }
    }

    pub fn with_history_path(mut self, history_path: impl Into<String>) -> Self {
        let history_path = history_path.into().trim().to_string();
        if !history_path.is_empty() {
            self.history_path = Some(history_path);
        }
        self
    }

    /// 渲染 compact 摘要正文（不含外层前后缀）。
    pub fn render_body(&self) -> String {
        match self.history_path.as_deref() {
            Some(history_path) => {
                format!(
                    "{}{COMPACT_SUMMARY_HISTORY_NOTE_PREFIX}{history_path}",
                    self.summary
                )
            },
            None => self.summary.clone(),
        }
    }

    /// 统一生成 compact 摘要消息。
    ///
    /// 这里保留稳定的前后缀协议，确保 projector / compact parser / replay
    /// 都基于同一份格式工作，避免不同模块各自拼接字符串。
    pub fn render(&self) -> String {
        format!(
            "{COMPACT_SUMMARY_PREFIX}{}{COMPACT_SUMMARY_CONTINUATION}",
            self.render_body()
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
    let summary_body = summary_with_suffix
        .strip_suffix(COMPACT_SUMMARY_CONTINUATION)
        .unwrap_or(summary_with_suffix)
        .trim();
    if summary_body.is_empty() {
        return None;
    }

    let (summary, history_path) = if let Some((summary, history_path)) = summary_body
        .rsplit_once(COMPACT_SUMMARY_HISTORY_NOTE_PREFIX)
        .filter(|(_, history_path)| !history_path.trim().is_empty())
    {
        (
            summary.trim().to_string(),
            Some(history_path.trim().to_string()),
        )
    } else {
        (summary_body.to_string(), None)
    };

    let mut envelope = CompactSummaryEnvelope::new(summary);
    if let Some(history_path) = history_path {
        envelope = envelope.with_history_path(history_path);
    }
    Some(envelope)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_summary_round_trip_preserves_summary_text() {
        let rendered = format_compact_summary("summary body");
        let parsed = parse_compact_summary_message(&rendered).expect("summary should parse");

        assert_eq!(parsed.summary, "summary body");
        assert_eq!(parsed.history_path, None);
    }

    #[test]
    fn compact_summary_round_trip_preserves_history_path() {
        let rendered = CompactSummaryEnvelope::new("summary body")
            .with_history_path("~/.astrcode/projects/demo/sessions/abc/session-abc.jsonl")
            .render();
        let parsed = parse_compact_summary_message(&rendered).expect("summary should parse");

        assert_eq!(parsed.summary, "summary body");
        assert_eq!(
            parsed.history_path.as_deref(),
            Some("~/.astrcode/projects/demo/sessions/abc/session-abc.jsonl")
        );
    }

    #[test]
    fn compact_summary_parser_rejects_non_protocol_text() {
        assert!(parse_compact_summary_message("plain text").is_none());
    }
}
