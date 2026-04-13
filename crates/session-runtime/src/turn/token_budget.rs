//! # Token 预算管理
//!
//! 解析用户消息中的 Token 预算标记（如 `+50k`、`+1.5m`、`use 100k tokens`），
//! 并在 Turn 执行过程中检查预算使用情况。
//!
//! ## 预算标记格式
//!
//! - 简写格式: `+50k`、`+1.5m`（k = 1000, m = 1_000_000）
//! - 短语格式: `use 100k tokens`
//!
//! ## 预算决策
//!
//! - **Continue**: 预算充足，继续执行
//! - **Stop**: 预算为 0，立即停止
//! - **DiminishingReturns**: 接近预算上限且最近增量很小，停止以避免浪费

use std::sync::LazyLock;

use regex::Regex;

static SHORTHAND_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(\+\s*\d+(?:\.\d+)?\s*[km]?)").expect("regex"));
static PHRASE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(use\s+\d+(?:\.\d+)?\s*[km]?\s+tokens?)").expect("regex"));
static NUMERIC_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(\d+(?:\.\d+)?)\s*([km]?)").expect("regex"));

/// 预算检查决策。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenBudgetDecision {
    /// 预算充足，继续执行。
    Continue,
    /// 预算为 0，立即停止。
    Stop,
    /// 接近预算上限且最近增量很小，停止以避免浪费。
    DiminishingReturns,
}

/// 解析后的 Token 预算结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedTokenBudget {
    /// 清除预算标记后的用户消息。
    pub cleaned_text: String,
    /// 解析出的 Token 预算（如果存在）。
    pub budget: Option<u64>,
}

/// 从用户消息中解析 Token 预算标记。
pub fn parse_token_budget(user_message: &str) -> Option<u64> {
    extract_budget_match(user_message).map(|(budget, _)| budget)
}

/// 清除用户消息中的预算标记并返回清理后的文本和预算值。
pub fn strip_token_budget_marker(user_message: &str) -> ParsedTokenBudget {
    let Some((_, matched_span)) = extract_budget_match(user_message) else {
        return ParsedTokenBudget {
            cleaned_text: user_message.trim().to_string(),
            budget: None,
        };
    };
    let budget = parse_token_budget(user_message);

    let mut cleaned = String::new();
    cleaned.push_str(user_message[..matched_span.start].trim_end());
    if !cleaned.is_empty() && matched_span.end < user_message.len() {
        cleaned.push(' ');
    }
    cleaned.push_str(user_message[matched_span.end..].trim_start());

    ParsedTokenBudget {
        cleaned_text: cleaned.trim().to_string(),
        budget,
    }
}

/// 根据当前使用量检查 Token 预算。
///
/// 返回决策：继续、停止、收益递减。
pub fn check_token_budget(
    turn_tokens_used: u64,
    budget: u64,
    continuation_count: u8,
    last_delta_tokens: usize,
    continuation_min_delta_tokens: usize,
    max_continuations: u8,
) -> TokenBudgetDecision {
    if budget == 0 {
        return TokenBudgetDecision::Stop;
    }
    let ninety_percent = budget.saturating_mul(9) / 10;
    if turn_tokens_used < ninety_percent {
        return TokenBudgetDecision::Continue;
    }
    if continuation_count >= max_continuations || last_delta_tokens < continuation_min_delta_tokens
    {
        return TokenBudgetDecision::DiminishingReturns;
    }
    TokenBudgetDecision::Continue
}

/// 构建 auto-continue 提示，告诉 LLM 已使用了多少预算。
pub fn build_auto_continue_nudge(turn_tokens_used: u64, budget: u64) -> String {
    let pct = if budget == 0 {
        0
    } else {
        ((turn_tokens_used as f64 / budget as f64) * 100.0).round() as u64
    };
    format!(
        "Stopped at {pct}% of token target ({turn_tokens_used} / {budget}). Keep working -- do \
         not summarize."
    )
}

/// 从文本中提取预算匹配。
fn extract_budget_match(user_message: &str) -> Option<(u64, std::ops::Range<usize>)> {
    if let Some(matched) = SHORTHAND_RE
        .find_iter(user_message)
        .find(|matched| has_budget_marker_boundaries(user_message, matched.start(), matched.end()))
    {
        return parse_budget_value(matched.as_str()).map(|budget| (budget, matched.range()));
    }

    PHRASE_RE.find(user_message).and_then(|matched| {
        parse_budget_value(matched.as_str()).map(|budget| (budget, matched.range()))
    })
}

