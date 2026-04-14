//! # 上下文压缩 (Context Compaction)
//!
//! 当会话消息接近 LLM 上下文窗口限制时，自动压缩历史消息以释放空间。
//!
//! ## 压缩策略
//!
//! 1. 将消息分为前缀（可压缩）和后缀（保留最近安全边界）
//! 2. 调用 LLM 对前缀生成摘要
//! 3. 用摘要替换前缀，保留后缀不变
//!
//! ## 重试机制
//!
//! 如果压缩请求本身超出上下文窗口，会逐步丢弃最旧的 compact unit 并重试，
//! 最多重试 3 次。

use astrcode_core::{
    AstrError, CancelToken, LlmMessage, LlmRequest, ModelLimits, Result, UserMessageOrigin,
    format_compact_summary, parse_compact_summary_message,
};
use astrcode_kernel::KernelGateway;
use chrono::{DateTime, Utc};

use super::token_usage::{effective_context_window, estimate_request_tokens};

const BASE_COMPACT_PROMPT_TEMPLATE: &str = include_str!("templates/compact/base.md");
const INCREMENTAL_COMPACT_PROMPT_TEMPLATE: &str = include_str!("templates/compact/incremental.md");

/// 最大 reactive compact 重试次数。
const MAX_COMPACT_ATTEMPTS: usize = 3;

/// 压缩配置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactConfig {
    /// 保留最近的用户 turn 数量。
    pub keep_recent_turns: usize,
    /// 压缩触发方式。
    pub trigger: astrcode_core::CompactTrigger,
}

/// 压缩执行结果。
#[derive(Debug, Clone)]
pub struct CompactResult {
    /// 压缩后的完整消息列表。
    pub messages: Vec<LlmMessage>,
    /// 压缩生成的摘要文本。
    pub summary: String,
    /// 保留的最近 turn 数。
    pub preserved_recent_turns: usize,
    /// 压缩前估算 token 数。
    pub pre_tokens: usize,
    /// 压缩后估算 token 数。
    pub post_tokens_estimate: usize,
    /// 被移除的消息数。
    pub messages_removed: usize,
    /// 释放的 token 数。
    pub tokens_freed: usize,
    /// 压缩时间戳。
    pub timestamp: DateTime<Utc>,
}

/// compact 输入的边界类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompactionBoundary {
    RealUserTurn,
    AssistantStep,
}

