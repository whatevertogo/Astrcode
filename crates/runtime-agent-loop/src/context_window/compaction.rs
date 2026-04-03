//! # 上下文压缩 (Context Compaction)
//!
//! 当会话消息接近 LLM 上下文窗口限制时，自动压缩历史消息以释放空间。
//!
//! ## 压缩策略
//!
//! 1. 将消息分为前缀（可压缩）和后缀（保留最近 Turn）
//! 2. 调用 LLM 对前缀生成摘要
//! 3. 用摘要替换前缀，保留后缀不变
//!
//! ## 重试机制
//!
//! 如果压缩请求本身超出上下文窗口，会逐步丢弃最旧的 Turn 并重试，
//! 最多重试 3 次。这是为了处理极端情况：即使前缀也可能超出窗口。
//!
//! ## 已知限制（TODO）
//!
//! - 当前仅支持文本消息，多模态消息需要额外处理
//! - 仅支持后缀保留（suffix-preserving），不支持 Claude 风格的 "from" 方向部分压缩
//! - 压缩后不恢复 prompt 附件（如文件内容），需要后续优化
//! - 当前总是重新摘要完整前缀，未来可升级为增量重压缩（incremental recompact）

use astrcode_core::{AstrError, CancelToken, LlmMessage, Result, UserMessageOrigin};
use chrono::{DateTime, Utc};

use crate::context_window::estimate_request_tokens;
use astrcode_runtime_llm::{LlmProvider, LlmRequest};

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

pub async fn auto_compact(
    provider: &dyn LlmProvider,
    messages: &[LlmMessage],
    base_system_prompt: Option<&str>,
    config: CompactConfig,
    cancel: CancelToken,
) -> Result<Option<CompactResult>> {
    // TODO(claude-auto-compact): Astrcode is still text-only here. Once multimodal messages land,
    // strip or downsample images/documents before sending the compact prompt, like Claude Code.
    // TODO(claude-auto-compact): add Claude-style partial compact ("from" direction) when prompt
    // cache support exists. v1 only supports suffix-preserving compaction.
    // TODO(claude-auto-compact): once sessions persist richer prompt attachments, restore them
    // after compaction here instead of scattering attachment recovery across prompt contributors.
    // TODO(claude-auto-compact): upgrade this entry point to incremental recompact by folding the
    // latest `CompactApplied` summary forward instead of always re-summarizing the full prefix.
    let preserved_recent_turns = config.keep_recent_turns.max(1);
    let (mut prefix, suffix, keep_start) = split_for_compaction(messages, preserved_recent_turns);
    if prefix.is_empty() {
        return Ok(None);
    }

    let pre_tokens = estimate_request_tokens(messages, base_system_prompt);
    let summary_prompt = build_compact_system_prompt(base_system_prompt);
    let mut attempts = 0usize;
    let summary = loop {
        let request_messages = compact_input_messages(&prefix);
        if request_messages.is_empty() {
            return Ok(None);
        }

        let request = LlmRequest::new(request_messages, Vec::new(), cancel.clone())
            .with_system(summary_prompt.clone());
        match provider.generate(request, None).await {
            Ok(output) => break extract_summary(&output.content)?,
            Err(error) if is_prompt_too_long(&error) && attempts < 3 => {
                attempts += 1;
                if !drop_oldest_turn_group(&mut prefix) {
                    return Err(error);
                }
            }
            Err(error) => return Err(error),
        }
    };

    let messages_removed = keep_start;
    let compacted_messages = compacted_messages(&summary, suffix);
    let post_tokens_estimate = estimate_request_tokens(&compacted_messages, base_system_prompt);
    Ok(Some(CompactResult {
        messages: compacted_messages,
        summary,
        preserved_recent_turns,
        pre_tokens,
        post_tokens_estimate,
        messages_removed,
        tokens_freed: pre_tokens.saturating_sub(post_tokens_estimate),
        timestamp: Utc::now(),
    }))
}

// TODO: 一旦提示配置能够独立拥有压缩策略而无需与通用系统提示耦合
// 替换此硬编码的摘要合约为更结构化的输入（如 JSON）以支持更丰富的摘要内容和更可靠的解析。
fn build_compact_system_prompt(base_system_prompt: Option<&str>) -> String {
    let mut prompt = String::from(
        "You are generating an internal compact summary for a coding-agent session. \
Summarize the provided conversation prefix so the agent can continue work without losing technical context.\n\n\
Rules:\n\
- Never call tools.\n\
- Keep the summary factual and implementation-oriented.\n\
- Ignore synthetic auto-continue nudges.\n\
- Do not address the end user.\n\
- Return exactly two XML blocks: <analysis>...</analysis> and <summary>...</summary>.\n\
- The <summary> block must contain these sections in order:\n\
  1. Primary Request and Intent\n\
  2. Key Technical Concepts\n\
  3. Files and Code Sections\n\
  4. Errors and fixes\n\
  5. Problem Solving\n\
  6. All user messages\n\
  7. Pending Tasks\n\
  8. Current Work\n\
  9. Optional Next Step\n",
    );

    if let Some(base_system_prompt) = base_system_prompt.filter(|value| !value.trim().is_empty()) {
        prompt.push_str("\nCurrent runtime system prompt for context:\n");
        prompt.push_str(base_system_prompt);
    }
    prompt
}

