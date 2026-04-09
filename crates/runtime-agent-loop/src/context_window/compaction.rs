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
//! 最多重试 3 次。这是为了处理极端情况：即使前缀也可能超出窗口。

use astrcode_core::{
    AstrError, CancelToken, LlmMessage, Result, UserMessageOrigin, format_compact_summary,
    parse_compact_summary_message,
};
use astrcode_runtime_llm::{LlmProvider, LlmRequest};
use chrono::{DateTime, Utc};

use crate::context_window::estimate_request_tokens;

const BASE_COMPACT_PROMPT_TEMPLATE: &str = include_str!("templates/compact/base.md");
const INCREMENTAL_COMPACT_PROMPT_TEMPLATE: &str = include_str!("templates/compact/incremental.md");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactConfig {
    pub keep_recent_turns: usize,
    pub trigger: astrcode_core::CompactTrigger,
}

#[derive(Debug, Clone)]
pub struct CompactResult {
    pub messages: Vec<LlmMessage>,
    pub summary: String,
    pub preserved_recent_turns: usize,
    pub pre_tokens: usize,
    pub post_tokens_estimate: usize,
    pub messages_removed: usize,
    pub tokens_freed: usize,
    pub timestamp: DateTime<Utc>,
}

/// compact 输入的边界类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompactionBoundary {
    RealUserTurn,
    AssistantStep,
}

