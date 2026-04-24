//! server-owned conversation read-model bridge。
//!
//! Why: route / terminal surface 不应直接暴露底层 conversation query DTO；
//! server 在这里收口正式 read-model surface。
#![allow(dead_code)]

use std::collections::{HashMap, HashSet};

use astrcode_core::{
    AgentEvent, ChildAgentRef, ChildSessionNotification, ChildSessionNotificationKind,
    CompactAppliedMeta, CompactTrigger, Phase, PromptMetricsPayload, SessionEventRecord,
};
use astrcode_tool_contract::{ToolExecutionResult, ToolOutputStream};
use serde_json::Value;
use tokio::sync::broadcast;

#[path = "conversation_read_model/facts.rs"]
mod facts;
#[path = "conversation_read_model/plan_projection.rs"]
mod plan_projection;

pub(crate) use facts::*;

pub(crate) const ROOT_AGENT_ID: &str = "root-agent";

#[derive(Debug)]
pub(crate) struct ConversationReplayStream {
    pub receiver: broadcast::Receiver<SessionEventRecord>,
    pub live_receiver: broadcast::Receiver<AgentEvent>,
}

#[derive(Debug)]
pub(crate) struct SessionReplay {
    pub history: Vec<SessionEventRecord>,
    pub receiver: broadcast::Receiver<SessionEventRecord>,
    pub live_receiver: broadcast::Receiver<AgentEvent>,
}

#[derive(Debug, Clone)]
pub(crate) struct SessionTranscriptSnapshot {
    pub records: Vec<SessionEventRecord>,
    pub cursor: Option<String>,
    pub phase: Phase,
}

#[derive(Debug)]
pub(crate) struct ConversationStreamReplayFacts {
    pub cursor: Option<String>,
    pub phase: Phase,
    pub seed_records: Vec<SessionEventRecord>,
    pub replay_frames: Vec<ConversationDeltaFrameFacts>,
    pub replay_history: Vec<SessionEventRecord>,
}

#[derive(Default)]
pub(crate) struct ConversationDeltaProjector {
    blocks: Vec<ConversationBlockFacts>,
    block_index: HashMap<String, usize>,
    turn_blocks: HashMap<String, TurnBlockRefs>,
    tool_blocks: HashMap<String, ToolBlockRefs>,
}

#[derive(Default)]
pub(crate) struct ConversationStreamProjector {
    projector: ConversationDeltaProjector,
    last_sent_cursor: Option<String>,
    fallback_live_cursor: Option<String>,
    step_progress: ConversationStepProgressFacts,
}

#[derive(Default, Clone)]
struct TurnBlockRefs {
    current_thinking: Option<String>,
    current_assistant: Option<String>,
    historical_thinking: Vec<String>,
    historical_assistant: Vec<String>,
    pending_thinking: Vec<String>,
    pending_assistant: Vec<String>,
    thinking_count: usize,
    assistant_count: usize,
}

#[derive(Default, Clone)]
struct ToolBlockRefs {
    turn_id: Option<String>,
    call: Option<String>,
    pending_live_stdout_bytes: usize,
    pending_live_stderr_bytes: usize,
}

#[derive(Clone, Copy)]
enum BlockKind {
    Thinking,
    Assistant,
}

#[derive(Clone, Copy)]
enum ProjectionSource {
    Durable,
    Live,
}

impl ProjectionSource {
    fn is_durable(self) -> bool {
        matches!(self, Self::Durable)
    }

    fn is_live(self) -> bool {
        matches!(self, Self::Live)
    }
}

impl ToolBlockRefs {
    fn reconcile_tool_chunk(
        &mut self,
        stream: ToolOutputStream,
        delta: &str,
        source: ProjectionSource,
    ) -> String {
        let pending_live_bytes = match stream {
            ToolOutputStream::Stdout => &mut self.pending_live_stdout_bytes,
            ToolOutputStream::Stderr => &mut self.pending_live_stderr_bytes,
        };

        if matches!(source, ProjectionSource::Live) {
            *pending_live_bytes += delta.len();
            return delta.to_string();
        }

        if *pending_live_bytes == 0 {
            return delta.to_string();
        }

        let consumed = (*pending_live_bytes).min(delta.len());
        *pending_live_bytes -= consumed;
        delta[consumed..].to_string()
    }
}

impl TurnBlockRefs {
    fn current_or_next_block_id(&mut self, turn_id: &str, kind: BlockKind) -> String {
        match kind {
            BlockKind::Thinking => {
                if let Some(block_id) = &self.current_thinking {
                    return block_id.clone();
                }
                self.thinking_count += 1;
                let block_id = turn_scoped_block_id(turn_id, "thinking", self.thinking_count);
                self.current_thinking = Some(block_id.clone());
                block_id
            },
            BlockKind::Assistant => {
                if let Some(block_id) = &self.current_assistant {
                    return block_id.clone();
                }
                self.assistant_count += 1;
                let block_id = turn_scoped_block_id(turn_id, "assistant", self.assistant_count);
                self.current_assistant = Some(block_id.clone());
                block_id
            },
        }
    }

    fn block_id_for_finalize(&mut self, turn_id: &str, kind: BlockKind) -> String {
        match kind {
            BlockKind::Thinking => {
                if let Some(block_id) = self.pending_thinking.first().cloned() {
                    self.pending_thinking.remove(0);
                    return block_id;
                }
                self.current_or_next_block_id(turn_id, kind)
            },
            BlockKind::Assistant => {
                if let Some(block_id) = self.pending_assistant.first().cloned() {
                    self.pending_assistant.remove(0);
                    return block_id;
                }
                self.current_or_next_block_id(turn_id, kind)
            },
        }
    }

    fn split_after_live_tool_boundary(&mut self) {
        if let Some(block_id) = self.current_thinking.take() {
            self.pending_thinking.push(block_id);
        }
        if let Some(block_id) = self.current_assistant.take() {
            self.pending_assistant.push(block_id);
        }
    }

    fn split_after_durable_tool_boundary(&mut self) {
        if let Some(block_id) = self.current_thinking.take() {
            self.historical_thinking.push(block_id);
        }
        if let Some(block_id) = self.current_assistant.take() {
            self.historical_assistant.push(block_id);
        }
    }

    fn all_block_ids(&self) -> Vec<String> {
        let mut ids = Vec::new();
        ids.extend(self.historical_thinking.iter().cloned());
        ids.extend(self.historical_assistant.iter().cloned());
        ids.extend(self.pending_thinking.iter().cloned());
        ids.extend(self.pending_assistant.iter().cloned());
        if let Some(block_id) = &self.current_thinking {
            ids.push(block_id.clone());
        }
        if let Some(block_id) = &self.current_assistant {
            ids.push(block_id.clone());
        }
        ids
    }
}

