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

use std::sync::OnceLock;

use astrcode_core::{
    AstrError, CancelToken, CompactAppliedMeta, CompactMode, CompactSummaryEnvelope, LlmMessage,
    LlmRequest, ModelLimits, Result, UserMessageOrigin, format_compact_summary,
    parse_compact_summary_message,
    tool_result_persist::{is_persisted_output, persisted_output_absolute_path},
};
use astrcode_kernel::KernelGateway;
use chrono::{DateTime, Utc};
use regex::Regex;

use super::token_usage::{effective_context_window, estimate_request_tokens};

const BASE_COMPACT_PROMPT_TEMPLATE: &str = include_str!("templates/compact/base.md");
const INCREMENTAL_COMPACT_PROMPT_TEMPLATE: &str = include_str!("templates/compact/incremental.md");

/// 压缩配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompactConfig {
    /// 保留最近的用户 turn 数量。
    pub keep_recent_turns: usize,
    /// 压缩触发方式。
    pub trigger: astrcode_core::CompactTrigger,
    /// compact 请求自身保留的输出预算。
    pub summary_reserve_tokens: usize,
    /// compact 允许的最大裁剪重试次数。
    pub max_retry_attempts: usize,
    /// compact 后注入给模型的旧历史 event log 路径提示。
    pub history_path: Option<String>,
    /// 仅对手动 compact 生效的附加指令。
    pub custom_instructions: Option<String>,
}

/// 压缩执行结果。
#[derive(Debug, Clone)]
pub(crate) struct CompactResult {
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
    /// compact 执行元数据。
    pub meta: CompactAppliedMeta,
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
    used_fallback: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CompactPromptMode {
    Fresh,
    Incremental { previous_summary: String },
}

impl CompactPromptMode {
    fn compact_mode(&self, retry_count: usize) -> CompactMode {
        if retry_count > 0 {
            CompactMode::RetrySalvage
        } else if matches!(self, Self::Incremental { .. }) {
            CompactMode::Incremental
        } else {
            CompactMode::Full
        }
    }
}

#[derive(Debug, Clone)]
struct PreparedCompactInput {
    messages: Vec<LlmMessage>,
    prompt_mode: CompactPromptMode,
    input_units: usize,
}

/// 执行自动压缩。
///
/// 通过 `gateway` 调用 LLM 对历史前缀生成摘要，替换为压缩后的消息。
/// 返回 `None` 表示没有可压缩的内容。
///
/// 当前系统只有这一套真实 compact 流程。若未来需要按 mode 调整行为，应扩展
/// `CompactConfig` / `ContextWindowSettings` 这类显式参数，而不是恢复未消费的粗粒度策略枚举。
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
    let (parsed_output, prepared_input) = loop {
        if !trim_prefix_until_compact_request_fits(
            &mut split.prefix,
            compact_prompt_context,
            gateway.model_limits(),
            &config,
        ) {
            return Err(AstrError::Internal(
                "compact request could not fit within summarization window".to_string(),
            ));
        }

        let prepared_input = prepare_compact_input(&split.prefix);
        if prepared_input.messages.is_empty() {
            return Ok(None);
        }

        let request = LlmRequest::new(prepared_input.messages.clone(), Vec::new(), cancel.clone())
            .with_system(render_compact_system_prompt(
                compact_prompt_context,
                prepared_input.prompt_mode.clone(),
                config.custom_instructions.as_deref(),
            ));
        match gateway.call_llm(request, None).await {
            Ok(output) => break (parse_compact_output(&output.content)?, prepared_input),
            Err(error) if is_prompt_too_long(&error) && attempts < config.max_retry_attempts => {
                attempts += 1;
                if !drop_oldest_compaction_unit(&mut split.prefix) {
                    return Err(AstrError::Internal(error.to_string()));
                }
                split.keep_start = split.prefix.len();
            },
            Err(error) => return Err(AstrError::Internal(error.to_string())),
        }
    };