fn compact_input_messages(messages: &[LlmMessage]) -> Vec<LlmMessage> {
    messages
        .iter()
        .filter_map(|message| match message {
            // Keep earlier compact summaries in the next compaction input because they are the
            // only surviving representation of history that was already folded away.
            LlmMessage::User {
                origin: UserMessageOrigin::CompactSummary,
                ..
            } => Some(message.clone()),
            LlmMessage::User {
                origin: UserMessageOrigin::User,
                ..
            } => Some(message.clone()),
            LlmMessage::User { .. } => None,
            _ => Some(message.clone()),
        })
        .collect()
}

fn split_for_compaction(
    messages: &[LlmMessage],
    keep_recent_turns: usize,
) -> (Vec<LlmMessage>, Vec<LlmMessage>, usize) {
    let user_turn_indices = messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| match message {
            LlmMessage::User {
                origin: UserMessageOrigin::User,
                ..
            } => Some(index),
            _ => None,
        })
        .collect::<Vec<_>>();
    if user_turn_indices.is_empty() {
        return (messages.to_vec(), Vec::new(), messages.len());
    }

    let keep_turns = keep_recent_turns.min(user_turn_indices.len()).max(1);
    let keep_start = user_turn_indices[user_turn_indices.len() - keep_turns];
    (
        messages[..keep_start].to_vec(),
        messages[keep_start..].to_vec(),
        keep_start,
    )
}

fn drop_oldest_turn_group(prefix: &mut Vec<LlmMessage>) -> bool {
    let Some(first_turn_index) = prefix.iter().position(|message| {
        matches!(
            message,
            LlmMessage::User {
                origin: UserMessageOrigin::User,
                ..
            }
        )
    }) else {
        return false;
    };

    let next_turn_index = prefix
        .iter()
        .enumerate()
        .skip(first_turn_index + 1)
        .find_map(|(index, message)| {
            matches!(
                message,
                LlmMessage::User {
                    origin: UserMessageOrigin::User,
                    ..
                }
            )
            .then_some(index)
        })
        .unwrap_or(prefix.len());

    if next_turn_index == 0 || next_turn_index >= prefix.len() {
        prefix.clear();
        return false;
    }

    prefix.drain(..next_turn_index);
    !prefix.is_empty()
}

fn compacted_messages(summary: &str, suffix: Vec<LlmMessage>) -> Vec<LlmMessage> {
    let mut messages = vec![LlmMessage::User {
        content: format!(
            "[Auto-compact summary]\n{}\n\nContinue from this summary without repeating it to the user.",
            summary.trim()
        ),
        origin: UserMessageOrigin::CompactSummary,
    }];
    messages.extend(suffix);
    messages
}

fn extract_summary(content: &str) -> Result<String> {
    let summary = if let Some(start) = content.find("<summary>") {
        let start = start + "<summary>".len();
        let end = content[start..]
            .find("</summary>")
            .map(|offset| start + offset)
            .unwrap_or(content.len());
        content[start..end].trim().to_string()
    } else {
        content.trim().to_string()
    };

    if summary.is_empty() {
        return Err(AstrError::LlmStreamError(
            "compact summary response was empty".to_string(),
        ));
    }
    Ok(summary)
}

/// 判断错误是否为 prompt too long。
///
/// 使用结构化错误分类 (P4.3)，替代原先的纯字符串匹配。
/// 同时保持向后兼容：仍支持从 AstrError 中提取信息。
pub fn is_prompt_too_long(error: &astrcode_core::AstrError) -> bool {
    match error {
        astrcode_core::AstrError::LlmRequestFailed { status, body } => {
            let body_lower = body.to_ascii_lowercase();
            matches!(*status, 400 | 413)
                && (body_lower.contains("prompt too long")
                    || body_lower.contains("context length")
                    || body_lower.contains("maximum context")
                    || body_lower.contains("too many tokens"))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_summary_prefers_summary_block() {
        let summary = extract_summary("<analysis>draft</analysis><summary>\nSection\n</summary>")
            .expect("summary should parse");

        assert_eq!(summary, "Section");
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
                content: "[Auto-compact summary]\nolder".to_string(),
                origin: UserMessageOrigin::CompactSummary,
            },
            LlmMessage::User {
                content: "newer".to_string(),
                origin: UserMessageOrigin::User,
            },
        ];

        let (prefix, suffix, keep_start) = split_for_compaction(&messages, 1);

        assert_eq!(keep_start, 3);
        assert_eq!(prefix.len(), 3);
        assert_eq!(suffix.len(), 1);
    }

    #[test]
    fn compact_input_messages_keeps_previous_compact_summaries() {
        let messages = vec![
            LlmMessage::User {
                content: "[Auto-compact summary]\nOlder work".to_string(),
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
    fn compacted_messages_inserts_summary_as_compact_user_message() {
        let compacted = compacted_messages("Older history", Vec::new());

        assert!(matches!(
            &compacted[0],
            LlmMessage::User {
                origin: UserMessageOrigin::CompactSummary,
                ..
            }
        ));
    }
}