/// 一段可以安全作为 compact 重试裁剪单位的前缀区间。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CompactionUnit {
    start: usize,
    boundary: CompactionBoundary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedCompactOutput {
    summary: String,
    has_analysis: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CompactPromptMode {
    Fresh,
    Incremental { previous_summary: String },
}

/// 执行自动压缩。
///
/// 通过 `gateway` 调用 LLM 对历史前缀生成摘要，替换为压缩后的消息。
/// 返回 `None` 表示没有可压缩的内容。
pub async fn auto_compact(
    gateway: &KernelGateway,
    messages: &[LlmMessage],
    compact_prompt_context: Option<&str>,
    config: CompactConfig,
    cancel: CancelToken,
) -> Result<Option<CompactResult>> {
    let preserved_recent_turns = config.keep_recent_turns.max(1);
    let Some(mut split) = split_for_compaction(messages, preserved_recent_turns) else {
        return Ok(None);
    };

    let pre_tokens = estimate_request_tokens(messages, compact_prompt_context);
    let mut attempts = 0usize;
    let summary = loop {
        if !trim_prefix_until_compact_request_fits(
            &mut split.prefix,
            compact_prompt_context,
            gateway.model_limits(),
        ) {
            return Err(AstrError::Internal(
                "compact request could not fit within summarization window".to_string(),
            ));
        }

        let request_messages = compact_input_messages(&split.prefix);
        if request_messages.is_empty() {
            return Ok(None);
        }

        let prompt_mode = latest_previous_summary(&split.prefix)
            .map(|previous_summary| CompactPromptMode::Incremental { previous_summary })
            .unwrap_or(CompactPromptMode::Fresh);
        let request = LlmRequest::new(request_messages, Vec::new(), cancel.clone()).with_system(
            render_compact_system_prompt(compact_prompt_context, prompt_mode),
        );
        match gateway.call_llm(request, None).await {
            Ok(output) => break parse_compact_output(&output.content)?.summary,
            Err(error) if is_prompt_too_long(&error) && attempts < MAX_COMPACT_ATTEMPTS => {
                attempts += 1;
                if !drop_oldest_compaction_unit(&mut split.prefix) {
                    return Err(AstrError::Internal(error.to_string()));
                }
                split.keep_start = split.prefix.len();
            },
            Err(error) => return Err(AstrError::Internal(error.to_string())),
        }
    };

    let compacted_messages = compacted_messages(&summary, split.suffix);
    let post_tokens_estimate = estimate_request_tokens(&compacted_messages, compact_prompt_context);
    Ok(Some(CompactResult {
        messages: compacted_messages,
        summary,
        preserved_recent_turns,
        pre_tokens,
        post_tokens_estimate,
        messages_removed: split.keep_start,
        tokens_freed: pre_tokens.saturating_sub(post_tokens_estimate),
        timestamp: Utc::now(),
    }))
}

/// 合并 compact 使用的 prompt 上下文。
pub fn merge_compact_prompt_context(
    runtime_system_prompt: Option<&str>,
    additional_system_prompt: Option<&str>,
) -> Option<String> {
    let runtime_system_prompt = runtime_system_prompt.filter(|v| !v.trim().is_empty());
    let additional_system_prompt = additional_system_prompt.filter(|v| !v.trim().is_empty());

    match (runtime_system_prompt, additional_system_prompt) {
        (None, None) => None,
        (Some(base), None) => Some(base.to_string()),
        (None, Some(additional)) => Some(additional.to_string()),
        (Some(base), Some(additional)) => Some(format!("{base}\n\n{additional}")),
    }
}

/// 判断错误是否为 prompt too long。
pub fn is_prompt_too_long(error: &astrcode_kernel::KernelError) -> bool {
    let message = error.to_string();
    // 检查常见 prompt-too-long 错误模式
    contains_ascii_case_insensitive(&message, "prompt too long")
        || contains_ascii_case_insensitive(&message, "context length")
        || contains_ascii_case_insensitive(&message, "maximum context")
        || contains_ascii_case_insensitive(&message, "too many tokens")
}

fn render_compact_system_prompt(
    compact_prompt_context: Option<&str>,
    mode: CompactPromptMode,
) -> String {
    let incremental_block = match mode {
        CompactPromptMode::Fresh => String::new(),
        CompactPromptMode::Incremental { previous_summary } => INCREMENTAL_COMPACT_PROMPT_TEMPLATE
            .replace("{{PREVIOUS_SUMMARY}}", previous_summary.trim()),
    };
    let runtime_context = compact_prompt_context
        .filter(|v| !v.trim().is_empty())
        .map(|v| format!("\nCurrent runtime system prompt for context:\n{v}"))
        .unwrap_or_default();

    BASE_COMPACT_PROMPT_TEMPLATE
        .replace("{{INCREMENTAL_MODE}}", incremental_block.trim())
        .replace("{{RUNTIME_CONTEXT}}", runtime_context.trim_end())
}

#[derive(Debug, Clone)]
struct CompactionSplit {
    prefix: Vec<LlmMessage>,
    suffix: Vec<LlmMessage>,
    keep_start: usize,
}

fn compact_input_messages(messages: &[LlmMessage]) -> Vec<LlmMessage> {
    messages
        .iter()
        .filter(|message| {
            !matches!(
                message,
                LlmMessage::User {
                    origin: UserMessageOrigin::CompactSummary
                        | UserMessageOrigin::ReactivationPrompt,
                    ..
                }
            )
        })
        .cloned()
        .collect()
}

fn latest_previous_summary(messages: &[LlmMessage]) -> Option<String> {
    messages.iter().rev().find_map(|message| match message {
        LlmMessage::User {
            content,
            origin: UserMessageOrigin::CompactSummary,
        } => parse_compact_summary_message(content).map(|envelope| envelope.summary),
        _ => None,
    })
}

/// 检查消息是否可以被压缩。
pub fn can_compact(messages: &[LlmMessage], keep_recent_turns: usize) -> bool {
    split_for_compaction(messages, keep_recent_turns).is_some()
}

fn split_for_compaction(
    messages: &[LlmMessage],
    keep_recent_turns: usize,
) -> Option<CompactionSplit> {
    if messages.is_empty() {
        return None;
    }

    let real_user_indices = real_user_turn_indices(messages);
    let primary_keep_start = real_user_indices
        .len()
        .checked_sub(keep_recent_turns.max(1))
        .map(|index| real_user_indices[index]);
    let keep_start = primary_keep_start
        .filter(|index| *index > 0)
        .or_else(|| fallback_keep_start(messages));

    let keep_start = keep_start?;
    Some(CompactionSplit {
        prefix: messages[..keep_start].to_vec(),
        suffix: messages[keep_start..].to_vec(),
        keep_start,
    })
}

fn real_user_turn_indices(messages: &[LlmMessage]) -> Vec<usize> {
    messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| match message {
            LlmMessage::User {
                origin: UserMessageOrigin::User,
                ..
            } => Some(index),
            _ => None,
        })
        .collect()
}