fn turn_scoped_block_id(turn_id: &str, role: &str, ordinal: usize) -> String {
    if ordinal <= 1 {
        format!("turn:{turn_id}:{role}")
    } else {
        format!("turn:{turn_id}:{role}:{ordinal}")
    }
}

fn prompt_metrics_block_id(turn_id: Option<&str>, step_index: u32) -> String {
    match turn_id {
        Some(turn_id) => format!("turn:{turn_id}:prompt_metrics:{}", step_index + 1),
        None => format!("session:prompt_metrics:{}", step_index + 1),
    }
}

fn should_suppress_tool_call_block(tool_name: &str, _input: Option<&Value>) -> bool {
    matches!(tool_name, "upsertSessionPlan" | "exitPlanMode")
}

fn summarize_inline_text(text: &str, max_chars: usize) -> Option<String> {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_text(&normalized, max_chars)
}

fn truncate_text(text: &str, max_chars: usize) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    if trimmed.chars().count() <= max_chars {
        return Some(trimmed.to_string());
    }

    Some(trimmed.chars().take(max_chars).collect::<String>() + "...")
}

fn tool_result_summary(result: &ToolExecutionResult) -> String {
    const MAX_SUMMARY_CHARS: usize = 120;

    if result.ok {
        if !result.output.trim().is_empty() {
            summarize_inline_text(&result.output, MAX_SUMMARY_CHARS)
                .unwrap_or_else(|| format!("{} completed", result.tool_name))
        } else {
            format!("{} completed", result.tool_name)
        }
    } else if let Some(error) = &result.error {
        summarize_inline_text(error, MAX_SUMMARY_CHARS)
            .unwrap_or_else(|| format!("{} failed", result.tool_name))
    } else if !result.output.trim().is_empty() {
        summarize_inline_text(&result.output, MAX_SUMMARY_CHARS)
            .unwrap_or_else(|| format!("{} failed", result.tool_name))
    } else {
        format!("{} failed", result.tool_name)
    }
}

fn classify_transcript_error(message: &str) -> ConversationTranscriptErrorKind {
    let lower = message.to_lowercase();
    if lower.contains("context window") || lower.contains("token limit") {
        ConversationTranscriptErrorKind::ContextWindowExceeded
    } else if lower.contains("rate limit") {
        ConversationTranscriptErrorKind::RateLimit
    } else if lower.contains("tool") {
        ConversationTranscriptErrorKind::ToolFatal
    } else {
        ConversationTranscriptErrorKind::ProviderError
    }
}

impl ConversationDeltaProjector {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn seed(&mut self, history: &[SessionEventRecord]) {
        for record in history {
            let _ = self.project_record(record);
        }
    }

    pub(crate) fn blocks(&self) -> &[ConversationBlockFacts] {
        &self.blocks
    }

    pub(crate) fn into_blocks(self) -> Vec<ConversationBlockFacts> {
        self.blocks
    }

    pub(crate) fn project_record(
        &mut self,
        record: &SessionEventRecord,
    ) -> Vec<ConversationDeltaFacts> {
        self.project_event(
            &record.event,
            ProjectionSource::Durable,
            Some(record.event_id.as_str()),
        )
    }

    pub(crate) fn project_live_event(&mut self, event: &AgentEvent) -> Vec<ConversationDeltaFacts> {
        self.project_event(event, ProjectionSource::Live, None)
    }

    fn project_event(
        &mut self,
        event: &AgentEvent,
        source: ProjectionSource,
        durable_event_id: Option<&str>,
    ) -> Vec<ConversationDeltaFacts> {
        match event {
            AgentEvent::UserMessage {
                turn_id, content, ..
            } if source.is_durable() => {
                self.append_user_block(&format!("turn:{turn_id}:user"), turn_id, content)
            },
            AgentEvent::ThinkingDelta { turn_id, delta, .. } => {
                self.append_markdown_streaming_block(turn_id, delta, BlockKind::Thinking)
            },
            AgentEvent::ModelDelta { turn_id, delta, .. } => {
                self.append_markdown_streaming_block(turn_id, delta, BlockKind::Assistant)
            },
            AgentEvent::StreamRetryStarted { turn_id, .. } if source.is_live() => {
                self.reset_live_markdown_blocks(turn_id)
            },
            AgentEvent::AssistantMessage {
                turn_id,
                content,
                reasoning_content,
                step_index,
                ..
            } if source.is_durable() => self.finalize_assistant_block(
                turn_id,
                content,
                reasoning_content.as_deref(),
                *step_index,
            ),
            AgentEvent::PromptMetrics {
                turn_id, metrics, ..
            } => self.upsert_prompt_metrics_block(turn_id.as_deref(), metrics),
            AgentEvent::ToolCallStart {
                turn_id,
                tool_call_id,
                tool_name,
                input,
                ..
            } => {
                if should_suppress_tool_call_block(tool_name, Some(input)) {
                    Vec::new()
                } else {
                    self.start_tool_call(turn_id, tool_call_id, tool_name, Some(input), source)
                }
            },
            AgentEvent::ToolCallDelta {
                turn_id,
                tool_call_id,
                tool_name,
                stream,
                delta,
                ..
            } => {
                if should_suppress_tool_call_block(tool_name, None) {
                    Vec::new()
                } else {
                    self.append_tool_stream(
                        turn_id,
                        tool_call_id,
                        tool_name,
                        *stream,
                        delta,
                        source,
                    )
                }
            },
            AgentEvent::ToolCallResult {
                turn_id, result, ..
            } => {
                if let Some(block) = plan_projection::plan_block_from_tool_result(turn_id, result) {
                    self.push_block(ConversationBlockFacts::Plan(Box::new(block)))
                } else if should_suppress_tool_call_block(&result.tool_name, None) {
                    Vec::new()
                } else {
                    self.complete_tool_call(turn_id, result, source)
                }
            },
            AgentEvent::CompactApplied {
                turn_id,
                trigger,
                summary,
                meta,
                preserved_recent_turns,
                ..
            } if source.is_durable() => {
                let block_id = format!(
                    "system:compact:{}",
                    turn_id
                        .clone()
                        .or_else(|| durable_event_id.map(ToString::to_string))
                        .unwrap_or_else(|| "session".to_string())
                );
                self.append_system_note(
                    &block_id,
                    ConversationSystemNoteKind::Compact,
                    summary,
                    Some(*trigger),
                    Some(meta.clone()),
                    Some(*preserved_recent_turns),
                )
            },
            AgentEvent::ChildSessionNotification { notification, .. } => {
                self.apply_child_notification(notification, source)
            },
            AgentEvent::Error {
                turn_id,
                code,
                message,
                ..
            } if source.is_durable() => self.append_error(turn_id.as_deref(), code, message),
            AgentEvent::TurnDone { turn_id, .. } if source.is_durable() => {
                self.complete_turn(turn_id)
            },
            AgentEvent::PhaseChanged { .. }
            | AgentEvent::SessionStarted { .. }
            | AgentEvent::SubRunStarted { .. }
            | AgentEvent::SubRunFinished { .. }
            | AgentEvent::AgentInputQueued { .. }
            | AgentEvent::AgentInputBatchStarted { .. }
            | AgentEvent::AgentInputBatchAcked { .. }
            | AgentEvent::AgentInputDiscarded { .. }
            | AgentEvent::UserMessage { .. }
            | AgentEvent::AssistantMessage { .. }
            | AgentEvent::StreamRetryStarted { .. }
            | AgentEvent::CompactApplied { .. }
            | AgentEvent::Error { .. }
            | AgentEvent::TurnDone { .. } => Vec::new(),
        }
    }