    let summary = {
        let summary = sanitize_compact_summary(&parsed_output.summary);
        if let Some(history_path) = config.history_path.as_deref() {
            CompactSummaryEnvelope::new(summary)
                .with_history_path(history_path)
                .render_body()
        } else {
            summary
        }
    };
    let output_summary_chars = summary.chars().count().min(u32::MAX as usize) as u32;
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
        meta: CompactAppliedMeta {
            mode: prepared_input.prompt_mode.compact_mode(attempts),
            instructions_present: config
                .custom_instructions
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty()),
            fallback_used: parsed_output.used_fallback || attempts > 0,
            retry_count: attempts.min(u32::MAX as usize) as u32,
            input_units: prepared_input.input_units.min(u32::MAX as usize) as u32,
            output_summary_chars,
        },
    }))
}

/// 合并 compact 使用的 prompt 上下文。
#[cfg(test)]
fn merge_compact_prompt_context(
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
    custom_instructions: Option<&str>,
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
    let custom_instruction_block = custom_instructions
        .filter(|value| !value.trim().is_empty())
        .map(|value| {
            format!(
                "\n## Manual Compact Instructions\nFollow these extra requirements for this \
                 compact only:\n{value}"
            )
        })
        .unwrap_or_default();

    BASE_COMPACT_PROMPT_TEMPLATE
        .replace("{{INCREMENTAL_MODE}}", incremental_block.trim())
        .replace("{{CUSTOM_INSTRUCTIONS}}", custom_instruction_block.trim())
        .replace("{{RUNTIME_CONTEXT}}", runtime_context.trim_end())
}

#[derive(Debug, Clone)]
struct CompactionSplit {
    prefix: Vec<LlmMessage>,
    suffix: Vec<LlmMessage>,
    keep_start: usize,
}

fn prepare_compact_input(messages: &[LlmMessage]) -> PreparedCompactInput {
    let prompt_mode = latest_previous_summary(messages)
        .map(|previous_summary| CompactPromptMode::Incremental { previous_summary })
        .unwrap_or(CompactPromptMode::Fresh);
    let messages = messages
        .iter()
        .filter_map(normalize_compaction_message)
        .collect::<Vec<_>>();
    let input_units = compaction_units(&messages).len().max(1);
    PreparedCompactInput {
        messages,
        prompt_mode,
        input_units,
    }
}

fn latest_previous_summary(messages: &[LlmMessage]) -> Option<String> {
    messages.iter().rev().find_map(|message| match message {
        LlmMessage::User {
            content,
            origin: UserMessageOrigin::CompactSummary,
        } => parse_compact_summary_message(content)
            .map(|envelope| sanitize_compact_summary(&envelope.summary)),
        _ => None,
    })
}

fn normalize_compaction_message(message: &LlmMessage) -> Option<LlmMessage> {
    match message {
        LlmMessage::User {
            content,
            origin: UserMessageOrigin::User,
        } => Some(LlmMessage::User {
            content: content.trim().to_string(),
            origin: UserMessageOrigin::User,
        }),
        LlmMessage::User { .. } => None,
        LlmMessage::Assistant {
            content,
            tool_calls,
            ..
        } => {
            let mut lines = Vec::new();
            let visible = collapse_compaction_whitespace(content);
            if !visible.is_empty() {
                lines.push(visible);
            }
            if !tool_calls.is_empty() {
                let names = tool_calls
                    .iter()
                    .map(|call| call.name.trim())
                    .filter(|name| !name.is_empty())
                    .collect::<Vec<_>>();
                if !names.is_empty() {
                    lines.push(format!("Requested tools: {}", names.join(", ")));
                }
            }
            let normalized = lines.join("\n");
            if normalized.trim().is_empty() {
                None
            } else {
                Some(LlmMessage::Assistant {
                    content: normalized,
                    tool_calls: Vec::new(),
                    reasoning: None,
                })
            }
        },
        LlmMessage::Tool {
            tool_call_id,
            content,
        } => {
            let normalized = normalize_compaction_tool_content(content);
            if normalized.is_empty() {
                None
            } else {
                Some(LlmMessage::Tool {
                    tool_call_id: tool_call_id.clone(),
                    content: normalized,
                })
            }
        },
    }
}

fn collapse_compaction_whitespace(content: &str) -> String {
    content
        .lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n")
        .split("\n\n\n")
        .collect::<Vec<_>>()
        .join("\n\n")
        .trim()
        .to_string()
}