fn fallback_keep_start(messages: &[LlmMessage]) -> Option<usize> {
    compaction_units(messages)
        .into_iter()
        .rev()
        .find(|unit| unit.boundary == CompactionBoundary::AssistantStep && unit.start > 0)
        .map(|unit| unit.start)
}

fn compaction_units(messages: &[LlmMessage]) -> Vec<CompactionUnit> {
    messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| match message {
            LlmMessage::User {
                origin: UserMessageOrigin::User,
                ..
            } => Some(CompactionUnit {
                start: index,
                boundary: CompactionBoundary::RealUserTurn,
            }),
            LlmMessage::Assistant { .. } => Some(CompactionUnit {
                start: index,
                boundary: CompactionBoundary::AssistantStep,
            }),
            _ => None,
        })
        .collect()
}

fn drop_oldest_compaction_unit(prefix: &mut Vec<LlmMessage>) -> bool {
    let mut boundary_starts =
        prefix
            .iter()
            .enumerate()
            .filter_map(|(index, message)| match message {
                LlmMessage::User {
                    origin: UserMessageOrigin::User,
                    ..
                }
                | LlmMessage::Assistant { .. } => Some(index),
                _ => None,
            });
    let _current_start = boundary_starts.next();
    let Some(next_start) = boundary_starts.next() else {
        prefix.clear();
        return false;
    };
    if next_start == 0 || next_start >= prefix.len() {
        prefix.clear();
        return false;
    }

    prefix.drain(..next_start);
    !prefix.is_empty()
}

fn trim_prefix_until_compact_request_fits(
    prefix: &mut Vec<LlmMessage>,
    compact_prompt_context: Option<&str>,
    limits: ModelLimits,
) -> bool {
    loop {
        let request_messages = compact_input_messages(prefix);
        if request_messages.is_empty() {
            return false;
        }

        let prompt_mode = latest_previous_summary(prefix)
            .map(|previous_summary| CompactPromptMode::Incremental { previous_summary })
            .unwrap_or(CompactPromptMode::Fresh);
        let system_prompt = render_compact_system_prompt(compact_prompt_context, prompt_mode);
        if compact_request_fits_window(&request_messages, &system_prompt, limits) {
            return true;
        }

        if !drop_oldest_compaction_unit(prefix) {
            return false;
        }
    }
}

fn compact_request_fits_window(
    request_messages: &[LlmMessage],
    system_prompt: &str,
    limits: ModelLimits,
) -> bool {
    estimate_request_tokens(request_messages, Some(system_prompt))
        <= effective_context_window(limits)
}

fn compacted_messages(summary: &str, suffix: Vec<LlmMessage>) -> Vec<LlmMessage> {
    let mut messages = vec![LlmMessage::User {
        content: format_compact_summary(summary),
        origin: UserMessageOrigin::CompactSummary,
    }];
    messages.extend(suffix);
    messages
}