    fn append_user_block(
        &mut self,
        block_id: &str,
        turn_id: &str,
        content: &str,
    ) -> Vec<ConversationDeltaFacts> {
        if self.block_index.contains_key(block_id) {
            return Vec::new();
        }
        self.push_block(ConversationBlockFacts::User(ConversationUserBlockFacts {
            id: block_id.to_string(),
            turn_id: Some(turn_id.to_string()),
            markdown: content.to_string(),
        }))
    }

    fn append_markdown_streaming_block(
        &mut self,
        turn_id: &str,
        delta: &str,
        kind: BlockKind,
    ) -> Vec<ConversationDeltaFacts> {
        let block_id = self
            .turn_blocks
            .entry(turn_id.to_string())
            .or_default()
            .current_or_next_block_id(turn_id, kind);
        if let Some(index) = self.block_index.get(&block_id).copied() {
            self.append_markdown(index, delta);
            return vec![ConversationDeltaFacts::Patch {
                block_id,
                patch: ConversationBlockPatchFacts::AppendMarkdown {
                    markdown: delta.to_string(),
                },
            }];
        }

        let block = match kind {
            BlockKind::Thinking => {
                ConversationBlockFacts::Thinking(ConversationThinkingBlockFacts {
                    id: block_id.clone(),
                    turn_id: Some(turn_id.to_string()),
                    status: ConversationBlockStatus::Streaming,
                    markdown: delta.to_string(),
                })
            },
            BlockKind::Assistant => {
                ConversationBlockFacts::Assistant(ConversationAssistantBlockFacts {
                    id: block_id,
                    turn_id: Some(turn_id.to_string()),
                    status: ConversationBlockStatus::Streaming,
                    markdown: delta.to_string(),
                    step_index: None,
                })
            },
        };
        self.push_block(block)
    }