fn normalize_compaction_tool_content(content: &str) -> String {
    let stripped_child_ref = strip_child_agent_reference_hint(content);
    let collapsed = collapse_compaction_whitespace(&stripped_child_ref);
    if collapsed.is_empty() {
        return String::new();
    }
    if is_persisted_output(&collapsed) {
        return summarize_persisted_tool_output(&collapsed);
    }

    const MAX_COMPACTION_TOOL_CHARS: usize = 1_600;
    let char_count = collapsed.chars().count();
    if char_count <= MAX_COMPACTION_TOOL_CHARS {
        return collapsed;
    }

    let preview = collapsed
        .chars()
        .take(MAX_COMPACTION_TOOL_CHARS)
        .collect::<String>();
    format!(
        "{preview}\n\n[tool output truncated for compaction; preserve only the conclusion, key \
         errors, important file paths, and referenced IDs]"
    )
}

fn summarize_persisted_tool_output(content: &str) -> String {
    let persisted_path = persisted_output_absolute_path(content)
        .unwrap_or_else(|| "unknown persisted path".to_string());
    format!(
        "Large tool output was persisted instead of inlined.\nPersisted path: \
         {persisted_path}\nPreserve only the conclusion, referenced path, and any error."
    )
}

fn sanitize_compact_summary(summary: &str) -> String {
    let had_route_sensitive_content = summary_has_route_sensitive_content(summary);
    let mut sanitized = summary.trim().to_string();
    sanitized = direct_child_validation_regex()
        .replace_all(
            &sanitized,
            "direct-child validation rejected a stale child reference; use the live direct-child \
             snapshot or the latest live tool result instead.",
        )
        .into_owned();
    sanitized = child_agent_reference_block_regex()
        .replace_all(
            &sanitized,
            "Child agent reference metadata existed earlier, but compacted history is not an \
             authoritative routing source.",
        )
        .into_owned();
    for (regex, replacement) in [
        (
            route_key_regex("agentId"),
            "${key}<latest-direct-child-agentId>",
        ),
        (
            route_key_regex("childAgentId"),
            "${key}<latest-direct-child-agentId>",
        ),
        (route_key_regex("parentAgentId"), "${key}<parent-agentId>"),
        (route_key_regex("subRunId"), "${key}<direct-child-subRunId>"),
        (route_key_regex("parentSubRunId"), "${key}<parent-subRunId>"),
        (route_key_regex("sessionId"), "${key}<session-id>"),
        (
            route_key_regex("childSessionId"),
            "${key}<child-session-id>",
        ),
        (route_key_regex("openSessionId"), "${key}<child-session-id>"),
    ] {
        sanitized = regex.replace_all(&sanitized, replacement).into_owned();
    }
    sanitized = exact_agent_instruction_regex()
        .replace_all(
            &sanitized,
            "Use only the latest live child snapshot or tool result for agent routing.",
        )
        .into_owned();
    sanitized = raw_root_agent_id_regex()
        .replace_all(&sanitized, "<agent-id>")
        .into_owned();
    sanitized = raw_agent_id_regex()
        .replace_all(&sanitized, "<agent-id>")
        .into_owned();
    sanitized = raw_subrun_id_regex()
        .replace_all(&sanitized, "<subrun-id>")
        .into_owned();
    sanitized = raw_session_id_regex()
        .replace_all(&sanitized, "<session-id>")
        .into_owned();
    sanitized = collapse_compaction_whitespace(&sanitized);
    if had_route_sensitive_content {
        ensure_compact_boundary_section(&sanitized)
    } else {
        sanitized
    }
}

fn ensure_compact_boundary_section(summary: &str) -> String {
    if summary.contains("## Compact Boundary") {
        return summary.to_string();
    }
    format!(
        "## Compact Boundary\n- Historical `agentId`, `subRunId`, and `sessionId` values from \
         compacted history are non-authoritative.\n- Use the live direct-child snapshot or the \
         latest live tool result / child notification for routing.\n\n{}",
        summary.trim()
    )
}

fn summary_has_route_sensitive_content(summary: &str) -> bool {
    direct_child_validation_regex().is_match(summary)
        || child_agent_reference_block_regex().is_match(summary)
        || exact_agent_instruction_regex().is_match(summary)
        || raw_root_agent_id_regex().is_match(summary)
        || raw_agent_id_regex().is_match(summary)
        || raw_subrun_id_regex().is_match(summary)
        || raw_session_id_regex().is_match(summary)
        || [
            route_key_regex("agentId"),
            route_key_regex("childAgentId"),
            route_key_regex("parentAgentId"),
            route_key_regex("subRunId"),
            route_key_regex("parentSubRunId"),
            route_key_regex("sessionId"),
            route_key_regex("childSessionId"),
            route_key_regex("openSessionId"),
        ]
        .into_iter()
        .any(|regex| regex.is_match(summary))
}

