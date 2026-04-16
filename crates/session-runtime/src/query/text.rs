//! query 层共享的文本规整与截断规则。
//!
//! Why: 各类只读快照都需要输出短摘要，统一到这里可以避免
//! 不同查询面各自复制 whitespace normalize / truncate 逻辑后逐渐漂移。

const DEFAULT_ELLIPSIS: &str = "...";

pub(crate) fn summarize_inline_text(text: &str, max_chars: usize) -> Option<String> {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_text(&normalized, max_chars)
}

pub(crate) fn truncate_text(text: &str, max_chars: usize) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    let char_count = trimmed.chars().count();
    if char_count <= max_chars {
        return Some(trimmed.to_string());
    }

    Some(trimmed.chars().take(max_chars).collect::<String>() + DEFAULT_ELLIPSIS)
}

#[cfg(test)]
mod tests {
    use super::{summarize_inline_text, truncate_text};

    #[test]
    fn summarize_inline_text_normalizes_whitespace_before_truncating() {
        assert_eq!(
            summarize_inline_text("  review   mailbox \n state  ", 120),
            Some("review mailbox state".to_string())
        );
    }

    #[test]
    fn truncate_text_trims_and_truncates_with_ascii_ellipsis() {
        assert_eq!(truncate_text("  hello  ", 10), Some("hello".to_string()));
        assert_eq!(truncate_text("   ", 10), None);
        assert_eq!(truncate_text(&"a".repeat(5), 3), Some("aaa...".to_string()));
    }
}