fn parse_compact_output(content: &str) -> Result<ParsedCompactOutput> {
    let has_analysis = content.contains("<analysis>") && content.contains("</analysis>");
    if !has_analysis {
        log::warn!("compact: missing <analysis> block in LLM response");
    }

    let Some(summary_start) = content.find("<summary>") else {
        return Err(AstrError::LlmStreamError(
            "compact response missing <summary> block".to_string(),
        ));
    };
    let summary_start = summary_start + "<summary>".len();
    let Some(summary_end_offset) = content[summary_start..].find("</summary>") else {
        return Err(AstrError::LlmStreamError(
            "compact response missing closing </summary> tag".to_string(),
        ));
    };
    let summary_end = summary_start + summary_end_offset;
    let summary = content[summary_start..summary_end].trim().to_string();
    if summary.is_empty() {
        return Err(AstrError::LlmStreamError(
            "compact summary response was empty".to_string(),
        ));
    }

    Ok(ParsedCompactOutput {
        summary,
        has_analysis,
    })
}

fn contains_ascii_case_insensitive(haystack: &str, needle: &str) -> bool {
    let needle = needle.as_bytes();
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_compact_system_prompt_keeps_do_not_continue_instruction_intact() {
        let prompt = render_compact_system_prompt(None, CompactPromptMode::Fresh);

        assert!(
            prompt.contains("**Do NOT continue the conversation.**"),
            "compact prompt must explicitly instruct the summarizer not to continue the session"
        );
    }

    #[test]
    fn render_compact_system_prompt_renders_incremental_block() {
        let prompt = render_compact_system_prompt(
            None,
            CompactPromptMode::Incremental {
                previous_summary: "older summary".to_string(),
            },
        );

        assert!(prompt.contains("## Incremental Mode"));
        assert!(prompt.contains("<previous-summary>"));
        assert!(prompt.contains("older summary"));
    }

    #[test]
    fn merge_compact_prompt_context_appends_hook_suffix_after_runtime_prompt() {
        let merged = merge_compact_prompt_context(Some("base"), Some("hook"))
            .expect("merged compact prompt context should exist");

        assert_eq!(merged, "base\n\nhook");
    }

    #[test]
    fn merge_compact_prompt_context_returns_none_when_both_empty() {
        assert!(merge_compact_prompt_context(None, None).is_none());
        assert!(merge_compact_prompt_context(Some("   "), Some(" \n\t ")).is_none());
    }

    #[test]
    fn parse_compact_output_requires_summary_block() {
        let error = parse_compact_output("plain text").expect_err("missing summary should fail");
        assert!(error.to_string().contains("missing <summary> block"));
    }

    #[test]
    fn parse_compact_output_requires_closed_summary_block() {
        let error =
            parse_compact_output("<summary>open").expect_err("unclosed summary should fail");
        assert!(error.to_string().contains("closing </summary>"));
    }

    #[test]
    fn parse_compact_output_prefers_summary_block() {
        let parsed =
            parse_compact_output("<analysis>draft</analysis><summary>\nSection\n</summary>")
                .expect("summary should parse");

        assert_eq!(parsed.summary, "Section");
        assert!(parsed.has_analysis);
    }

    #[test]
    fn split_for_compaction_preserves_recent_real_user_turns() {
        let messages = vec![
            LlmMessage::User {
                content: "older".to_string(),
                origin: UserMessageOrigin::User,
            },
            LlmMessage::Assistant {
                content: "ack".to_string(),
                tool_calls: Vec::new(),
                reasoning: None,
            },
            LlmMessage::User {
                content: format_compact_summary("older"),
                origin: UserMessageOrigin::CompactSummary,
            },
            LlmMessage::User {
                content: "newer".to_string(),
                origin: UserMessageOrigin::User,
            },
        ];

        let split = split_for_compaction(&messages, 1).expect("split should exist");

        assert_eq!(split.keep_start, 3);
        assert_eq!(split.prefix.len(), 3);
        assert_eq!(split.suffix.len(), 1);
    }

    #[test]
    fn split_for_compaction_falls_back_to_assistant_boundary_for_single_turn() {
        let messages = vec![
            LlmMessage::User {
                content: "task".to_string(),
                origin: UserMessageOrigin::User,
            },
            LlmMessage::Assistant {
                content: "step 1".to_string(),
                tool_calls: Vec::new(),
                reasoning: None,
            },
            LlmMessage::Assistant {
                content: "step 2".to_string(),
                tool_calls: Vec::new(),
                reasoning: None,
            },
        ];

        let split = split_for_compaction(&messages, 1).expect("single turn should still split");
        assert_eq!(split.keep_start, 2);
    }

    #[test]
    fn compacted_messages_inserts_summary_as_compact_user_message() {
        let compacted = compacted_messages("Older history", Vec::new());

        assert!(matches!(
            &compacted[0],
            LlmMessage::User {
                origin: UserMessageOrigin::CompactSummary,
                ..
            }
        ));
        assert_eq!(compacted.len(), 1);
    }

    #[test]
    fn compact_input_messages_skips_synthetic_user_messages() {
        let filtered = compact_input_messages(&[
            LlmMessage::User {
                content: "summary".to_string(),
                origin: UserMessageOrigin::CompactSummary,
            },
            LlmMessage::User {
                content: "wake up".to_string(),
                origin: UserMessageOrigin::ReactivationPrompt,
            },
            LlmMessage::User {
                content: "real user".to_string(),
                origin: UserMessageOrigin::User,
            },
        ]);

        assert_eq!(filtered.len(), 1);
        assert!(matches!(
            &filtered[0],
            LlmMessage::User {
                content,
                origin: UserMessageOrigin::User
            } if content == "real user"
        ));
    }

    #[test]
    fn drop_oldest_compaction_unit_is_deterministic() {
        let mut prefix = vec![
            LlmMessage::User {
                content: "task".to_string(),
                origin: UserMessageOrigin::User,
            },
            LlmMessage::Assistant {
                content: "step-1".to_string(),
                tool_calls: Vec::new(),
                reasoning: None,
            },
            LlmMessage::Assistant {
                content: "step-2".to_string(),
                tool_calls: Vec::new(),
                reasoning: None,
            },
        ];

        assert!(drop_oldest_compaction_unit(&mut prefix));
        assert!(matches!(
            &prefix[0],
            LlmMessage::Assistant { content, .. } if content == "step-1"
        ));
    }

    #[test]
    fn trim_prefix_until_compact_request_fits_drops_oldest_units_before_calling_llm() {
        let mut prefix = vec![
            LlmMessage::User {
                content: "very old request ".repeat(1200),
                origin: UserMessageOrigin::User,
            },
            LlmMessage::Assistant {
                content: "first step".repeat(1200),
                tool_calls: Vec::new(),
                reasoning: None,
            },
            LlmMessage::Assistant {
                content: "latest step".to_string(),
                tool_calls: Vec::new(),
                reasoning: None,
            },
        ];

        let trimmed = trim_prefix_until_compact_request_fits(
            &mut prefix,
            None,
            ModelLimits {
                context_window: 23_000,
                max_output_tokens: 2_000,
            },
        );

        assert!(trimmed);
        assert!(matches!(
            prefix.as_slice(),
            [LlmMessage::Assistant { content, .. }] if content == "latest step"
        ));
    }

    #[test]
    fn can_compact_returns_false_for_empty_messages() {
        assert!(!can_compact(&[], 2));
    }

    #[test]
    fn can_compact_returns_true_when_enough_turns() {
        let messages = vec![
            LlmMessage::User {
                content: "turn-1".to_string(),
                origin: UserMessageOrigin::User,
            },
            LlmMessage::Assistant {
                content: "reply".to_string(),
                tool_calls: Vec::new(),
                reasoning: None,
            },
            LlmMessage::User {
                content: "turn-2".to_string(),
                origin: UserMessageOrigin::User,
            },
        ];
        assert!(can_compact(&messages, 1));
    }
}
