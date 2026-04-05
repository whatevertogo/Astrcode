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
use astrcode_runtime_llm::{LlmProvider, LlmRequest};
use chrono::{DateTime, Utc};

use crate::context_window::estimate_request_tokens;

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
    compact_prompt_context: Option<&str>,
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
    // `compact_prompt_context` 是正常请求所见的运行时 system prompt 参考材料。
    // compact 流程不会直接把它当成最终 system prompt，而是嵌入专用 compact 模板中，
    // 让摘要模型知道当前会话原本运行在什么约束之下。
    let preserved_recent_turns = config.keep_recent_turns.max(1);
    let (mut prefix, suffix, keep_start) = split_for_compaction(messages, preserved_recent_turns);
    if prefix.is_empty() {
        return Ok(None);
    }

    let pre_tokens = estimate_request_tokens(messages, compact_prompt_context);
    let summary_prompt = render_compact_system_prompt(compact_prompt_context);
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
            },
            Err(error) => return Err(error),
        }
    };

    let messages_removed = keep_start;
    let auto_continue = matches!(config.trigger, astrcode_core::CompactTrigger::Auto);
    let compacted_messages = compacted_messages(&summary, suffix, auto_continue);
    let post_tokens_estimate = estimate_request_tokens(&compacted_messages, compact_prompt_context);
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

// TODO: 一旦提示配置能够独立拥有压缩策略而无需与通用系统提示耦合，
// 替换此硬编码的摘要合约为更结构化的输入（如 JSON）以支持更丰富的摘要内容和更可靠的解析。
// TODO: 未来考虑通过一个独立的压缩 agent 来生成摘要，允许更复杂的压缩逻辑和多轮压缩对话，
// 而不仅仅是单轮系统提示调用。
fn render_compact_system_prompt(compact_prompt_context: Option<&str>) -> String {
    // 整合 kimi-cli、pi-mono、opencode 等 CLI 工具的最佳实践：
    // - pi-mono: 结构化进度跟踪（Done/In Progress/Blocked）、迭代式更新、分支摘要
    // - kimi-cli: 压缩优先级规则（MUST KEEP/MERGE/REMOVE/CONDENSE）、代码长度阈值
    // - opencode: 发现跟踪（Discoveries）、目标与指令分离
    let mut prompt = String::from(
        "You are a context summarization assistant for a coding-agent session.\nYour summary will \
         replace earlier conversation history so another agent can continue seamlessly.\n\n## \
         CRITICAL RULES\n**DO NOT CALL ANY TOOLS.** This is for summary generation only.\n**Do \
         NOT continue the conversation.** Only output the structured summary.\n\n## Compression \
         Priorities (highest → lowest)\n1. **Current Task State** — What's being worked on, exact \
         status, immediate next steps\n2. **Errors & Solutions** — Stack traces, error messages, \
         and how they were resolved\n3. **User Requests** — All user messages verbatim in \
         order\n4. **Code Changes** — Final working versions; for code < 15 lines keep all, \
         otherwise signatures + key logic only\n5. **Key Decisions** — The \"why\" behind \
         choices, not just \"what\"\n6. **Discoveries** — Important learnings about the codebase, \
         APIs, or constraints\n7. **Environment** — Config/setup only if relevant to continuing \
         work\n\n## Compression Rules\n**MUST KEEP:** Error messages, stack traces, working \
         solutions, current task, exact file paths, function names\n**MERGE:** Similar \
         discussions into single summary points\n**REMOVE:** Redundant explanations, failed \
         attempts (keep only lessons learned), boilerplate code\n**CONDENSE:** Long code blocks → \
         signatures + key logic; long explanations → bullet points\n\n## Output Format\nReturn \
         exactly two XML blocks:\n\n<analysis>\n[Self-check before writing]\n- Did I cover ALL \
         user messages?\n- Is the current task state accurate?\n- Are all errors and their \
         solutions captured?\n- Are file paths and function names \
         exact?\n</analysis>\n\n<summary>\n\n## Goal\n[What the user is trying to accomplish — \
         can be multiple items]\n\n## Constraints & Preferences\n- [User-specified constraints, \
         preferences, requirements]\n- [Or \"(none)\" if not mentioned]\n\n## Progress\n### \
         Done\n- [x] [Completed tasks with brief outcome]\n\n### In Progress\n- [ ] [Current work \
         with status]\n\n### Blocked\n- [Issues preventing progress, or \"(none)\"]\n\n## Key \
         Decisions\n- **[Decision]**: [Rationale — why this choice was made]\n\n## Discoveries\n- \
         [Important learnings about codebase/APIs/constraints that future agent should \
         know]\n\n## Files\n### Read\n- `path/to/file` — [Why read, key findings]\n\n### \
         Modified/Created\n- `path/to/file` — [What changed, why]\n\n## Errors & Fixes\n- \
         **Error**: [Exact error message/stack trace]\n  - **Cause**: [Root cause]\n  - **Fix**: \
         [How it was resolved]\n\n## Next Steps\n1. [Ordered list of what should happen \
         next]\n\n## Critical Context\n[Any essential information not covered above, or \
         \"(none)\"]\n\n</summary>\n\n## Rules\n- Output **only** the <analysis> and <summary> \
         blocks — no preamble, no closing remarks.\n- Be concise. Prefer bullet points over \
         paragraphs.\n- Ignore synthetic auto-continue nudges.\n- Write in third-person, factual \
         tone. Do not address the end user.\n- Preserve exact file paths, function names, error \
         messages — never paraphrase these.\n- If a section has no content, write \"(none)\" \
         rather than omitting it.",
    );

    if let Some(compact_prompt_context) =
        compact_prompt_context.filter(|value| !value.trim().is_empty())
    {
        prompt.push_str("\nCurrent runtime system prompt for context:\n");
        prompt.push_str(compact_prompt_context);
    }
    prompt
}