fn child_agent_reference_block_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?is)Child agent reference:\s*(?:\n- .*)+")
            .expect("child agent reference regex should compile")
    })
}

fn direct_child_validation_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?i)not a direct child of caller[^\n]*")
            .expect("direct child validation regex should compile")
    })
}

fn route_key_regex(key: &str) -> &'static Regex {
    static AGENT_ID: OnceLock<Regex> = OnceLock::new();
    static CHILD_AGENT_ID: OnceLock<Regex> = OnceLock::new();
    static PARENT_AGENT_ID: OnceLock<Regex> = OnceLock::new();
    static SUB_RUN_ID: OnceLock<Regex> = OnceLock::new();
    static PARENT_SUB_RUN_ID: OnceLock<Regex> = OnceLock::new();
    static SESSION_ID: OnceLock<Regex> = OnceLock::new();
    static CHILD_SESSION_ID: OnceLock<Regex> = OnceLock::new();
    static OPEN_SESSION_ID: OnceLock<Regex> = OnceLock::new();
    let slot = match key {
        "agentId" => &AGENT_ID,
        "childAgentId" => &CHILD_AGENT_ID,
        "parentAgentId" => &PARENT_AGENT_ID,
        "subRunId" => &SUB_RUN_ID,
        "parentSubRunId" => &PARENT_SUB_RUN_ID,
        "sessionId" => &SESSION_ID,
        "childSessionId" => &CHILD_SESSION_ID,
        "openSessionId" => &OPEN_SESSION_ID,
        other => panic!("unsupported route key regex: {other}"),
    };
    slot.get_or_init(|| {
        Regex::new(&format!(
            r"(?i)(?P<key>`?{key}`?\s*[:=]\s*`?)[^`\s,;\])]+`?"
        ))
        .expect("route key regex should compile")
    })
}

fn exact_agent_instruction_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r"(?i)(use this exact `agentid` value[^\n]*|copy it byte-for-byte[^\n]*|keep `agentid` exact[^\n]*)",
        )
        .expect("exact agent instruction regex should compile")
    })
}

fn raw_root_agent_id_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"\broot-agent:[A-Za-z0-9._:-]+\b")
            .expect("raw root agent id regex should compile")
    })
}

fn raw_agent_id_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"\bagent-[A-Za-z0-9._:-]+\b").expect("raw agent id regex should compile")
    })
}

fn raw_subrun_id_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"\bsubrun-[A-Za-z0-9._:-]+\b").expect("raw subrun regex should compile")
    })
}

fn raw_session_id_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"\bsession-[A-Za-z0-9._:-]+\b").expect("raw session regex should compile")
    })
}

fn strip_child_agent_reference_hint(content: &str) -> String {
    let Some((prefix, child_ref_block)) = content.split_once("\n\nChild agent reference:") else {
        return content.to_string();
    };
    let mut has_reference_fields = false;
    for line in child_ref_block.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("- agentId:")
            || trimmed.starts_with("- subRunId:")
            || trimmed.starts_with("- openSessionId:")
            || trimmed.starts_with("- status:")
        {
            has_reference_fields = true;
        }
    }
    let child_ref_summary = if has_reference_fields {
        "Child agent reference existed in the original tool result. Do not reuse any agentId, \
         subRunId, or sessionId from compacted history; rely on the latest live tool result or \
         current direct-child snapshot instead."
            .to_string()
    } else {
        "Child agent reference metadata existed in the original tool result, but compacted history \
         is not an authoritative source for later agent routing."
            .to_string()
    };
    let prefix = prefix.trim();
    if prefix.is_empty() {
        child_ref_summary
    } else {
        format!("{prefix}\n\n{child_ref_summary}")
    }
}

