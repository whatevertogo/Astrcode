//! # Token 预算管理 (Token Budget)
//!
//! 解析用户消息中的 Token 预算标记（如 `+50k`、`+1.5m`、`use 100k tokens`），
//! 并在 Turn 执行过程中检查预算使用情况。
//!
//! ## 预算标记格式
//!
//! - 简写格式: `+50k`、`+1.5m`（k = 1000, m = 1000000）
//! - 短语格式: `use 100k tokens`
//!
//! ## 预算决策
//!
//! - **Continue**: 预算充足，继续执行
//! - **Stop**: 预算为 0，立即停止
//! - **DiminishingReturns**: 接近预算上限且最近增量很小，停止以避免浪费
//!
//! ## Auto-Continue Nudge
//!
//! 当 Turn 接近预算但未耗尽时，会生成一个自动继续提示（nudge），
//! 告诉 LLM 已使用了多少预算，鼓励其继续工作。

use regex::Regex;
use std::sync::LazyLock;

static SHORTHAND_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(\+\s*\d+(?:\.\d+)?\s*[km]?)").expect("regex"));
static PHRASE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(use\s+\d+(?:\.\d+)?\s*[km]?\s+tokens?)").expect("regex"));
static NUMERIC_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(\d+(?:\.\d+)?)\s*([km]?)").expect("regex"));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TokenBudgetDecision {
    Continue,
    Stop,
    DiminishingReturns,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedTokenBudget {
    pub cleaned_text: String,
    pub budget: Option<u64>,
}

pub(crate) fn parse_token_budget(user_message: &str) -> Option<u64> {
    extract_budget_match(user_message).map(|(budget, _)| budget)
}

pub(crate) fn strip_token_budget_marker(user_message: &str) -> ParsedTokenBudget {
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

pub(crate) fn check_token_budget(
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

pub(crate) fn build_auto_continue_nudge(turn_tokens_used: u64, budget: u64) -> String {
    let pct = if budget == 0 {
        0
    } else {
        ((turn_tokens_used as f64 / budget as f64) * 100.0).round() as u64
    };
    format!(
        "Stopped at {pct}% of token target ({turn_tokens_used} / {budget}). Keep working -- do not summarize."
    )
}

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
    fn under_ninety_percent_continues_without_diminishing_returns_checks() {
        assert_eq!(
            check_token_budget(899, 1_000, 99, 0, usize::MAX, 0),
            TokenBudgetDecision::Continue
        );
    }

    #[test]
    fn above_ninety_percent_continues_when_delta_is_still_large_enough() {
        assert_eq!(
            check_token_budget(950, 1_000, 0, 600, 500, 3),
            TokenBudgetDecision::Continue
        );
    }
}