/// 合并 compact 使用的 prompt 上下文。
///
/// compact 模板本身由 `render_compact_system_prompt()` 统一负责，这里只做两件事：
/// 1. 保留当前运行时已有的 system prompt 上下文
/// 2. 把 hook 提供的附加约束追加到末尾
///
/// 同时在 merge 边界折叠空字符串，避免把 `Some("")` 继续向下游传播。
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

fn compacted_messages(
    summary: &str,
    suffix: Vec<LlmMessage>,
    auto_continue: bool,
) -> Vec<LlmMessage> {
    let mut messages = vec![LlmMessage::User {
        content: format!(
            "[Auto-compact summary]\n{}\n\nContinue from this summary without repeating it to the \
             user.",
            summary.trim()
        ),
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
        },
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_compact_system_prompt_keeps_do_not_continue_instruction_intact() {
        let prompt = render_compact_system_prompt(None);

        assert!(
            prompt.contains("**Do NOT continue the conversation.**"),
            "compact prompt must explicitly instruct the summarizer not to continue the session"
        );
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
        let compacted = compacted_messages("Older history", Vec::new(), false);

        assert!(matches!(
            &compacted[0],
            LlmMessage::User {
                origin: UserMessageOrigin::CompactSummary,
                ..
            }
        ));
        assert_eq!(
            compacted.len(),
            1,
            "no auto-continue nudge when auto_continue=false"
        );
    }

    #[test]
    fn compacted_messages_appends_auto_continue_nudge_when_auto() {
        let compacted = compacted_messages("Older history", Vec::new(), true);

        assert_eq!(compacted.len(), 2);
        assert!(matches!(
            &compacted[0],
            LlmMessage::User {
                origin: UserMessageOrigin::CompactSummary,
                ..
            }
        ));
        assert!(matches!(
            &compacted[1],
            LlmMessage::User {
                content,
                origin: UserMessageOrigin::AutoContinueNudge,
            } if content.contains("compacted")
        ));
    }

    #[test]
    fn compacted_messages_omits_nudge_when_not_auto() {
        let compacted = compacted_messages("Older history", Vec::new(), false);

        assert_eq!(compacted.len(), 1);
        assert!(!compacted.iter().any(|m| matches!(
            m,
            LlmMessage::User {
                origin: UserMessageOrigin::AutoContinueNudge,
                ..
            }
        )));
    }
}
