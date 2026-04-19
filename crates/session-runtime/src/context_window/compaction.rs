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

use std::{collections::HashSet, sync::OnceLock};

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

#[path = "compaction/protocol.rs"]
mod protocol;
use protocol::*;

/// 压缩配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompactConfig {
    /// 保留最近的用户 turn 数量。
    pub keep_recent_turns: usize,
    /// 额外保留最近真实用户消息的数量。
    pub keep_recent_user_messages: usize,
    /// 压缩触发方式。
    pub trigger: astrcode_core::CompactTrigger,
    /// compact 请求自身保留的输出预算。
    pub summary_reserve_tokens: usize,
    /// compact 请求的最大输出 token 上限。
    pub max_output_tokens: usize,
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
    /// 最近真实用户消息的极短目的摘要。
    pub recent_user_context_digest: Option<String>,
    /// compact 后重新注入的最近真实用户消息原文。
    pub recent_user_context_messages: Vec<String>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct CompactContractViolation {
    detail: String,
}

impl CompactContractViolation {
    fn from_parsed_output(parsed: &ParsedCompactOutput) -> Option<Self> {
        if parsed.used_fallback {
            return Some(Self {
                detail: "response did not contain a strict <summary> XML block and required \
                         fallback parsing"
                    .to_string(),
            });
        }
        if !parsed.has_analysis {
            return Some(Self {
                detail: "response omitted the required <analysis> block".to_string(),
            });
        }
        if !parsed.has_recent_user_context_digest_block {
            return Some(Self {
                detail: "response omitted the required <recent_user_context_digest> block"
                    .to_string(),
            });
        }
        None
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct CompactRetryState {
    salvage_attempts: usize,
    contract_retry_count: usize,
    contract_repair_feedback: Option<String>,
}

impl CompactRetryState {
    fn schedule_contract_retry(&mut self, detail: String) {
        self.contract_retry_count = self.contract_retry_count.saturating_add(1);
        self.contract_repair_feedback = Some(detail);
    }

    fn note_salvage_attempt(&mut self) {
        self.salvage_attempts = self.salvage_attempts.saturating_add(1);
    }
}

#[derive(Debug, Clone)]
struct CompactExecutionResult {
    parsed_output: ParsedCompactOutput,
    prepared_input: PreparedCompactInput,
    retry_state: CompactRetryState,
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
    let recent_user_context_messages =
        collect_recent_user_context_messages(messages, config.keep_recent_user_messages);
    let preserved_recent_turns = config
        .keep_recent_turns
        .max(config.keep_recent_user_messages)
        .max(1);
    let Some(mut split) = split_for_compaction(messages, preserved_recent_turns) else {
        return Ok(None);
    };

    let pre_tokens = estimate_request_tokens(messages, compact_prompt_context);
    let effective_max_output_tokens = config
        .max_output_tokens
        .min(gateway.model_limits().max_output_tokens)
        .max(1);
    let Some(execution) = execute_compact_request_with_retries(
        gateway,
        &mut split,
        compact_prompt_context,
        &config,
        &recent_user_context_messages,
        effective_max_output_tokens,
        cancel,
    )
    .await?
    else {
        return Ok(None);
    };

    let summary = {
        let summary = sanitize_compact_summary(&execution.parsed_output.summary);
        if let Some(history_path) = config.history_path.as_deref() {
            CompactSummaryEnvelope::new(summary)
                .with_history_path(history_path)
                .render_body()
        } else {
            summary
        }
    };
    let recent_user_context_digest = execution
        .parsed_output
        .recent_user_context_digest
        .as_deref()
        .map(sanitize_recent_user_context_digest)
        .filter(|value| !value.is_empty());
    let compacted_messages = compacted_messages(
        &summary,
        recent_user_context_digest.as_deref(),
        &recent_user_context_messages,
        split.keep_start,
        split.suffix,
    );
    Ok(Some(build_compact_result(
        compacted_messages,
        summary,
        recent_user_context_digest,
        recent_user_context_messages,
        preserved_recent_turns,
        pre_tokens,
        split.keep_start,
        compact_prompt_context,
        &config,
        execution,
    )))
}

#[derive(Debug, Clone)]
struct CompactionSplit {
    prefix: Vec<LlmMessage>,
    suffix: Vec<LlmMessage>,
    keep_start: usize,
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
    recent_user_context_messages: &[RecentUserContextMessage],
) -> bool {
    loop {
        let prepared_input = prepare_compact_input(prefix);
        if prepared_input.messages.is_empty() {
            return false;
        }

        let system_prompt = render_compact_system_prompt(
            compact_prompt_context,
            prepared_input.prompt_mode,
            config
                .max_output_tokens
                .min(limits.max_output_tokens)
                .max(1),
            recent_user_context_messages,
            config.custom_instructions.as_deref(),
            None,
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

async fn execute_compact_request_with_retries(
    gateway: &KernelGateway,
    split: &mut CompactionSplit,
    compact_prompt_context: Option<&str>,
    config: &CompactConfig,
    recent_user_context_messages: &[RecentUserContextMessage],
    effective_max_output_tokens: usize,
    cancel: CancelToken,
) -> Result<Option<CompactExecutionResult>> {
    let mut retry_state = CompactRetryState::default();
    loop {
        if !trim_prefix_until_compact_request_fits(
            &mut split.prefix,
            compact_prompt_context,
            gateway.model_limits(),
            config,
            recent_user_context_messages,
        ) {
            return Err(AstrError::Internal(
                "compact request could not fit within summarization window".to_string(),
            ));
        }

        let prepared_input = prepare_compact_input(&split.prefix);
        if prepared_input.messages.is_empty() {
            return Ok(None);
        }

        let request = build_compact_request(
            prepared_input.messages.clone(),
            compact_prompt_context,
            &prepared_input.prompt_mode,
            effective_max_output_tokens,
            recent_user_context_messages,
            config.custom_instructions.as_deref(),
            retry_state.contract_repair_feedback.as_deref(),
            cancel.clone(),
        );

        match gateway.call_llm(request, None).await {
            Ok(output) => match parse_compact_output(&output.content) {
                Ok(parsed_output) => {
                    if let Some(violation) =
                        CompactContractViolation::from_parsed_output(&parsed_output)
                    {
                        if retry_state.contract_retry_count < config.max_retry_attempts {
                            retry_state.schedule_contract_retry(violation.detail);
                            continue;
                        }
                    }
                    return Ok(Some(CompactExecutionResult {
                        parsed_output,
                        prepared_input,
                        retry_state,
                    }));
                },
                Err(error) if retry_state.contract_retry_count < config.max_retry_attempts => {
                    retry_state.schedule_contract_retry(error.to_string());
                    continue;
                },
                Err(error) => return Err(error),
            },
            Err(error)
                if is_prompt_too_long(&error)
                    && retry_state.salvage_attempts < config.max_retry_attempts =>
            {
                retry_state.note_salvage_attempt();
                if !drop_oldest_compaction_unit(&mut split.prefix) {
                    return Err(AstrError::Internal(error.to_string()));
                }
                split.keep_start = split.prefix.len();
            },
            Err(error) => return Err(AstrError::Internal(error.to_string())),
        }
    }
}

fn build_compact_request(
    messages: Vec<LlmMessage>,
    compact_prompt_context: Option<&str>,
    prompt_mode: &CompactPromptMode,
    effective_max_output_tokens: usize,
    recent_user_context_messages: &[RecentUserContextMessage],
    custom_instructions: Option<&str>,
    contract_repair_feedback: Option<&str>,
    cancel: CancelToken,
) -> LlmRequest {
    LlmRequest::new(messages, Vec::new(), cancel)
        .with_system(render_compact_system_prompt(
            compact_prompt_context,
            prompt_mode.clone(),
            effective_max_output_tokens,
            recent_user_context_messages,
            custom_instructions,
            contract_repair_feedback,
        ))
        .with_max_output_tokens_override(effective_max_output_tokens)
}

fn build_compact_result(
    compacted_messages: Vec<LlmMessage>,
    summary: String,
    recent_user_context_digest: Option<String>,
    recent_user_context_messages: Vec<RecentUserContextMessage>,
    preserved_recent_turns: usize,
    pre_tokens: usize,
    messages_removed: usize,
    compact_prompt_context: Option<&str>,
    config: &CompactConfig,
    execution: CompactExecutionResult,
) -> CompactResult {
    let CompactExecutionResult {
        parsed_output,
        prepared_input,
        retry_state,
    } = execution;
    let post_tokens_estimate = estimate_request_tokens(&compacted_messages, compact_prompt_context);
    let output_summary_chars = summary.chars().count().min(u32::MAX as usize) as u32;

    CompactResult {
        messages: compacted_messages,
        summary,
        recent_user_context_digest,
        recent_user_context_messages: recent_user_context_messages
            .into_iter()
            .map(|message| message.content)
            .collect(),
        preserved_recent_turns,
        pre_tokens,
        post_tokens_estimate,
        messages_removed,
        tokens_freed: pre_tokens.saturating_sub(post_tokens_estimate),
        timestamp: Utc::now(),
        meta: CompactAppliedMeta {
            mode: prepared_input
                .prompt_mode
                .compact_mode(retry_state.salvage_attempts),
            instructions_present: config
                .custom_instructions
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty()),
            fallback_used: parsed_output.used_fallback || retry_state.salvage_attempts > 0,
            retry_count: retry_state.salvage_attempts.min(u32::MAX as usize) as u32,
            input_units: prepared_input.input_units.min(u32::MAX as usize) as u32,
            output_summary_chars,
        },
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

fn compacted_messages(
    summary: &str,
    recent_user_context_digest: Option<&str>,
    recent_user_context_messages: &[RecentUserContextMessage],
    keep_start: usize,
    suffix: Vec<LlmMessage>,
) -> Vec<LlmMessage> {
    let recent_user_context_indices = recent_user_context_messages
        .iter()
        .map(|message| message.index)
        .collect::<HashSet<_>>();
    let mut messages = vec![LlmMessage::User {
        content: format_compact_summary(summary),
        origin: UserMessageOrigin::CompactSummary,
    }];
    if let Some(digest) = recent_user_context_digest.filter(|value| !value.trim().is_empty()) {
        messages.push(LlmMessage::User {
            content: digest.trim().to_string(),
            origin: UserMessageOrigin::RecentUserContextDigest,
        });
    }
    for message in recent_user_context_messages {
        messages.push(LlmMessage::User {
            content: message.content.clone(),
            origin: UserMessageOrigin::RecentUserContext,
        });
    }
    messages.extend(
        suffix
            .into_iter()
            .enumerate()
            .filter(|(offset, message)| {
                let is_reinjected_real_user_message = matches!(
                    message,
                    LlmMessage::User {
                        origin: UserMessageOrigin::User,
                        ..
                    }
                ) && recent_user_context_indices
                    .contains(&(keep_start + offset));
                !is_reinjected_real_user_message
            })
            .map(|(_, message)| message),
    );
    messages
}

#[cfg(test)]
mod tests;