fn parse_budget_value(value: &str) -> Option<u64> {
    let captures = NUMERIC_RE.captures(value)?;
    let amount = captures.get(1)?.as_str().parse::<f64>().ok()?;
    let multiplier = match captures
        .get(2)
        .map(|value| value.as_str().to_ascii_lowercase())
    {
        Some(unit) if unit == "k" => 1_000f64,
        Some(unit) if unit == "m" => 1_000_000f64,
        _ => 1f64,
    };
    Some((amount * multiplier) as u64)
}

/// 检查预算标记是否在有效的边界位置（避免误匹配如 `C++20`）。
fn has_budget_marker_boundaries(message: &str, start: usize, end: usize) -> bool {
    boundary_char_before(message, start).is_none_or(is_budget_prefix_boundary)
        && boundary_char_after(message, end).is_none_or(is_budget_suffix_boundary)
}

fn boundary_char_before(message: &str, index: usize) -> Option<char> {
    message[..index].chars().next_back()
}

fn boundary_char_after(message: &str, index: usize) -> Option<char> {
    message[index..].chars().next()
}

fn is_budget_prefix_boundary(ch: char) -> bool {
    ch.is_whitespace() || matches!(ch, '(' | '[' | '{' | '"' | '\'')
}

fn is_budget_suffix_boundary(ch: char) -> bool {
    ch.is_whitespace()
        || matches!(
            ch,
            ')' | ']' | '}' | '.' | ',' | ';' | ':' | '!' | '?' | '"' | '\''
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_shorthand_and_phrase_budgets() {
        assert_eq!(parse_token_budget("+500k"), Some(500_000));
        assert_eq!(parse_token_budget("use 2M tokens"), Some(2_000_000));
    }

    #[test]
    fn stripping_budget_marker_preserves_user_text() {
        let parsed = strip_token_budget_marker("Fix the bug +500k");
        assert_eq!(parsed.cleaned_text, "Fix the bug");
        assert_eq!(parsed.budget, Some(500_000));
    }

    #[test]
    fn shorthand_budget_requires_standalone_boundaries() {
        // C++20 不应被误匹配为预算标记
        assert_eq!(parse_token_budget("Support C++20 modules"), None);
        assert_eq!(parse_token_budget("fix foo+1 parsing"), None);
        assert_eq!(
            strip_token_budget_marker("Support C++20 modules").cleaned_text,
            "Support C++20 modules"
        );
        assert_eq!(
            strip_token_budget_marker("fix foo+1 parsing").cleaned_text,
            "fix foo+1 parsing"
        );
    }

    #[test]
    fn diminishing_returns_stops_small_continuations() {
        assert_eq!(
            check_token_budget(950, 1_000, 3, 100, 500, 3),
            TokenBudgetDecision::DiminishingReturns
        );
    }

    #[test]
    fn zero_budget_stops_immediately() {
        assert_eq!(
            check_token_budget(0, 0, 0, 1_000, 100, 3),
            TokenBudgetDecision::Stop
        );
    }

    #[test]
    fn under_ninety_percent_continues() {
        assert_eq!(
            check_token_budget(899, 1_000, 99, 0, usize::MAX, 0),
            TokenBudgetDecision::Continue
        );
    }

    #[test]
    fn above_ninety_percent_continues_when_delta_still_large() {
        assert_eq!(
            check_token_budget(950, 1_000, 0, 600, 500, 3),
            TokenBudgetDecision::Continue
        );
    }

    // ── auto-continue 场景测试 ──────────────────────────────

    #[test]
    fn continue_when_budget_plenty_and_delta_large() {
        // 预算远未用完且最近输出量较大，应该继续
        assert_eq!(
            check_token_budget(500, 1_000, 1, 800, 500, 3),
            TokenBudgetDecision::Continue
        );
    }

    #[test]
    fn stop_at_max_continuations() {
        // 超过 90% 预算且达到最大续写次数，应停止
        assert_eq!(
            check_token_budget(950, 1_000, 3, 800, 500, 3),
            TokenBudgetDecision::DiminishingReturns
        );
    }

    #[test]
    fn stop_on_diminishing_returns() {
        // 超过 90% 预算但最近增量很小，收益递减
        assert_eq!(
            check_token_budget(920, 1_000, 1, 50, 500, 3),
            TokenBudgetDecision::DiminishingReturns
        );
    }

    #[test]
    fn nudge_shows_percentage_and_usage() {
        let nudge = build_auto_continue_nudge(500_000, 1_000_000);
        assert!(nudge.contains("50%"));
        assert!(nudge.contains("500000"));
        assert!(nudge.contains("1000000"));
    }

    #[test]
    fn nudge_handles_zero_budget_gracefully() {
        let nudge = build_auto_continue_nudge(500, 0);
        assert!(!nudge.is_empty());
    }
}