    fn reset_live_markdown_blocks(&mut self, turn_id: &str) -> Vec<ConversationDeltaFacts> {
        let block_ids = self
            .turn_blocks
            .get(turn_id)
            .map(|refs| {
                [
                    refs.current_thinking.clone(),
                    refs.current_assistant.clone(),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut deltas = Vec::new();
        for block_id in block_ids {
            let Some(index) = self.block_index.get(&block_id).copied() else {
                continue;
            };
            if self.block_markdown(index).is_empty() {
                continue;
            }
            self.replace_markdown(index, "");
            deltas.push(ConversationDeltaFacts::Patch {
                block_id,
                patch: ConversationBlockPatchFacts::ReplaceMarkdown {
                    markdown: String::new(),
                },
            });
        }
        deltas
    }

    fn finalize_assistant_block(
        &mut self,
        turn_id: &str,
        content: &str,
        reasoning_content: Option<&str>,
        step_index: Option<u32>,
    ) -> Vec<ConversationDeltaFacts> {
        let (assistant_id, thinking_id) = {
            let turn_refs = self.turn_blocks.entry(turn_id.to_string()).or_default();
            (
                turn_refs.block_id_for_finalize(turn_id, BlockKind::Assistant),
                reasoning_content
                    .filter(|value| !value.trim().is_empty())
                    .map(|_| turn_refs.block_id_for_finalize(turn_id, BlockKind::Thinking)),
            )
        };
        let mut deltas = Vec::new();

        if let (Some(reasoning), Some(thinking_id)) = (
            reasoning_content.filter(|value| !value.trim().is_empty()),
            thinking_id,
        ) {
            deltas.extend(self.ensure_full_markdown_block(
                &thinking_id,
                turn_id,
                reasoning,
                BlockKind::Thinking,
            ));
            if let Some(delta) =
                self.complete_block(&thinking_id, ConversationBlockStatus::Complete)
            {
                deltas.push(delta);
            }
        }

        deltas.extend(self.ensure_full_markdown_block(
            &assistant_id,
            turn_id,
            content,
            BlockKind::Assistant,
        ));
        if let Some(delta) = self.refresh_assistant_step_index(&assistant_id, step_index) {
            deltas.push(delta);
        }
        if let Some(delta) = self.complete_block(&assistant_id, ConversationBlockStatus::Complete) {
            deltas.push(delta);
        }
        deltas
    }

    fn ensure_full_markdown_block(
        &mut self,
        block_id: &str,
        turn_id: &str,
        content: &str,
        kind: BlockKind,
    ) -> Vec<ConversationDeltaFacts> {
        if let Some(index) = self.block_index.get(block_id).copied() {
            let existing = self.block_markdown(index);
            self.replace_markdown(index, content);
            if content.starts_with(&existing) {
                let suffix = &content[existing.len()..];
                if suffix.is_empty() {
                    return Vec::new();
                }
                return vec![ConversationDeltaFacts::Patch {
                    block_id: block_id.to_string(),
                    patch: ConversationBlockPatchFacts::AppendMarkdown {
                        markdown: suffix.to_string(),
                    },
                }];
            }
            return vec![ConversationDeltaFacts::Patch {
                block_id: block_id.to_string(),
                patch: ConversationBlockPatchFacts::ReplaceMarkdown {
                    markdown: content.to_string(),
                },
            }];
        }

        let block = match kind {
            BlockKind::Thinking => {
                ConversationBlockFacts::Thinking(ConversationThinkingBlockFacts {
                    id: block_id.to_string(),
                    turn_id: Some(turn_id.to_string()),
                    status: ConversationBlockStatus::Streaming,
                    markdown: content.to_string(),
                })
            },
            BlockKind::Assistant => {
                ConversationBlockFacts::Assistant(ConversationAssistantBlockFacts {
                    id: block_id.to_string(),
                    turn_id: Some(turn_id.to_string()),
                    status: ConversationBlockStatus::Streaming,
                    markdown: content.to_string(),
                    step_index: None,
                })
            },
        };
        self.push_block(block)
    }

    fn refresh_assistant_step_index(
        &mut self,
        block_id: &str,
        step_index: Option<u32>,
    ) -> Option<ConversationDeltaFacts> {
        let index = self.block_index.get(block_id).copied()?;
        let ConversationBlockFacts::Assistant(block) = &mut self.blocks[index] else {
            return None;
        };
        if block.step_index == step_index {
            return None;
        }
        block.step_index = step_index;
        Some(ConversationDeltaFacts::Append {
            block: Box::new(self.blocks[index].clone()),
        })
    }

    fn upsert_prompt_metrics_block(
        &mut self,
        turn_id: Option<&str>,
        metrics: &PromptMetricsPayload,
    ) -> Vec<ConversationDeltaFacts> {
        let block = ConversationBlockFacts::PromptMetrics(ConversationPromptMetricsBlockFacts {
            id: prompt_metrics_block_id(turn_id, metrics.step_index),
            turn_id: turn_id.map(ToString::to_string),
            step_index: metrics.step_index,
            estimated_tokens: metrics.estimated_tokens,
            context_window: metrics.context_window,
            effective_window: metrics.effective_window,
            threshold_tokens: metrics.threshold_tokens,
            truncated_tool_results: metrics.truncated_tool_results,
            provider_input_tokens: metrics.provider_input_tokens,
            provider_output_tokens: metrics.provider_output_tokens,
            cache_creation_input_tokens: metrics.cache_creation_input_tokens,
            cache_read_input_tokens: metrics.cache_read_input_tokens,
            provider_cache_metrics_supported: metrics.provider_cache_metrics_supported,
            prompt_cache_reuse_hits: metrics.prompt_cache_reuse_hits,
            prompt_cache_reuse_misses: metrics.prompt_cache_reuse_misses,
            prompt_cache_unchanged_layers: metrics.prompt_cache_unchanged_layers.clone(),
            prompt_cache_diagnostics: metrics.prompt_cache_diagnostics.clone(),
        });

        self.upsert_block(block)
    }

    fn start_tool_call(
        &mut self,
        turn_id: &str,
        tool_call_id: &str,
        tool_name: &str,
        input: Option<&Value>,
        source: ProjectionSource,
    ) -> Vec<ConversationDeltaFacts> {
        let block_id = format!("tool:{tool_call_id}:call");
        let refs = self
            .tool_blocks
            .entry(tool_call_id.to_string())
            .or_default();
        refs.turn_id = Some(turn_id.to_string());
        refs.call = Some(block_id.clone());
        if self.block_index.contains_key(&block_id) {
            return Vec::new();
        }

        let turn_refs = self.turn_blocks.entry(turn_id.to_string()).or_default();
        if source.is_live() {
            turn_refs.split_after_live_tool_boundary();
        } else {
            turn_refs.split_after_durable_tool_boundary();
        }

        self.push_block(ConversationBlockFacts::ToolCall(Box::new(
            ToolCallBlockFacts {
                id: block_id,
                turn_id: Some(turn_id.to_string()),
                tool_call_id: tool_call_id.to_string(),
                tool_name: tool_name.to_string(),
                status: ConversationBlockStatus::Streaming,
                input: input.cloned(),
                summary: None,
                error: None,
                duration_ms: None,
                truncated: false,
                metadata: None,
                child_ref: None,
                streams: ToolCallStreamsFacts::default(),
            },
        )))
    }

    fn append_tool_stream(
        &mut self,
        turn_id: &str,
        tool_call_id: &str,
        tool_name: &str,
        stream: ToolOutputStream,
        delta: &str,
        source: ProjectionSource,
    ) -> Vec<ConversationDeltaFacts> {
        let mut deltas = self.start_tool_call(turn_id, tool_call_id, tool_name, None, source);
        let refs = self
            .tool_blocks
            .entry(tool_call_id.to_string())
            .or_default();
        let chunk = refs.reconcile_tool_chunk(stream, delta, source);
        if chunk.is_empty() {
            return deltas;
        }

        let Some(call_block_id) = refs.call.clone() else {
            return deltas;
        };
        let Some(index) = self.block_index.get(&call_block_id).copied() else {
            return deltas;
        };
        self.append_tool_stream_content(index, stream, &chunk);
        deltas.push(ConversationDeltaFacts::Patch {
            block_id: call_block_id,
            patch: ConversationBlockPatchFacts::AppendToolStream { stream, chunk },
        });
        deltas
    }

    fn complete_tool_call(
        &mut self,
        turn_id: &str,
        result: &ToolExecutionResult,
        source: ProjectionSource,
    ) -> Vec<ConversationDeltaFacts> {
        let mut deltas = self.start_tool_call(
            turn_id,
            &result.tool_call_id,
            &result.tool_name,
            None,
            source,
        );
        let status = if result.ok {
            ConversationBlockStatus::Complete
        } else {
            ConversationBlockStatus::Failed
        };
        let summary = tool_result_summary(result);
        let refs = self
            .tool_blocks
            .entry(result.tool_call_id.clone())
            .or_default();
        if source.is_durable() {
            refs.pending_live_stdout_bytes = 0;
            refs.pending_live_stderr_bytes = 0;
        }
        let Some(call_block_id) = refs.call.clone() else {
            return deltas;
        };

        if let Some(index) = self.block_index.get(&call_block_id).copied() {
            if self.replace_tool_summary(index, &summary) {
                deltas.push(ConversationDeltaFacts::Patch {
                    block_id: call_block_id.clone(),
                    patch: ConversationBlockPatchFacts::ReplaceSummary {
                        summary: summary.clone(),
                    },
                });
            }
            if self.replace_tool_error(index, result.error.as_deref()) {
                deltas.push(ConversationDeltaFacts::Patch {
                    block_id: call_block_id.clone(),
                    patch: ConversationBlockPatchFacts::ReplaceError {
                        error: result.error.clone(),
                    },
                });
            }
            if self.replace_tool_duration(index, result.duration_ms) {
                deltas.push(ConversationDeltaFacts::Patch {
                    block_id: call_block_id.clone(),
                    patch: ConversationBlockPatchFacts::ReplaceDuration {
                        duration_ms: result.duration_ms,
                    },
                });
            }
            if self.replace_tool_truncated(index, result.truncated) {
                deltas.push(ConversationDeltaFacts::Patch {
                    block_id: call_block_id.clone(),
                    patch: ConversationBlockPatchFacts::SetTruncated {
                        truncated: result.truncated,
                    },
                });
            }
            if let Some(metadata) = &result.metadata {
                if self.replace_tool_metadata(index, metadata) {
                    deltas.push(ConversationDeltaFacts::Patch {
                        block_id: call_block_id.clone(),
                        patch: ConversationBlockPatchFacts::ReplaceMetadata {
                            metadata: metadata.clone(),
                        },
                    });
                }
            }
            if let Some(child_ref) = result
                .continuation()
                .and_then(astrcode_core::ExecutionContinuation::child_agent_ref)
            {
                if self.replace_tool_child_ref(index, child_ref) {
                    deltas.push(ConversationDeltaFacts::Patch {
                        block_id: call_block_id.clone(),
                        patch: ConversationBlockPatchFacts::ReplaceChildRef {
                            child_ref: child_ref.clone(),
                        },
                    });
                }
            }
            if let Some(delta) = self.complete_block(&call_block_id, status) {
                deltas.push(delta);
            }
        }

        deltas
    }

    fn append_system_note(
        &mut self,
        block_id: &str,
        note_kind: ConversationSystemNoteKind,
        markdown: &str,
        compact_trigger: Option<CompactTrigger>,
        compact_meta: Option<CompactAppliedMeta>,
        compact_preserved_recent_turns: Option<u32>,
    ) -> Vec<ConversationDeltaFacts> {
        if self.block_index.contains_key(block_id) {
            return Vec::new();
        }
        self.push_block(ConversationBlockFacts::SystemNote(
            ConversationSystemNoteBlockFacts {
                id: block_id.to_string(),
                note_kind,
                markdown: markdown.to_string(),
                compact_trigger,
                compact_meta,
                compact_preserved_recent_turns,
            },
        ))
    }

    fn apply_child_notification(
        &mut self,
        notification: &ChildSessionNotification,
        source: ProjectionSource,
    ) -> Vec<ConversationDeltaFacts> {
        let mut deltas = Vec::new();
        if let Some(source_tool_call_id) = notification.source_tool_call_id.as_deref() {
            if let Some(call_block_id) = self
                .tool_blocks
                .get(source_tool_call_id)
                .and_then(|refs| refs.call.clone())
            {
                if let Some(index) = self.block_index.get(&call_block_id).copied() {
                    if self.replace_tool_child_ref(index, &notification.child_ref) {
                        deltas.push(ConversationDeltaFacts::Patch {
                            block_id: call_block_id,
                            patch: ConversationBlockPatchFacts::ReplaceChildRef {
                                child_ref: notification.child_ref.clone(),
                            },
                        });
                    }
                }
            }
        }

        if source.is_durable() {
            deltas.extend(self.append_child_handoff(notification));
        }
        deltas
    }

    fn append_child_handoff(
        &mut self,
        notification: &ChildSessionNotification,
    ) -> Vec<ConversationDeltaFacts> {
        let block_id = format!("child:{}", notification.notification_id);
        if self.block_index.contains_key(&block_id) {
            return Vec::new();
        }
        self.push_block(ConversationBlockFacts::ChildHandoff(
            ConversationChildHandoffBlockFacts {
                id: block_id,
                handoff_kind: match notification.kind {
                    ChildSessionNotificationKind::Started
                    | ChildSessionNotificationKind::Resumed => {
                        ConversationChildHandoffKind::Delegated
                    },
                    ChildSessionNotificationKind::ProgressSummary
                    | ChildSessionNotificationKind::Waiting => {
                        ConversationChildHandoffKind::Progress
                    },
                    ChildSessionNotificationKind::Delivered
                    | ChildSessionNotificationKind::Closed
                    | ChildSessionNotificationKind::Failed => {
                        ConversationChildHandoffKind::Returned
                    },
                },
                child_ref: notification.child_ref.clone(),
                message: notification
                    .delivery
                    .as_ref()
                    .map(|delivery| delivery.payload.message().to_string()),
            },
        ))
    }

    fn append_error(
        &mut self,
        turn_id: Option<&str>,
        _code: &str,
        message: &str,
    ) -> Vec<ConversationDeltaFacts> {
        let block_id = format!("turn:{}:error", turn_id.unwrap_or("session"));
        if self.block_index.contains_key(&block_id) {
            return Vec::new();
        }
        self.push_block(ConversationBlockFacts::Error(ConversationErrorBlockFacts {
            id: block_id,
            turn_id: turn_id.map(ToString::to_string),
            code: classify_transcript_error(message),
            message: message.to_string(),
        }))
    }

    fn complete_turn(&mut self, turn_id: &str) -> Vec<ConversationDeltaFacts> {
        let Some(refs) = self.turn_blocks.get(turn_id).cloned() else {
            return Vec::new();
        };
        let mut deltas = Vec::new();
        for block_id in refs.all_block_ids() {
            if let Some(delta) =
                self.complete_streaming_block(&block_id, ConversationBlockStatus::Complete)
            {
                deltas.push(delta);
            }
        }
        let tool_blocks = self
            .tool_blocks
            .values()
            .filter(|tool| tool.turn_id.as_deref() == Some(turn_id))
            .cloned()
            .collect::<Vec<_>>();
        for tool in tool_blocks {
            if let Some(call) = &tool.call {
                if let Some(delta) =
                    self.complete_streaming_block(call, ConversationBlockStatus::Complete)
                {
                    deltas.push(delta);
                }
            }
        }
        deltas
    }

    fn push_block(&mut self, block: ConversationBlockFacts) -> Vec<ConversationDeltaFacts> {
        let id = block_id(&block).to_string();
        self.block_index.insert(id, self.blocks.len());
        self.blocks.push(block.clone());
        vec![ConversationDeltaFacts::Append {
            block: Box::new(block),
        }]
    }

    fn upsert_block(&mut self, block: ConversationBlockFacts) -> Vec<ConversationDeltaFacts> {
        let id = block_id(&block).to_string();
        if let Some(index) = self.block_index.get(&id).copied() {
            if self.blocks[index] == block {
                return Vec::new();
            }
            self.blocks[index] = block.clone();
            return vec![ConversationDeltaFacts::Append {
                block: Box::new(block),
            }];
        }
        self.push_block(block)
    }

    fn complete_block(
        &mut self,
        block_id: &str,
        status: ConversationBlockStatus,
    ) -> Option<ConversationDeltaFacts> {
        if let Some(index) = self.block_index.get(block_id).copied() {
            if self.block_status(index) == Some(status) {
                return None;
            }
            self.set_status(index, status);
            return Some(ConversationDeltaFacts::Complete {
                block_id: block_id.to_string(),
                status,
            });
        }
        None
    }

    fn complete_streaming_block(
        &mut self,
        block_id: &str,
        status: ConversationBlockStatus,
    ) -> Option<ConversationDeltaFacts> {
        let index = self.block_index.get(block_id).copied()?;
        if self.block_status(index) != Some(ConversationBlockStatus::Streaming) {
            return None;
        }
        self.complete_block(block_id, status)
    }

    fn append_markdown(&mut self, index: usize, markdown: &str) {
        match &mut self.blocks[index] {
            ConversationBlockFacts::Thinking(block) => block.markdown.push_str(markdown),
            ConversationBlockFacts::Assistant(block) => block.markdown.push_str(markdown),
            _ => {},
        }
    }

    fn replace_markdown(&mut self, index: usize, markdown: &str) {
        match &mut self.blocks[index] {
            ConversationBlockFacts::Thinking(block) => block.markdown = markdown.to_string(),
            ConversationBlockFacts::Assistant(block) => block.markdown = markdown.to_string(),
            _ => {},
        }
    }

    fn append_tool_stream_content(&mut self, index: usize, stream: ToolOutputStream, chunk: &str) {
        if let ConversationBlockFacts::ToolCall(block) = &mut self.blocks[index] {
            match stream {
                ToolOutputStream::Stdout => block.streams.stdout.push_str(chunk),
                ToolOutputStream::Stderr => block.streams.stderr.push_str(chunk),
            }
        }
    }

    fn replace_tool_summary(&mut self, index: usize, summary: &str) -> bool {
        if let ConversationBlockFacts::ToolCall(block) = &mut self.blocks[index] {
            if block.summary.as_deref() == Some(summary) {
                return false;
            }
            block.summary = Some(summary.to_string());
            return true;
        }
        false
    }

    fn replace_tool_error(&mut self, index: usize, error: Option<&str>) -> bool {
        if let ConversationBlockFacts::ToolCall(block) = &mut self.blocks[index] {
            let new_error = error.map(ToString::to_string);
            if block.error == new_error {
                return false;
            }
            block.error = new_error;
            return true;
        }
        false
    }

    fn replace_tool_duration(&mut self, index: usize, duration_ms: u64) -> bool {
        if let ConversationBlockFacts::ToolCall(block) = &mut self.blocks[index] {
            if duration_ms == 0 && block.duration_ms.is_some() {
                return false;
            }
            if block.duration_ms == Some(duration_ms) {
                return false;
            }
            block.duration_ms = Some(duration_ms);
            return true;
        }
        false
    }

    fn replace_tool_truncated(&mut self, index: usize, truncated: bool) -> bool {
        if let ConversationBlockFacts::ToolCall(block) = &mut self.blocks[index] {
            if block.truncated == truncated {
                return false;
            }
            block.truncated = truncated;
            return true;
        }
        false
    }

    fn replace_tool_metadata(&mut self, index: usize, metadata: &Value) -> bool {
        if let ConversationBlockFacts::ToolCall(block) = &mut self.blocks[index] {
            if block.metadata.as_ref() == Some(metadata) {
                return false;
            }
            block.metadata = Some(metadata.clone());
            return true;
        }
        false
    }

    fn replace_tool_child_ref(&mut self, index: usize, child_ref: &ChildAgentRef) -> bool {
        if let ConversationBlockFacts::ToolCall(block) = &mut self.blocks[index] {
            if block.child_ref.as_ref() == Some(child_ref) {
                return false;
            }
            block.child_ref = Some(child_ref.clone());
            return true;
        }
        false
    }

    fn set_status(&mut self, index: usize, status: ConversationBlockStatus) {
        match &mut self.blocks[index] {
            ConversationBlockFacts::Thinking(block) => block.status = status,
            ConversationBlockFacts::Assistant(block) => block.status = status,
            ConversationBlockFacts::ToolCall(block) => block.status = status,
            _ => {},
        }
    }

    fn block_markdown(&self, index: usize) -> String {
        match &self.blocks[index] {
            ConversationBlockFacts::Thinking(block) => block.markdown.clone(),
            ConversationBlockFacts::Assistant(block) => block.markdown.clone(),
            _ => String::new(),
        }
    }

    fn block_status(&self, index: usize) -> Option<ConversationBlockStatus> {
        match &self.blocks[index] {
            ConversationBlockFacts::Thinking(block) => Some(block.status),
            ConversationBlockFacts::Assistant(block) => Some(block.status),
            ConversationBlockFacts::ToolCall(block) => Some(block.status),
            _ => None,
        }
    }
}

impl ConversationStreamProjector {
    pub(crate) fn new(
        last_sent_cursor: Option<String>,
        facts: &ConversationStreamReplayFacts,
    ) -> Self {
        let mut projector = ConversationDeltaProjector::new();
        projector.seed(&facts.seed_records);
        let step_progress = durable_step_progress_from_blocks(projector.blocks());
        Self {
            projector,
            last_sent_cursor,
            fallback_live_cursor: fallback_live_cursor(facts),
            step_progress,
        }
    }

    pub(crate) fn last_sent_cursor(&self) -> Option<&str> {
        self.last_sent_cursor.as_deref()
    }

    pub(crate) fn step_progress(&self) -> &ConversationStepProgressFacts {
        &self.step_progress
    }

    pub(crate) fn seed_initial_replay(
        &mut self,
        facts: &ConversationStreamReplayFacts,
    ) -> Vec<ConversationDeltaFrameFacts> {
        let frames = facts.replay_frames.clone();
        self.observe_durable_frames(&frames);
        frames
    }

    pub(crate) fn project_durable_record(
        &mut self,
        record: &SessionEventRecord,
    ) -> Vec<ConversationDeltaFrameFacts> {
        let deltas = self.projector.project_record(record);
        self.wrap_durable_deltas(record.event_id.as_str(), deltas)
    }

    pub(crate) fn project_live_event(
        &mut self,
        event: &AgentEvent,
    ) -> Vec<ConversationDeltaFrameFacts> {
        self.observe_live_event_step(event);
        let cursor = self.live_cursor();
        self.projector
            .project_live_event(event)
            .into_iter()
            .map(|delta| ConversationDeltaFrameFacts {
                cursor: cursor.clone(),
                step_progress: self.step_progress.clone(),
                delta,
            })
            .collect()
    }

    pub(crate) fn recover_from(
        &mut self,
        recovered: &ConversationStreamReplayFacts,
    ) -> Vec<ConversationDeltaFrameFacts> {
        self.fallback_live_cursor = fallback_live_cursor(recovered);
        let mut frames = Vec::new();
        for record in &recovered.replay_history {
            frames.extend(self.project_durable_record(record));
        }
        frames
    }

    fn wrap_durable_deltas(
        &mut self,
        cursor: &str,
        deltas: Vec<ConversationDeltaFacts>,
    ) -> Vec<ConversationDeltaFrameFacts> {
        if deltas.is_empty() {
            return Vec::new();
        }
        let cursor_owned = cursor.to_string();
        self.last_sent_cursor = Some(cursor_owned.clone());
        deltas
            .into_iter()
            .map(|delta| {
                self.observe_durable_delta_step(&delta);
                ConversationDeltaFrameFacts {
                    cursor: cursor_owned.clone(),
                    step_progress: self.step_progress.clone(),
                    delta,
                }
            })
            .collect()
    }

    fn observe_durable_frames(&mut self, frames: &[ConversationDeltaFrameFacts]) {
        if let Some(cursor) = frames.last().map(|frame| frame.cursor.clone()) {
            self.last_sent_cursor = Some(cursor);
        }
        if let Some(step_progress) = frames.last().map(|frame| frame.step_progress.clone()) {
            self.step_progress = step_progress;
        }
    }

    fn live_cursor(&self) -> String {
        self.last_sent_cursor
            .clone()
            .or_else(|| self.fallback_live_cursor.clone())
            .unwrap_or_else(|| "0.0".to_string())
    }

    fn observe_durable_delta_step(&mut self, delta: &ConversationDeltaFacts) {
        observe_durable_delta_step(&mut self.step_progress, delta);
    }

    fn observe_live_event_step(&mut self, event: &AgentEvent) {
        let turn_id = match event {
            AgentEvent::ThinkingDelta { turn_id, .. }
            | AgentEvent::ModelDelta { turn_id, .. }
            | AgentEvent::StreamRetryStarted { turn_id, .. }
            | AgentEvent::ToolCallStart { turn_id, .. }
            | AgentEvent::ToolCallDelta { turn_id, .. }
            | AgentEvent::ToolCallResult { turn_id, .. } => Some(turn_id.as_str()),
            AgentEvent::TurnDone { turn_id, .. } => {
                if self
                    .step_progress
                    .live
                    .as_ref()
                    .is_some_and(|cursor| cursor.turn_id == *turn_id)
                {
                    self.step_progress.live = None;
                }
                None
            },
            _ => None,
        };
        let Some(turn_id) = turn_id else {
            return;
        };

        let step_index = self
            .step_progress
            .durable
            .as_ref()
            .filter(|cursor| cursor.turn_id == turn_id)
            .map(|cursor| cursor.step_index.saturating_add(1))
            .unwrap_or(0);
        let next_live = ConversationStepCursorFacts {
            turn_id: turn_id.to_string(),
            step_index,
        };
        if self.step_progress.durable.as_ref().is_some_and(|cursor| {
            cursor.turn_id == next_live.turn_id && cursor.step_index >= next_live.step_index
        }) {
            return;
        }
        if self.step_progress.live.as_ref() == Some(&next_live) {
            return;
        }
        self.step_progress.live = Some(next_live);
    }
}

pub(crate) fn project_conversation_snapshot(
    records: &[SessionEventRecord],
    phase: Phase,
) -> ConversationSnapshotFacts {
    let mut projector = ConversationDeltaProjector::new();
    projector.seed(records);
    let blocks = suppress_draft_approval_plan_leakage(projector.into_blocks());
    ConversationSnapshotFacts {
        cursor: records.last().map(|record| record.event_id.clone()),
        phase,
        step_progress: durable_step_progress_from_blocks(&blocks),
        blocks,
    }
}

pub(crate) fn build_conversation_replay_frames(
    seed_records: &[SessionEventRecord],
    history: &[SessionEventRecord],
) -> Vec<ConversationDeltaFrameFacts> {
    let mut projector = ConversationDeltaProjector::new();
    projector.seed(seed_records);
    let mut step_progress = durable_step_progress_from_blocks(projector.blocks());
    let mut raw_frames = Vec::new();
    for record in history {
        raw_frames.extend(
            projector
                .project_record(record)
                .into_iter()
                .map(|delta| (record.event_id.clone(), delta)),
        );
    }
    let hidden_block_ids = draft_approval_leakage_hidden_block_ids(projector.blocks());

    let mut frames = Vec::new();
    for (cursor, delta) in raw_frames {
        if delta_block_id(&delta).is_some_and(|block_id| hidden_block_ids.contains(block_id)) {
            continue;
        }
        observe_durable_delta_step(&mut step_progress, &delta);
        frames.push(ConversationDeltaFrameFacts {
            cursor,
            step_progress: step_progress.clone(),
            delta,
        });
    }
    frames
}

fn suppress_draft_approval_plan_leakage(
    blocks: Vec<ConversationBlockFacts>,
) -> Vec<ConversationBlockFacts> {
    let hidden_block_ids = draft_approval_leakage_hidden_block_ids(&blocks);
    blocks
        .into_iter()
        .filter(|block| !hidden_block_ids.contains(block_id(block)))
        .collect()
}

fn draft_approval_leakage_hidden_block_ids(blocks: &[ConversationBlockFacts]) -> HashSet<String> {
    let mut turn_facts = HashMap::<String, (bool, bool)>::new();
    for block in blocks {
        match block {
            ConversationBlockFacts::User(block) => {
                let Some(turn_id) = block.turn_id.as_deref() else {
                    continue;
                };
                let facts = turn_facts
                    .entry(turn_id.to_string())
                    .or_insert((false, false));
                if is_approval_like_turn_text(&block.markdown) {
                    facts.0 = true;
                }
            },
            ConversationBlockFacts::Plan(block) => {
                let Some(turn_id) = block.turn_id.as_deref() else {
                    continue;
                };
                let facts = turn_facts
                    .entry(turn_id.to_string())
                    .or_insert((false, false));
                if block.status.as_deref() == Some("awaiting_approval")
                    || matches!(
                        block.event_kind,
                        ConversationPlanEventKind::Presented
                            | ConversationPlanEventKind::ReviewPending
                    )
                {
                    facts.1 = true;
                }
            },
            _ => {},
        }
    }

    blocks
        .iter()
        .filter_map(|block| {
            let turn_id = turn_id(block)?;
            let (approval_like_user, has_review_plan) = turn_facts.get(turn_id).copied()?;
            if !approval_like_user || !has_review_plan {
                return None;
            }
            matches!(
                block,
                ConversationBlockFacts::Assistant(_) | ConversationBlockFacts::Thinking(_)
            )
            .then(|| block_id(block).to_string())
        })
        .collect()
}

fn delta_block_id(delta: &ConversationDeltaFacts) -> Option<&str> {
    match delta {
        ConversationDeltaFacts::Append { block } => Some(block_id(block.as_ref())),
        ConversationDeltaFacts::Patch { block_id, .. }
        | ConversationDeltaFacts::Complete { block_id, .. } => Some(block_id.as_str()),
    }
}

fn turn_id(block: &ConversationBlockFacts) -> Option<&str> {
    match block {
        ConversationBlockFacts::User(block) => block.turn_id.as_deref(),
        ConversationBlockFacts::Assistant(block) => block.turn_id.as_deref(),
        ConversationBlockFacts::Thinking(block) => block.turn_id.as_deref(),
        ConversationBlockFacts::PromptMetrics(block) => block.turn_id.as_deref(),
        ConversationBlockFacts::Plan(block) => block.turn_id.as_deref(),
        ConversationBlockFacts::ToolCall(block) => block.turn_id.as_deref(),
        ConversationBlockFacts::Error(block) => block.turn_id.as_deref(),
        ConversationBlockFacts::SystemNote(_) | ConversationBlockFacts::ChildHandoff(_) => None,
    }
}

fn is_approval_like_turn_text(text: &str) -> bool {
    let normalized_english = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    for phrase in ["approved", "go ahead", "implement it"] {
        if normalized_english == phrase
            || (phrase != "implement it" && normalized_english.starts_with(&format!("{phrase} ")))
        {
            return true;
        }
    }

    let normalized_chinese = text
        .chars()
        .filter(|ch| {
            !ch.is_whitespace()
                && !matches!(
                    ch,
                    ',' | '.'
                        | '!'
                        | '?'
                        | ';'
                        | ':'
                        | '，'
                        | '。'
                        | '！'
                        | '？'
                        | '；'
                        | '：'
                        | '【'
                        | '】'
                        | '、'
                )
        })
        .collect::<String>();
    for phrase in ["同意", "可以", "按这个做", "开始实现"] {
        let matched = if matches!(phrase, "同意" | "可以") {
            normalized_chinese == phrase
        } else {
            normalized_chinese == phrase || normalized_chinese.starts_with(phrase)
        };
        if matched {
            return true;
        }
    }

    false
}

fn fallback_live_cursor(facts: &ConversationStreamReplayFacts) -> Option<String> {
    facts
        .seed_records
        .last()
        .map(|record| record.event_id.clone())
        .or_else(|| {
            facts
                .replay_history
                .last()
                .map(|record| record.event_id.clone())
        })
}

fn block_id(block: &ConversationBlockFacts) -> &str {
    match block {
        ConversationBlockFacts::User(block) => &block.id,
        ConversationBlockFacts::Assistant(block) => &block.id,
        ConversationBlockFacts::Thinking(block) => &block.id,
        ConversationBlockFacts::PromptMetrics(block) => &block.id,
        ConversationBlockFacts::Plan(block) => &block.id,
        ConversationBlockFacts::ToolCall(block) => &block.id,
        ConversationBlockFacts::Error(block) => &block.id,
        ConversationBlockFacts::SystemNote(block) => &block.id,
        ConversationBlockFacts::ChildHandoff(block) => &block.id,
    }
}

fn durable_step_progress_from_blocks(
    blocks: &[ConversationBlockFacts],
) -> ConversationStepProgressFacts {
    let mut step_progress = ConversationStepProgressFacts::default();
    for block in blocks {
        observe_durable_block_step(&mut step_progress, block);
    }
    step_progress
}

fn observe_durable_delta_step(
    step_progress: &mut ConversationStepProgressFacts,
    delta: &ConversationDeltaFacts,
) {
    if let ConversationDeltaFacts::Append { block } = delta {
        observe_durable_block_step(step_progress, block.as_ref());
    }
}

fn observe_durable_block_step(
    step_progress: &mut ConversationStepProgressFacts,
    block: &ConversationBlockFacts,
) {
    let step_cursor = match block {
        ConversationBlockFacts::PromptMetrics(block) => Some(ConversationStepCursorFacts {
            turn_id: block
                .turn_id
                .clone()
                .unwrap_or_else(|| "session".to_string()),
            step_index: block.step_index,
        }),
        ConversationBlockFacts::Assistant(block) => {
            block
                .step_index
                .map(|step_index| ConversationStepCursorFacts {
                    turn_id: block
                        .turn_id
                        .clone()
                        .unwrap_or_else(|| "session".to_string()),
                    step_index,
                })
        },
        _ => None,
    };

    if let Some(step_cursor) = step_cursor {
        step_progress.durable = Some(step_cursor.clone());
        if let Some(live) = step_progress.live.as_ref() {
            if live.turn_id != step_cursor.turn_id || live.step_index <= step_cursor.step_index {
                step_progress.live = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::AgentEventContext;

    use super::*;

    #[test]
    fn stream_retry_reset_clears_live_markdown_blocks_without_duplicates() {
        let mut projector = ConversationDeltaProjector::new();

        projector.project_live_event(&AgentEvent::ThinkingDelta {
            turn_id: "turn-1".to_string(),
            agent: AgentEventContext::default(),
            delta: "old thought".to_string(),
        });
        projector.project_live_event(&AgentEvent::ModelDelta {
            turn_id: "turn-1".to_string(),
            agent: AgentEventContext::default(),
            delta: "old answer".to_string(),
        });

        let reset = projector.project_live_event(&AgentEvent::StreamRetryStarted {
            turn_id: "turn-1".to_string(),
            agent: AgentEventContext::default(),
            attempt: 2,
            max_attempts: 2,
            reason: "bad stream".to_string(),
        });
        assert_eq!(
            reset,
            vec![
                ConversationDeltaFacts::Patch {
                    block_id: "turn:turn-1:thinking".to_string(),
                    patch: ConversationBlockPatchFacts::ReplaceMarkdown {
                        markdown: String::new(),
                    },
                },
                ConversationDeltaFacts::Patch {
                    block_id: "turn:turn-1:assistant".to_string(),
                    patch: ConversationBlockPatchFacts::ReplaceMarkdown {
                        markdown: String::new(),
                    },
                },
            ]
        );

        let retry_delta = projector.project_live_event(&AgentEvent::ModelDelta {
            turn_id: "turn-1".to_string(),
            agent: AgentEventContext::default(),
            delta: "new answer".to_string(),
        });
        assert_eq!(
            retry_delta,
            vec![ConversationDeltaFacts::Patch {
                block_id: "turn:turn-1:assistant".to_string(),
                patch: ConversationBlockPatchFacts::AppendMarkdown {
                    markdown: "new answer".to_string(),
                },
            }]
        );
    }
}