/// 一段可以安全作为 compact 重试裁剪单位的前缀区间。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CompactionUnit {
    pub start: usize,
    pub boundary: CompactionBoundary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedCompactOutput {
    pub summary: String,
    pub has_analysis: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CompactPromptMode {
    Fresh,
    Incremental { previous_summary: String },
}

pub async fn auto_compact(
    provider: &dyn LlmProvider,
    messages: &[LlmMessage],
    compact_prompt_context: Option<&str>,
    config: CompactConfig,
    cancel: CancelToken,
) -> Result<Option<CompactResult>> {
    // TODO(claude-auto-compact): Astrcode is still text-only here. Once multimodal messages land,
    // strip or downsample images/documents before sending the compact prompt, like Claude Code.
    // TODO(claude-auto-compact): add Claude-style partial compact ("from" direction) when prompt
    // cache support exists. v1 only supports suffix-preserving compaction.
    // TODO(claude-auto-compact): add cache-sharing fork when prompt cache semantics become stable.
    // TODO(claude-auto-compact): once sessions persist richer prompt attachments, restore them
    // after compaction here instead of scattering attachment recovery across prompt contributors.
    // `compact_prompt_context` 是正常请求所见的运行时 system prompt 参考材料。
    // compact 流程不会直接把它当成最终 system prompt，而是嵌入专用 compact 模板中，
    // 让摘要模型知道当前会话原本运行在什么约束之下。
    let preserved_recent_turns = config.keep_recent_turns.max(1);
    let Some(mut split) = split_for_compaction(messages, preserved_recent_turns) else {
        return Ok(None);
    };

    let pre_tokens = estimate_request_tokens(messages, compact_prompt_context);
    let mut attempts = 0usize;
    let summary = loop {
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
        match provider.generate(request, None).await {
            Ok(output) => break parse_compact_output(&output.content)?.summary,
            Err(error) if is_prompt_too_long(&error) && attempts < 3 => {
                attempts += 1;
                if !drop_oldest_compaction_unit(&mut split.prefix) {
                    return Err(error);
                }
                split.keep_start = split.prefix.len();
            },
            Err(error) => return Err(error),
        }
    };

    let auto_continue = matches!(config.trigger, astrcode_core::CompactTrigger::Auto);
    let compacted_messages = compacted_messages(&summary, split.suffix, auto_continue);
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

#[derive(Debug, Clone)]
struct CompactionSplit {
    prefix: Vec<LlmMessage>,
    suffix: Vec<LlmMessage>,
    keep_start: usize,
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
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!("\nCurrent runtime system prompt for context:\n{value}"))
        .unwrap_or_default();

    BASE_COMPACT_PROMPT_TEMPLATE
        .replace("{{INCREMENTAL_MODE}}", incremental_block.trim())
        .replace("{{RUNTIME_CONTEXT}}", runtime_context.trim_end())
}

/// 合并 compact 使用的 prompt 上下文。
///
/// compact 模板本身由 `render_compact_system_prompt()` 统一负责，这里只做两件事：
/// 1. 保留当前运行时已有的 system prompt 上下文
/// 2. 把 hook 提供的附加约束追加到末尾
///
/// 同时在 merge 边界折叠空字符串，避免把 `Some(\"\")` 继续向下游传播。
pub(crate) fn merge_compact_prompt_context(
    runtime_system_prompt: Option<&str>,
    additional_system_prompt: Option<&str>,
) -> Option<String> {
    let runtime_system_prompt = runtime_system_prompt.filter(|value| !value.trim().is_empty());
    let additional_system_prompt =
        additional_system_prompt.filter(|value| !value.trim().is_empty());

    match (runtime_system_prompt, additional_system_prompt) {
        (None, None) => None,
        (Some(base), None) => Some(base.to_string()),
        (None, Some(additional)) => Some(additional.to_string()),
        (Some(base), Some(additional)) => Some(format!("{base}\n\n{additional}")),
    }
}

fn compact_input_messages(messages: &[LlmMessage]) -> Vec<LlmMessage> {
    // TODO(claude-auto-compact): 一旦 LlmMessage::User.content 从 String 扩展为
    // Vec<ContentPart>（支持多模态），在此处将 Image/Document 类型的 ContentPart 替换为
    // 文本占位符（"[image]" / "[document: filename]"），避免压缩请求因包含二进制数据而
    // 超出上下文窗口或被 Provider 拒绝。当前 content 是纯 String，暂无需过滤。
    let mut filtered = Vec::with_capacity(messages.len());
    for message in messages {
        match message {
            LlmMessage::User {
                origin: UserMessageOrigin::CompactSummary,
                ..
            }
            | LlmMessage::User {
                origin: UserMessageOrigin::ReactivationPrompt,
                ..
            }
            | LlmMessage::User {
                origin: UserMessageOrigin::User,
                ..
            } => filtered.push(message.clone()),
            LlmMessage::User { .. } => {},
            _ => filtered.push(message.clone()),
        }
    }
    filtered
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
///
/// 返回 `Some` 表示存在可压缩的前缀，返回 `None` 表示没有可压缩的内容。
/// 这个函数用于在调用 provider 之前进行早期检查，避免不必要的 API 调用。
pub(crate) fn can_compact(messages: &[LlmMessage], keep_recent_turns: usize) -> bool {
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
    // 当没有“旧 turn 前缀”可折叠时，允许在单 turn 长会话中退化到最近的 assistant step
    // 边界。这样手动 compact 和极长单用户请求仍有一条安全的 prefix compaction 路径，
    // 同时不在 tool result 中间切断。
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

fn compacted_messages(
    summary: &str,
    suffix: Vec<LlmMessage>,
    auto_continue: bool,
) -> Vec<LlmMessage> {
    let mut messages = vec![LlmMessage::User {
        content: format_compact_summary(summary),
        origin: UserMessageOrigin::CompactSummary,
    }];
    if auto_continue {
        messages.push(LlmMessage::User {
            content: "The conversation was compacted. Continue from where you left off."
                .to_string(),
            origin: UserMessageOrigin::AutoContinueNudge,
        });
    }
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

/// 判断错误是否为 prompt too long。
///
/// 使用结构化错误分类 (P4.3)，替代原先的纯字符串匹配。
/// 同时保持向后兼容：仍支持从 AstrError 中提取信息。
pub fn is_prompt_too_long(error: &astrcode_core::AstrError) -> bool {
    match error {
        astrcode_core::AstrError::LlmRequestFailed { status, body } => {
            matches!(*status, 400 | 413)
                && (contains_ascii_case_insensitive(body, "prompt too long")
                    || contains_ascii_case_insensitive(body, "context length")
                    || contains_ascii_case_insensitive(body, "maximum context")
                    || contains_ascii_case_insensitive(body, "too many tokens"))
        },
        _ => false,
    }
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
    fn merge_compact_prompt_context_keeps_existing_runtime_prompt_when_hook_is_empty() {
        let merged = merge_compact_prompt_context(Some("base"), None)
            .expect("runtime prompt context should be preserved");

        assert_eq!(merged, "base");
    }

    #[test]
    fn merge_compact_prompt_context_returns_none_when_both_empty() {
        assert!(merge_compact_prompt_context(None, None).is_none());
        assert!(merge_compact_prompt_context(Some("   "), Some(" \n\t ")).is_none());
    }

    #[test]
    fn merge_compact_prompt_context_keeps_additional_when_runtime_is_empty() {
        let merged = merge_compact_prompt_context(None, Some("hook"))
            .expect("hook prompt context should be preserved");

        assert_eq!(merged, "hook");
    }

    #[test]
    fn render_compact_system_prompt_skips_whitespace_only_context() {
        let prompt_none = render_compact_system_prompt(None, CompactPromptMode::Fresh);
        let prompt_ws = render_compact_system_prompt(Some("   \n\t  "), CompactPromptMode::Fresh);

        assert_eq!(prompt_ws, prompt_none);
        assert!(!prompt_ws.contains("Current runtime system prompt for context:"));
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
    fn compact_input_messages_keeps_previous_compact_summaries() {
        let messages = vec![
            LlmMessage::User {
                content: format_compact_summary("Older work"),
                origin: UserMessageOrigin::CompactSummary,
            },
            LlmMessage::User {
                content: "continue".to_string(),
                origin: UserMessageOrigin::AutoContinueNudge,
            },
        ];

        let filtered = compact_input_messages(&messages);

        assert_eq!(filtered.len(), 1);
        assert!(matches!(
            &filtered[0],
            LlmMessage::User {
                origin: UserMessageOrigin::CompactSummary,
                ..
            }
        ));
    }

    #[test]
    fn compact_input_messages_keeps_reactivation_prompts_but_not_as_real_turns() {
        let messages = vec![
            LlmMessage::User {
                content: "# Child Session Delivery".to_string(),
                origin: UserMessageOrigin::ReactivationPrompt,
            },
            LlmMessage::User {
                content: "real user".to_string(),
                origin: UserMessageOrigin::User,
            },
        ];

        let filtered = compact_input_messages(&messages);
        let split = split_for_compaction(&messages, 1).expect("split should exist");

        assert_eq!(filtered.len(), 2);
        assert_eq!(split.keep_start, 1);
    }

    #[test]
    fn compacted_messages_inserts_summary_as_compact_user_message() {
        let compacted = compacted_messages("Older history", Vec::new(), false);

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
    fn compacted_messages_appends_auto_continue_nudge_when_auto() {
        let compacted = compacted_messages("Older history", Vec::new(), true);

        assert_eq!(compacted.len(), 2);
        assert!(matches!(
            &compacted[1],
            LlmMessage::User {
                content,
                origin: UserMessageOrigin::AutoContinueNudge,
            } if content.contains("compacted")
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
}