/// 检查消息是否可以被压缩。
#[cfg(test)]
fn can_compact(messages: &[LlmMessage], keep_recent_turns: usize) -> bool {
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
    config: &CompactConfig,
) -> bool {
    loop {
        let prepared_input = prepare_compact_input(prefix);
        if prepared_input.messages.is_empty() {
            return false;
        }

        let system_prompt = render_compact_system_prompt(
            compact_prompt_context,
            prepared_input.prompt_mode,
            config.custom_instructions.as_deref(),
        );
        if compact_request_fits_window(
            &prepared_input.messages,
            &system_prompt,
            limits,
            config.summary_reserve_tokens,
        ) {
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
    summary_reserve_tokens: usize,
) -> bool {
    estimate_request_tokens(request_messages, Some(system_prompt))
        <= effective_context_window(limits, summary_reserve_tokens)
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
    let normalized = strip_outer_markdown_code_fence(content);
    let has_analysis = extract_xml_block(&normalized, "analysis").is_some();
    if !has_analysis {
        log::warn!("compact: missing <analysis> block in LLM response");
    }

    if has_opening_xml_tag(&normalized, "summary") && !has_closing_xml_tag(&normalized, "summary") {
        return Err(AstrError::LlmStreamError(
            "compact response missing closing </summary> tag".to_string(),
        ));
    }

    let mut used_fallback = false;
    let summary = if let Some(summary) = extract_xml_block(&normalized, "summary") {
        summary.to_string()
    } else if let Some(structured) = extract_structured_summary_fallback(&normalized) {
        used_fallback = true;
        structured
    } else {
        let fallback = strip_xml_block(&normalized, "analysis");
        let fallback = clean_compact_fallback_text(&fallback);
        if fallback.is_empty() {
            return Err(AstrError::LlmStreamError(
                "compact response missing <summary> block".to_string(),
            ));
        }
        log::warn!("compact: missing <summary> block, falling back to raw content");
        used_fallback = true;
        fallback
    };
    if summary.is_empty() {
        return Err(AstrError::LlmStreamError(
            "compact summary response was empty".to_string(),
        ));
    }

    Ok(ParsedCompactOutput {
        summary,
        has_analysis,
        used_fallback,
    })
}

fn extract_structured_summary_fallback(content: &str) -> Option<String> {
    let cleaned = clean_compact_fallback_text(content);
    let lower = cleaned.to_ascii_lowercase();
    let candidates = ["## summary", "# summary", "summary:"];
    for marker in candidates {
        if let Some(start) = lower.find(marker) {
            let body = cleaned[start + marker.len()..].trim();
            if !body.is_empty() {
                return Some(body.to_string());
            }
        }
    }
    None
}

fn extract_xml_block<'a>(content: &'a str, tag: &str) -> Option<&'a str> {
    xml_block_regex(tag)
        .captures(content)
        .and_then(|captures| captures.name("body"))
        .map(|body| body.as_str().trim())
}

fn strip_xml_block(content: &str, tag: &str) -> String {
    xml_block_regex(tag).replace(content, "").into_owned()
}

fn has_opening_xml_tag(content: &str, tag: &str) -> bool {
    xml_opening_tag_regex(tag).is_match(content)
}

fn has_closing_xml_tag(content: &str, tag: &str) -> bool {
    xml_closing_tag_regex(tag).is_match(content)
}

fn strip_markdown_code_fence(content: &str) -> String {
    let trimmed = content.trim();
    if !trimmed.starts_with("```") {
        return trimmed.to_string();
    }

    let mut lines = trimmed.lines();
    let Some(first_line) = lines.next() else {
        return trimmed.to_string();
    };
    if !first_line.trim_start().starts_with("```") {
        return trimmed.to_string();
    }

    let body = lines.collect::<Vec<_>>().join("\n");
    let body = body.trim_end();
    body.strip_suffix("```").unwrap_or(body).trim().to_string()
}

fn strip_outer_markdown_code_fence(content: &str) -> String {
    let mut current = content.trim().to_string();
    loop {
        let stripped = strip_markdown_code_fence(&current);
        if stripped == current {
            return current;
        }
        current = stripped;
    }
}

fn clean_compact_fallback_text(content: &str) -> String {
    let without_code_fence = strip_outer_markdown_code_fence(content);
    let lines = without_code_fence
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>();
    let first_meaningful = lines
        .iter()
        .position(|line| !line.trim().is_empty())
        .unwrap_or(lines.len());
    let cleaned = lines
        .into_iter()
        .skip(first_meaningful)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();
    strip_leading_summary_preamble(&cleaned)
}

fn strip_leading_summary_preamble(content: &str) -> String {
    let mut lines = content.lines();
    let Some(first_line) = lines.next() else {
        return String::new();
    };
    let trimmed_first_line = first_line.trim();
    if is_summary_preamble_line(trimmed_first_line) {
        return lines.collect::<Vec<_>>().join("\n").trim().to_string();
    }
    content.trim().to_string()
}

fn is_summary_preamble_line(line: &str) -> bool {
    let normalized = line
        .trim_matches(|ch: char| matches!(ch, '*' | '#' | '-' | ':' | ' '))
        .trim();
    normalized.eq_ignore_ascii_case("summary")
        || normalized.eq_ignore_ascii_case("here is the summary")
        || normalized.eq_ignore_ascii_case("compact summary")
        || normalized.eq_ignore_ascii_case("here's the summary")
}

fn xml_block_regex(tag: &str) -> &'static Regex {
    static SUMMARY_REGEX: OnceLock<Regex> = OnceLock::new();
    static ANALYSIS_REGEX: OnceLock<Regex> = OnceLock::new();

    match tag {
        "summary" => SUMMARY_REGEX.get_or_init(|| {
            Regex::new(r"(?is)<summary(?:\s+[^>]*)?\s*>(?P<body>.*?)</summary\s*>")
                .expect("summary regex should compile")
        }),
        "analysis" => ANALYSIS_REGEX.get_or_init(|| {
            Regex::new(r"(?is)<analysis(?:\s+[^>]*)?\s*>(?P<body>.*?)</analysis\s*>")
                .expect("analysis regex should compile")
        }),
        other => panic!("unsupported compact xml tag: {other}"),
    }
}

fn xml_opening_tag_regex(tag: &str) -> &'static Regex {
    static SUMMARY_REGEX: OnceLock<Regex> = OnceLock::new();
    static ANALYSIS_REGEX: OnceLock<Regex> = OnceLock::new();

    match tag {
        "summary" => SUMMARY_REGEX.get_or_init(|| {
            Regex::new(r"(?i)<summary(?:\s+[^>]*)?\s*>")
                .expect("summary opening regex should compile")
        }),
        "analysis" => ANALYSIS_REGEX.get_or_init(|| {
            Regex::new(r"(?i)<analysis(?:\s+[^>]*)?\s*>")
                .expect("analysis opening regex should compile")
        }),
        other => panic!("unsupported compact xml tag: {other}"),
    }
}

fn xml_closing_tag_regex(tag: &str) -> &'static Regex {
    static SUMMARY_REGEX: OnceLock<Regex> = OnceLock::new();
    static ANALYSIS_REGEX: OnceLock<Regex> = OnceLock::new();

    match tag {
        "summary" => SUMMARY_REGEX.get_or_init(|| {
            Regex::new(r"(?i)</summary\s*>").expect("summary closing regex should compile")
        }),
        "analysis" => ANALYSIS_REGEX.get_or_init(|| {
            Regex::new(r"(?i)</analysis\s*>").expect("analysis closing regex should compile")
        }),
        other => panic!("unsupported compact xml tag: {other}"),
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

    fn test_compact_config() -> CompactConfig {
        CompactConfig {
            keep_recent_turns: 1,
            trigger: astrcode_core::CompactTrigger::Manual,
            summary_reserve_tokens: 20_000,
            max_retry_attempts: 3,
            history_path: None,
            custom_instructions: None,
        }
    }

    #[test]
    fn render_compact_system_prompt_keeps_do_not_continue_instruction_intact() {
        let prompt = render_compact_system_prompt(None, CompactPromptMode::Fresh, None);

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
            None,
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
    fn parse_compact_output_requires_non_empty_content() {
        let error = parse_compact_output("   ").expect_err("empty compact output should fail");
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
    fn parse_compact_output_accepts_case_insensitive_summary_block() {
        let parsed = parse_compact_output("<ANALYSIS>draft</ANALYSIS><SUMMARY>Section</SUMMARY>")
            .expect("summary should parse");

        assert_eq!(parsed.summary, "Section");
        assert!(parsed.has_analysis);
    }

    #[test]
    fn parse_compact_output_falls_back_to_plain_text_summary() {
        let parsed = parse_compact_output("## Goal\n- preserve current task")
            .expect("plain text summary should parse");

        assert_eq!(parsed.summary, "## Goal\n- preserve current task");
        assert!(!parsed.has_analysis);
    }

    #[test]
    fn parse_compact_output_strips_outer_code_fence_before_parsing() {
        let parsed = parse_compact_output(
            "```xml\n<analysis>draft</analysis>\n<summary>Section</summary>\n```",
        )
        .expect("fenced xml summary should parse");

        assert_eq!(parsed.summary, "Section");
        assert!(parsed.has_analysis);
    }

    #[test]
    fn parse_compact_output_strips_common_summary_preamble_in_fallback() {
        let parsed = parse_compact_output("Summary:\n## Goal\n- preserve current task")
            .expect("summary preamble fallback should parse");

        assert_eq!(parsed.summary, "## Goal\n- preserve current task");
    }

    #[test]
    fn parse_compact_output_accepts_summary_tag_attributes() {
        let parsed = parse_compact_output(
            "<analysis class=\"draft\">draft</analysis><summary \
             format=\"markdown\">Section</summary>",
        )
        .expect("tag attributes should parse");

        assert_eq!(parsed.summary, "Section");
        assert!(parsed.has_analysis);
    }

    #[test]
    fn parse_compact_output_does_not_treat_analysis_only_as_summary() {
        let error = parse_compact_output("<analysis>draft</analysis>")
            .expect_err("analysis-only output should still fail");

        assert!(error.to_string().contains("missing <summary> block"));
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
    fn prepare_compact_input_strips_history_note_from_previous_summary() {
        let filtered = prepare_compact_input(&[LlmMessage::User {
            content: CompactSummaryEnvelope::new("older summary")
                .with_history_path("~/.astrcode/projects/demo/sessions/abc/session-abc.jsonl")
                .render(),
            origin: UserMessageOrigin::CompactSummary,
        }]);

        assert!(matches!(
            filtered.prompt_mode,
            CompactPromptMode::Incremental { ref previous_summary }
                if previous_summary == "older summary"
        ));
    }

    #[test]
    fn prepare_compact_input_skips_synthetic_user_messages() {
        let filtered = prepare_compact_input(&[
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

        assert_eq!(filtered.messages.len(), 1);
        assert!(matches!(
            &filtered.messages[0],
            LlmMessage::User {
                content,
                origin: UserMessageOrigin::User
            } if content == "real user"
        ));
    }

    #[test]
    fn normalize_compaction_tool_content_removes_exact_child_identifiers() {
        let normalized = normalize_compaction_tool_content(
            "spawn 已在后台启动。\n\nChild agent reference:\n- agentId: agent-1\n- subRunId: \
             subrun-1\n- sessionId: session-parent\n- openSessionId: session-child\n- status: \
             running\nUse this exact `agentId` value in later send/observe/close calls.",
        );

        assert!(normalized.contains("spawn 已在后台启动。"));
        assert!(normalized.contains("Do not reuse any agentId"));
        assert!(!normalized.contains("agent-1"));
        assert!(!normalized.contains("subrun-1"));
        assert!(!normalized.contains("session-child"));
    }

    #[test]
    fn sanitize_compact_summary_replaces_stale_route_identifiers_with_boundary_guidance() {
        let sanitized = sanitize_compact_summary(
            "## Progress\n- Spawned agent-3 and later called observe(agent-2).\n- Error: agent \
             'agent-2' is not a direct child of caller 'agent-root:session-parent' (actual \
             parent: agent-1); send/observe/close only support direct children.\n- Child ref \
             payload: agentId=agent-2 subRunId=subrun-2 openSessionId=session-child-2",
        );

        assert!(sanitized.contains("## Compact Boundary"));
        assert!(sanitized.contains("live direct-child snapshot"));
        assert!(sanitized.contains("<agent-id>"));
        assert!(sanitized.contains("<subrun-id>") || sanitized.contains("<direct-child-subRunId>"));
        assert!(sanitized.contains("<child-session-id>") || sanitized.contains("<session-id>"));
        assert!(!sanitized.contains("agent-2"));
        assert!(!sanitized.contains("subrun-2"));
        assert!(!sanitized.contains("session-child-2"));
        assert!(!sanitized.contains("not a direct child of caller"));
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
            &test_compact_config(),
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
