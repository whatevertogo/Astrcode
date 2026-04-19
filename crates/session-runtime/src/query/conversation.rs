//! authoritative conversation / tool display 读模型。
//!
//! Why: 工具展示的聚合语义属于单 session query 真相，不应该继续滞留在
//! `server` route/projector 或前端 regroup 逻辑里。

use std::collections::HashMap;

use astrcode_core::{
    AgentEvent, ChildAgentRef, ChildSessionNotification, ChildSessionNotificationKind,
    CompactAppliedMeta, CompactTrigger, Phase, SessionEventRecord, ToolExecutionResult,
    ToolOutputStream,
};
use serde_json::Value;

use crate::SessionReplay;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationBlockStatus {
    Streaming,
    Complete,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationSystemNoteKind {
    Compact,
    SystemNote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationChildHandoffKind {
    Delegated,
    Progress,
    Returned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationTranscriptErrorKind {
    ProviderError,
    ContextWindowExceeded,
    ToolFatal,
    RateLimit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationPlanEventKind {
    Saved,
    ReviewPending,
    Presented,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationPlanReviewKind {
    RevisePlan,
    FinalReview,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ToolCallStreamsFacts {
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationUserBlockFacts {
    pub id: String,
    pub turn_id: Option<String>,
    pub markdown: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationAssistantBlockFacts {
    pub id: String,
    pub turn_id: Option<String>,
    pub status: ConversationBlockStatus,
    pub markdown: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationThinkingBlockFacts {
    pub id: String,
    pub turn_id: Option<String>,
    pub status: ConversationBlockStatus,
    pub markdown: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationPlanReviewFacts {
    pub kind: ConversationPlanReviewKind,
    pub checklist: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConversationPlanBlockersFacts {
    pub missing_headings: Vec<String>,
    pub invalid_sections: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationPlanBlockFacts {
    pub id: String,
    pub turn_id: Option<String>,
    pub tool_call_id: String,
    pub event_kind: ConversationPlanEventKind,
    pub title: String,
    pub plan_path: String,
    pub summary: Option<String>,
    pub status: Option<String>,
    pub slug: Option<String>,
    pub updated_at: Option<String>,
    pub content: Option<String>,
    pub review: Option<ConversationPlanReviewFacts>,
    pub blockers: ConversationPlanBlockersFacts,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCallBlockFacts {
    pub id: String,
    pub turn_id: Option<String>,
    pub tool_call_id: String,
    pub tool_name: String,
    pub status: ConversationBlockStatus,
    pub input: Option<Value>,
    pub summary: Option<String>,
    pub error: Option<String>,
    pub duration_ms: Option<u64>,
    pub truncated: bool,
    pub metadata: Option<Value>,
    pub child_ref: Option<ChildAgentRef>,
    pub streams: ToolCallStreamsFacts,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationErrorBlockFacts {
    pub id: String,
    pub turn_id: Option<String>,
    pub code: ConversationTranscriptErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationSystemNoteBlockFacts {
    pub id: String,
    pub note_kind: ConversationSystemNoteKind,
    pub markdown: String,
    pub compact_trigger: Option<CompactTrigger>,
    pub compact_meta: Option<CompactAppliedMeta>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationChildHandoffBlockFacts {
    pub id: String,
    pub handoff_kind: ConversationChildHandoffKind,
    pub child_ref: ChildAgentRef,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConversationBlockFacts {
    User(ConversationUserBlockFacts),
    Assistant(ConversationAssistantBlockFacts),
    Thinking(ConversationThinkingBlockFacts),
    Plan(Box<ConversationPlanBlockFacts>),
    ToolCall(Box<ToolCallBlockFacts>),
    Error(ConversationErrorBlockFacts),
    SystemNote(ConversationSystemNoteBlockFacts),
    ChildHandoff(ConversationChildHandoffBlockFacts),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConversationBlockPatchFacts {
    AppendMarkdown {
        markdown: String,
    },
    ReplaceMarkdown {
        markdown: String,
    },
    AppendToolStream {
        stream: ToolOutputStream,
        chunk: String,
    },
    ReplaceSummary {
        summary: String,
    },
    ReplaceMetadata {
        metadata: Value,
    },
    ReplaceError {
        error: Option<String>,
    },
    ReplaceDuration {
        duration_ms: u64,
    },
    ReplaceChildRef {
        child_ref: ChildAgentRef,
    },
    SetTruncated {
        truncated: bool,
    },
    SetStatus {
        status: ConversationBlockStatus,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConversationDeltaFacts {
    AppendBlock {
        block: Box<ConversationBlockFacts>,
    },
    PatchBlock {
        block_id: String,
        patch: ConversationBlockPatchFacts,
    },
    CompleteBlock {
        block_id: String,
        status: ConversationBlockStatus,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConversationDeltaFrameFacts {
    pub cursor: String,
    pub delta: ConversationDeltaFacts,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConversationSnapshotFacts {
    pub cursor: Option<String>,
    pub phase: Phase,
    pub blocks: Vec<ConversationBlockFacts>,
}

#[derive(Debug)]
pub struct ConversationStreamReplayFacts {
    pub cursor: Option<String>,
    pub phase: Phase,
    pub seed_records: Vec<SessionEventRecord>,
    pub replay_frames: Vec<ConversationDeltaFrameFacts>,
    pub replay: SessionReplay,
}

#[derive(Default)]
pub struct ConversationDeltaProjector {
    blocks: Vec<ConversationBlockFacts>,
    block_index: HashMap<String, usize>,
    turn_blocks: HashMap<String, TurnBlockRefs>,
    tool_blocks: HashMap<String, ToolBlockRefs>,
}

#[derive(Default)]
pub struct ConversationStreamProjector {
    projector: ConversationDeltaProjector,
    last_sent_cursor: Option<String>,
    fallback_live_cursor: Option<String>,
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

impl ConversationDeltaProjector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn seed(&mut self, history: &[SessionEventRecord]) {
        for record in history {
            let _ = self.project_record(record);
        }
    }

    pub fn blocks(&self) -> &[ConversationBlockFacts] {
        &self.blocks
    }

    pub fn into_blocks(self) -> Vec<ConversationBlockFacts> {
        self.blocks
    }

    pub fn project_record(&mut self, record: &SessionEventRecord) -> Vec<ConversationDeltaFacts> {
        self.project_event(
            &record.event,
            ProjectionSource::Durable,
            Some(record.event_id.as_str()),
        )
    }

    pub fn project_live_event(&mut self, event: &AgentEvent) -> Vec<ConversationDeltaFacts> {
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
            AgentEvent::AssistantMessage {
                turn_id,
                content,
                reasoning_content,
                ..
            } if source.is_durable() => {
                self.finalize_assistant_block(turn_id, content, reasoning_content.as_deref())
            },
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
            } => self.append_tool_stream(turn_id, tool_call_id, tool_name, *stream, delta, source),
            AgentEvent::ToolCallResult {
                turn_id, result, ..
            } => {
                if let Some(block) = plan_block_from_tool_result(turn_id, result) {
                    self.push_block(ConversationBlockFacts::Plan(Box::new(block)))
                } else {
                    self.complete_tool_call(turn_id, result, source)
                }
            },
            AgentEvent::CompactApplied {
                turn_id,
                trigger,
                summary,
                meta,
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
            | AgentEvent::PromptMetrics { .. }
            | AgentEvent::SubRunStarted { .. }
            | AgentEvent::SubRunFinished { .. }
            | AgentEvent::AgentInputQueued { .. }
            | AgentEvent::AgentInputBatchStarted { .. }
            | AgentEvent::AgentInputBatchAcked { .. }
            | AgentEvent::AgentInputDiscarded { .. }
            | AgentEvent::UserMessage { .. }
            | AgentEvent::AssistantMessage { .. }
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
            return vec![ConversationDeltaFacts::PatchBlock {
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
                })
            },
        };
        self.push_block(block)
    }

    fn finalize_assistant_block(
        &mut self,
        turn_id: &str,
        content: &str,
        reasoning_content: Option<&str>,
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
                return vec![ConversationDeltaFacts::PatchBlock {
                    block_id: block_id.to_string(),
                    patch: ConversationBlockPatchFacts::AppendMarkdown {
                        markdown: suffix.to_string(),
                    },
                }];
            }
            return vec![ConversationDeltaFacts::PatchBlock {
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
                })
            },
        };
        self.push_block(block)
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
        deltas.push(ConversationDeltaFacts::PatchBlock {
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
                deltas.push(ConversationDeltaFacts::PatchBlock {
                    block_id: call_block_id.clone(),
                    patch: ConversationBlockPatchFacts::ReplaceSummary {
                        summary: summary.clone(),
                    },
                });
            }
            if self.replace_tool_error(index, result.error.as_deref()) {
                deltas.push(ConversationDeltaFacts::PatchBlock {
                    block_id: call_block_id.clone(),
                    patch: ConversationBlockPatchFacts::ReplaceError {
                        error: result.error.clone(),
                    },
                });
            }
            if self.replace_tool_duration(index, result.duration_ms) {
                deltas.push(ConversationDeltaFacts::PatchBlock {
                    block_id: call_block_id.clone(),
                    patch: ConversationBlockPatchFacts::ReplaceDuration {
                        duration_ms: result.duration_ms,
                    },
                });
            }
            if self.replace_tool_truncated(index, result.truncated) {
                deltas.push(ConversationDeltaFacts::PatchBlock {
                    block_id: call_block_id.clone(),
                    patch: ConversationBlockPatchFacts::SetTruncated {
                        truncated: result.truncated,
                    },
                });
            }
            if let Some(metadata) = &result.metadata {
                if self.replace_tool_metadata(index, metadata) {
                    deltas.push(ConversationDeltaFacts::PatchBlock {
                        block_id: call_block_id.clone(),
                        patch: ConversationBlockPatchFacts::ReplaceMetadata {
                            metadata: metadata.clone(),
                        },
                    });
                }
            }
            if let Some(child_ref) = &result.child_ref {
                if self.replace_tool_child_ref(index, child_ref) {
                    deltas.push(ConversationDeltaFacts::PatchBlock {
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
                        deltas.push(ConversationDeltaFacts::PatchBlock {
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
        code: &str,
        message: &str,
    ) -> Vec<ConversationDeltaFacts> {
        if code == "interrupted" {
            return Vec::new();
        }
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
        vec![ConversationDeltaFacts::AppendBlock {
            block: Box::new(block),
        }]
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
            return Some(ConversationDeltaFacts::CompleteBlock {
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
    pub fn new(last_sent_cursor: Option<String>, facts: &ConversationStreamReplayFacts) -> Self {
        let mut projector = ConversationDeltaProjector::new();
        projector.seed(&facts.seed_records);
        Self {
            projector,
            last_sent_cursor,
            fallback_live_cursor: fallback_live_cursor(facts),
        }
    }

    pub fn last_sent_cursor(&self) -> Option<&str> {
        self.last_sent_cursor.as_deref()
    }

    pub fn seed_initial_replay(
        &mut self,
        facts: &ConversationStreamReplayFacts,
    ) -> Vec<ConversationDeltaFrameFacts> {
        let frames = facts.replay_frames.clone();
        self.observe_durable_frames(&frames);
        frames
    }

    pub fn project_durable_record(
        &mut self,
        record: &SessionEventRecord,
    ) -> Vec<ConversationDeltaFrameFacts> {
        let deltas = self.projector.project_record(record);
        self.wrap_durable_deltas(record.event_id.as_str(), deltas)
    }

    pub fn project_live_event(&mut self, event: &AgentEvent) -> Vec<ConversationDeltaFrameFacts> {
        let cursor = self.live_cursor();
        self.projector
            .project_live_event(event)
            .into_iter()
            .map(|delta| ConversationDeltaFrameFacts {
                cursor: cursor.clone(),
                delta,
            })
            .collect()
    }

    pub fn recover_from(
        &mut self,
        recovered: &ConversationStreamReplayFacts,
    ) -> Vec<ConversationDeltaFrameFacts> {
        self.fallback_live_cursor = fallback_live_cursor(recovered);
        let mut frames = Vec::new();
        for record in &recovered.replay.history {
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
            .map(|delta| ConversationDeltaFrameFacts {
                cursor: cursor_owned.clone(),
                delta,
            })
            .collect()
    }

    fn observe_durable_frames(&mut self, frames: &[ConversationDeltaFrameFacts]) {
        if let Some(cursor) = frames.last().map(|frame| frame.cursor.clone()) {
            self.last_sent_cursor = Some(cursor);
        }
    }

    fn live_cursor(&self) -> String {
        self.last_sent_cursor
            .clone()
            .or_else(|| self.fallback_live_cursor.clone())
            .unwrap_or_else(|| "0.0".to_string())
    }
}

pub(crate) fn project_conversation_snapshot(
    records: &[SessionEventRecord],
    phase: Phase,
) -> ConversationSnapshotFacts {
    let mut projector = ConversationDeltaProjector::new();
    projector.seed(records);
    ConversationSnapshotFacts {
        cursor: records.last().map(|record| record.event_id.clone()),
        phase,
        blocks: projector.into_blocks(),
    }
}

pub(crate) fn build_conversation_replay_frames(
    seed_records: &[SessionEventRecord],
    history: &[SessionEventRecord],
) -> Vec<ConversationDeltaFrameFacts> {
    let mut projector = ConversationDeltaProjector::new();
    projector.seed(seed_records);
    let mut frames = Vec::new();
    for record in history {
        for delta in projector.project_record(record) {
            frames.push(ConversationDeltaFrameFacts {
                cursor: record.event_id.clone(),
                delta,
            });
        }
    }
    frames
}

pub(crate) fn fallback_live_cursor(facts: &ConversationStreamReplayFacts) -> Option<String> {
    facts
        .seed_records
        .last()
        .map(|record| record.event_id.clone())
        .or_else(|| {
            facts
                .replay
                .history
                .last()
                .map(|record| record.event_id.clone())
        })
}

fn block_id(block: &ConversationBlockFacts) -> &str {
    match block {
        ConversationBlockFacts::User(block) => &block.id,
        ConversationBlockFacts::Assistant(block) => &block.id,
        ConversationBlockFacts::Thinking(block) => &block.id,
        ConversationBlockFacts::Plan(block) => &block.id,
        ConversationBlockFacts::ToolCall(block) => &block.id,
        ConversationBlockFacts::Error(block) => &block.id,
        ConversationBlockFacts::SystemNote(block) => &block.id,
        ConversationBlockFacts::ChildHandoff(block) => &block.id,
    }
}

fn should_suppress_tool_call_block(tool_name: &str, _input: Option<&Value>) -> bool {
    matches!(tool_name, "upsertSessionPlan" | "exitPlanMode")
}

fn plan_block_from_tool_result(
    turn_id: &str,
    result: &ToolExecutionResult,
) -> Option<ConversationPlanBlockFacts> {
    if !result.ok {
        return None;
    }

    let metadata = result.metadata.as_ref()?.as_object()?;
    match result.tool_name.as_str() {
        "upsertSessionPlan" => {
            let title = json_string(metadata, "title")?;
            let plan_path = json_string(metadata, "planPath")?;
            Some(ConversationPlanBlockFacts {
                id: format!("plan:{}:saved", result.tool_call_id),
                turn_id: Some(turn_id.to_string()),
                tool_call_id: result.tool_call_id.clone(),
                event_kind: ConversationPlanEventKind::Saved,
                title,
                plan_path,
                summary: Some(tool_result_summary(result)),
                status: json_string(metadata, "status"),
                slug: json_string(metadata, "slug"),
                updated_at: json_string(metadata, "updatedAt"),
                content: None,
                review: None,
                blockers: ConversationPlanBlockersFacts::default(),
            })
        },
        "exitPlanMode" => match json_string(metadata, "schema").as_deref() {
            Some("sessionPlanExit") => plan_presented_block(turn_id, result, metadata),
            Some("sessionPlanExitReviewPending") | Some("sessionPlanExitBlocked") => {
                plan_review_pending_block(turn_id, result, metadata)
            },
            _ => None,
        },
        _ => None,
    }
}

fn plan_presented_block(
    turn_id: &str,
    result: &ToolExecutionResult,
    metadata: &serde_json::Map<String, Value>,
) -> Option<ConversationPlanBlockFacts> {
    let plan = metadata.get("plan")?.as_object()?;
    Some(ConversationPlanBlockFacts {
        id: format!("plan:{}:presented", result.tool_call_id),
        turn_id: Some(turn_id.to_string()),
        tool_call_id: result.tool_call_id.clone(),
        event_kind: ConversationPlanEventKind::Presented,
        title: json_string(plan, "title")?,
        plan_path: json_string(plan, "planPath")?,
        summary: Some("计划已呈递".to_string()),
        status: json_string(plan, "status"),
        slug: json_string(plan, "slug"),
        updated_at: json_string(plan, "updatedAt"),
        content: json_string(plan, "content"),
        review: None,
        blockers: ConversationPlanBlockersFacts::default(),
    })
}

fn plan_review_pending_block(
    turn_id: &str,
    result: &ToolExecutionResult,
    metadata: &serde_json::Map<String, Value>,
) -> Option<ConversationPlanBlockFacts> {
    let plan = metadata.get("plan")?.as_object()?;
    let review = metadata
        .get("review")
        .and_then(Value::as_object)
        .and_then(|review| {
            let kind = match json_string(review, "kind").as_deref() {
                Some("revise_plan") => ConversationPlanReviewKind::RevisePlan,
                Some("final_review") => ConversationPlanReviewKind::FinalReview,
                _ => return None,
            };
            Some(ConversationPlanReviewFacts {
                kind,
                checklist: json_string_array(review, "checklist"),
            })
        });
    let blockers = metadata
        .get("blockers")
        .and_then(Value::as_object)
        .map(|blockers| ConversationPlanBlockersFacts {
            missing_headings: json_string_array(blockers, "missingHeadings"),
            invalid_sections: json_string_array(blockers, "invalidSections"),
        })
        .unwrap_or_default();

    Some(ConversationPlanBlockFacts {
        id: format!("plan:{}:review-pending", result.tool_call_id),
        turn_id: Some(turn_id.to_string()),
        tool_call_id: result.tool_call_id.clone(),
        event_kind: ConversationPlanEventKind::ReviewPending,
        title: json_string(plan, "title")?,
        plan_path: json_string(plan, "planPath")?,
        summary: Some(match review.as_ref().map(|review| review.kind) {
            Some(ConversationPlanReviewKind::RevisePlan) => "正在修计划".to_string(),
            Some(ConversationPlanReviewKind::FinalReview) => "正在做退出前自审".to_string(),
            None => "继续完善中".to_string(),
        }),
        status: None,
        slug: None,
        updated_at: None,
        content: None,
        review,
        blockers,
    })
}

fn json_string(container: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    container
        .get(key)
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn json_string_array(container: &serde_json::Map<String, Value>, key: &str) -> Vec<String> {
    container
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn tool_result_summary(result: &ToolExecutionResult) -> String {
    const MAX_SUMMARY_CHARS: usize = 120;

    if result.ok {
        if !result.output.trim().is_empty() {
            crate::query::text::summarize_inline_text(&result.output, MAX_SUMMARY_CHARS)
                .unwrap_or_else(|| format!("{} completed", result.tool_name))
        } else {
            format!("{} completed", result.tool_name)
        }
    } else if let Some(error) = &result.error {
        crate::query::text::summarize_inline_text(error, MAX_SUMMARY_CHARS)
            .unwrap_or_else(|| format!("{} failed", result.tool_name))
    } else if !result.output.trim().is_empty() {
        crate::query::text::summarize_inline_text(&result.output, MAX_SUMMARY_CHARS)
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

#[cfg(test)]
mod tests {
    use std::{path::Path, sync::Arc};

    use astrcode_core::{
        AgentEvent, AgentEventContext, AgentLifecycleStatus, ChildAgentRef, ChildExecutionIdentity,
        ChildSessionLineageKind, ChildSessionNotification, ChildSessionNotificationKind,
        DeleteProjectResult, EventStore, ParentDelivery, ParentDeliveryOrigin,
        ParentDeliveryPayload, ParentDeliveryTerminalSemantics, ParentExecutionRef, Phase,
        SessionEventRecord, SessionId, SessionMeta, SessionTurnAcquireResult, StorageEvent,
        StorageEventPayload, StoredEvent, ToolExecutionResult, ToolOutputStream, UserMessageOrigin,
    };
    use async_trait::async_trait;
    use chrono::Utc;
    use serde_json::json;
    use tokio::sync::broadcast;

    use super::{
        ConversationBlockFacts, ConversationBlockPatchFacts, ConversationBlockStatus,
        ConversationChildHandoffKind, ConversationDeltaFacts, ConversationDeltaProjector,
        ConversationPlanEventKind, ConversationStreamProjector, ConversationStreamReplayFacts,
        build_conversation_replay_frames, fallback_live_cursor, project_conversation_snapshot,
    };
    use crate::{
        SessionReplay, SessionRuntime,
        turn::test_support::{NoopMetrics, NoopPromptFactsProvider, test_kernel},
    };

    #[test]
    fn snapshot_projects_tool_call_block_with_streams_and_terminal_fields() {
        let records = vec![
            record(
                "1.1",
                AgentEvent::ToolCallStart {
                    turn_id: "turn-1".to_string(),
                    agent: sample_agent_context(),
                    tool_call_id: "call-1".to_string(),
                    tool_name: "shell_command".to_string(),
                    input: json!({ "command": "pwd" }),
                },
            ),
            record(
                "1.2",
                AgentEvent::ToolCallDelta {
                    turn_id: "turn-1".to_string(),
                    agent: sample_agent_context(),
                    tool_call_id: "call-1".to_string(),
                    tool_name: "shell_command".to_string(),
                    stream: ToolOutputStream::Stdout,
                    delta: "line-1\n".to_string(),
                },
            ),
            record(
                "1.3",
                AgentEvent::ToolCallDelta {
                    turn_id: "turn-1".to_string(),
                    agent: sample_agent_context(),
                    tool_call_id: "call-1".to_string(),
                    tool_name: "shell_command".to_string(),
                    stream: ToolOutputStream::Stderr,
                    delta: "warn\n".to_string(),
                },
            ),
            record(
                "1.4",
                AgentEvent::ToolCallResult {
                    turn_id: "turn-1".to_string(),
                    agent: sample_agent_context(),
                    result: ToolExecutionResult {
                        tool_call_id: "call-1".to_string(),
                        tool_name: "shell_command".to_string(),
                        ok: false,
                        output: "line-1\n".to_string(),
                        error: Some("permission denied".to_string()),
                        metadata: Some(json!({ "path": "/tmp", "truncated": true })),
                        child_ref: None,
                        duration_ms: 42,
                        truncated: true,
                    },
                },
            ),
        ];

        let snapshot = project_conversation_snapshot(&records, Phase::CallingTool);
        let tool = snapshot
            .blocks
            .iter()
            .find_map(|block| match block {
                ConversationBlockFacts::ToolCall(block) => Some(block),
                _ => None,
            })
            .expect("tool block should exist");

        assert_eq!(tool.tool_call_id, "call-1");
        assert_eq!(tool.status, ConversationBlockStatus::Failed);
        assert_eq!(tool.streams.stdout, "line-1\n");
        assert_eq!(tool.streams.stderr, "warn\n");
        assert_eq!(tool.error.as_deref(), Some("permission denied"));
        assert_eq!(tool.duration_ms, Some(42));
        assert!(tool.truncated);
    }

    #[test]
    fn snapshot_preserves_failed_tool_status_after_turn_done() {
        let records = vec![
            record(
                "1.1",
                AgentEvent::ToolCallStart {
                    turn_id: "turn-1".to_string(),
                    agent: sample_agent_context(),
                    tool_call_id: "call-1".to_string(),
                    tool_name: "shell_command".to_string(),
                    input: json!({ "command": "missing-command" }),
                },
            ),
            record(
                "1.2",
                AgentEvent::ToolCallResult {
                    turn_id: "turn-1".to_string(),
                    agent: sample_agent_context(),
                    result: ToolExecutionResult {
                        tool_call_id: "call-1".to_string(),
                        tool_name: "shell_command".to_string(),
                        ok: false,
                        output: String::new(),
                        error: Some("command not found".to_string()),
                        metadata: None,
                        child_ref: None,
                        duration_ms: 127,
                        truncated: false,
                    },
                },
            ),
            record(
                "1.3",
                AgentEvent::TurnDone {
                    turn_id: "turn-1".to_string(),
                    agent: sample_agent_context(),
                },
            ),
        ];

        let snapshot = project_conversation_snapshot(&records, Phase::Idle);
        let tool = snapshot
            .blocks
            .iter()
            .find_map(|block| match block {
                ConversationBlockFacts::ToolCall(block) => Some(block),
                _ => None,
            })
            .expect("tool block should exist");

        assert_eq!(tool.status, ConversationBlockStatus::Failed);
        assert_eq!(tool.error.as_deref(), Some("command not found"));
        assert_eq!(tool.duration_ms, Some(127));
    }

    #[test]
    fn snapshot_projects_plan_blocks_in_durable_event_order() {
        let records = vec![
            record(
                "1.1",
                AgentEvent::ToolCallStart {
                    turn_id: "turn-1".to_string(),
                    agent: sample_agent_context(),
                    tool_call_id: "call-plan-save".to_string(),
                    tool_name: "upsertSessionPlan".to_string(),
                    input: json!({
                        "title": "Cleanup crates",
                        "content": "# Plan: Cleanup crates"
                    }),
                },
            ),
            record(
                "1.2",
                AgentEvent::ToolCallResult {
                    turn_id: "turn-1".to_string(),
                    agent: sample_agent_context(),
                    result: ToolExecutionResult {
                        tool_call_id: "call-plan-save".to_string(),
                        tool_name: "upsertSessionPlan".to_string(),
                        ok: true,
                        output: "updated session plan".to_string(),
                        error: None,
                        metadata: Some(json!({
                            "planPath": "C:/Users/demo/.astrcode/projects/demo/sessions/session-1/plan/cleanup-crates.md",
                            "slug": "cleanup-crates",
                            "status": "draft",
                            "title": "Cleanup crates",
                            "updatedAt": "2026-04-19T09:00:00Z"
                        })),
                        child_ref: None,
                        duration_ms: 7,
                        truncated: false,
                    },
                },
            ),
            record(
                "1.3",
                AgentEvent::ToolCallStart {
                    turn_id: "turn-1".to_string(),
                    agent: sample_agent_context(),
                    tool_call_id: "call-shell".to_string(),
                    tool_name: "shell_command".to_string(),
                    input: json!({ "command": "pwd" }),
                },
            ),
            record(
                "1.4",
                AgentEvent::ToolCallResult {
                    turn_id: "turn-1".to_string(),
                    agent: sample_agent_context(),
                    result: ToolExecutionResult {
                        tool_call_id: "call-shell".to_string(),
                        tool_name: "shell_command".to_string(),
                        ok: true,
                        output: "D:/GitObjectsOwn/Astrcode".to_string(),
                        error: None,
                        metadata: None,
                        child_ref: None,
                        duration_ms: 9,
                        truncated: false,
                    },
                },
            ),
            record(
                "1.5",
                AgentEvent::ToolCallStart {
                    turn_id: "turn-1".to_string(),
                    agent: sample_agent_context(),
                    tool_call_id: "call-plan-exit".to_string(),
                    tool_name: "exitPlanMode".to_string(),
                    input: json!({}),
                },
            ),
            record(
                "1.6",
                AgentEvent::ToolCallResult {
                    turn_id: "turn-1".to_string(),
                    agent: sample_agent_context(),
                    result: ToolExecutionResult {
                        tool_call_id: "call-plan-exit".to_string(),
                        tool_name: "exitPlanMode".to_string(),
                        ok: true,
                        output: "Before exiting plan mode, do one final self-review.".to_string(),
                        error: None,
                        metadata: Some(json!({
                            "schema": "sessionPlanExitReviewPending",
                            "plan": {
                                "title": "Cleanup crates",
                                "planPath": "C:/Users/demo/.astrcode/projects/demo/sessions/session-1/plan/cleanup-crates.md"
                            },
                            "review": {
                                "kind": "final_review",
                                "checklist": [
                                    "Re-check assumptions against the code you already inspected."
                                ]
                            },
                            "blockers": {
                                "missingHeadings": ["## Verification"],
                                "invalidSections": []
                            }
                        })),
                        child_ref: None,
                        duration_ms: 5,
                        truncated: false,
                    },
                },
            ),
        ];

        let snapshot = project_conversation_snapshot(&records, Phase::Idle);
        assert_eq!(snapshot.blocks.len(), 3);
        assert!(matches!(
            &snapshot.blocks[0],
            ConversationBlockFacts::Plan(block)
                if block.tool_call_id == "call-plan-save"
                    && block.event_kind == ConversationPlanEventKind::Saved
        ));
        assert!(matches!(
            &snapshot.blocks[1],
            ConversationBlockFacts::ToolCall(block) if block.tool_call_id == "call-shell"
        ));
        assert!(matches!(
            &snapshot.blocks[2],
            ConversationBlockFacts::Plan(block)
                if block.tool_call_id == "call-plan-exit"
                    && block.event_kind == ConversationPlanEventKind::ReviewPending
        ));
    }

    #[test]
    fn live_then_durable_tool_delta_dedupes_chunk_on_same_tool_block() {
        let facts = sample_stream_replay_facts(
            vec![record(
                "1.1",
                AgentEvent::ToolCallStart {
                    turn_id: "turn-1".to_string(),
                    agent: sample_agent_context(),
                    tool_call_id: "call-1".to_string(),
                    tool_name: "shell_command".to_string(),
                    input: json!({ "command": "pwd" }),
                },
            )],
            vec![record(
                "1.2",
                AgentEvent::ToolCallDelta {
                    turn_id: "turn-1".to_string(),
                    agent: sample_agent_context(),
                    tool_call_id: "call-1".to_string(),
                    tool_name: "shell_command".to_string(),
                    stream: ToolOutputStream::Stdout,
                    delta: "line-1\n".to_string(),
                },
            )],
        );
        let mut stream = ConversationStreamProjector::new(Some("1.1".to_string()), &facts);

        let live_frames = stream.project_live_event(&AgentEvent::ToolCallDelta {
            turn_id: "turn-1".to_string(),
            agent: sample_agent_context(),
            tool_call_id: "call-1".to_string(),
            tool_name: "shell_command".to_string(),
            stream: ToolOutputStream::Stdout,
            delta: "line-1\n".to_string(),
        });
        assert_eq!(live_frames.len(), 1);

        let replayed = stream.recover_from(&facts);
        assert!(
            replayed.is_empty(),
            "durable replay should not duplicate the live-emitted chunk"
        );
    }

    #[test]
    fn child_notification_patches_tool_block_and_appends_handoff_block() {
        let mut projector = ConversationDeltaProjector::new();
        projector.seed(&[record(
            "1.1",
            AgentEvent::ToolCallStart {
                turn_id: "turn-1".to_string(),
                agent: sample_agent_context(),
                tool_call_id: "call-spawn".to_string(),
                tool_name: "spawn_agent".to_string(),
                input: json!({ "task": "inspect" }),
            },
        )]);

        let deltas = projector.project_record(&record(
            "1.2",
            AgentEvent::ChildSessionNotification {
                turn_id: Some("turn-1".to_string()),
                agent: sample_agent_context(),
                notification: sample_child_notification(),
            },
        ));

        assert!(deltas.iter().any(|delta| matches!(
            delta,
            ConversationDeltaFacts::PatchBlock {
                block_id,
                patch: ConversationBlockPatchFacts::ReplaceChildRef { .. },
            } if block_id == "tool:call-spawn:call"
        )));
        assert!(deltas.iter().any(|delta| matches!(
            delta,
            ConversationDeltaFacts::AppendBlock {
                block,
            } if matches!(
                block.as_ref(),
                ConversationBlockFacts::ChildHandoff(block)
                    if block.handoff_kind == ConversationChildHandoffKind::Returned
            )
        )));
    }

    #[tokio::test]
    async fn runtime_query_builds_snapshot_and_stream_replay_facts() {
        let event_store = Arc::new(ReplayOnlyEventStore::new(vec![
            stored(
                1,
                storage_event(
                    Some("turn-1"),
                    StorageEventPayload::UserMessage {
                        content: "inspect repo".to_string(),
                        origin: UserMessageOrigin::User,
                        timestamp: Utc::now(),
                    },
                ),
            ),
            stored(
                2,
                storage_event(
                    Some("turn-1"),
                    StorageEventPayload::ToolCall {
                        tool_call_id: "call-1".to_string(),
                        tool_name: "shell_command".to_string(),
                        args: json!({ "command": "pwd" }),
                    },
                ),
            ),
            stored(
                3,
                storage_event(
                    Some("turn-1"),
                    StorageEventPayload::ToolCallDelta {
                        tool_call_id: "call-1".to_string(),
                        tool_name: "shell_command".to_string(),
                        stream: ToolOutputStream::Stdout,
                        delta: "D:/GitObjectsOwn/Astrcode\n".to_string(),
                    },
                ),
            ),
            stored(
                4,
                storage_event(
                    Some("turn-1"),
                    StorageEventPayload::ToolResult {
                        tool_call_id: "call-1".to_string(),
                        tool_name: "shell_command".to_string(),
                        output: "D:/GitObjectsOwn/Astrcode\n".to_string(),
                        success: true,
                        error: None,
                        metadata: None,
                        child_ref: None,
                        duration_ms: 7,
                    },
                ),
            ),
            stored(
                5,
                storage_event(
                    Some("turn-1"),
                    StorageEventPayload::AssistantFinal {
                        content: "done".to_string(),
                        reasoning_content: Some("think".to_string()),
                        reasoning_signature: None,
                        timestamp: None,
                    },
                ),
            ),
        ]));
        let runtime = SessionRuntime::new(
            Arc::new(test_kernel(8192)),
            Arc::new(NoopPromptFactsProvider),
            event_store,
            Arc::new(NoopMetrics),
        );

        let snapshot = runtime
            .conversation_snapshot("session-1")
            .await
            .expect("snapshot should build");
        assert!(snapshot.blocks.iter().any(|block| matches!(
            block,
            ConversationBlockFacts::ToolCall(block)
                if block.tool_call_id == "call-1"
        )));

        let transcript = runtime
            .session_transcript_snapshot("session-1")
            .await
            .expect("transcript snapshot should build");
        assert!(transcript.records.len() > 4);
        let cursor = transcript.records[3].event_id.clone();

        let replay = runtime
            .conversation_stream_replay("session-1", Some(cursor.as_str()))
            .await
            .expect("replay facts should build");
        assert_eq!(
            replay
                .seed_records
                .last()
                .map(|record| record.event_id.as_str()),
            Some(cursor.as_str())
        );
        assert!(!replay.replay_frames.is_empty());
        assert_eq!(
            fallback_live_cursor(&replay).as_deref(),
            Some(cursor.as_str())
        );
    }

    fn sample_stream_replay_facts(
        seed_records: Vec<SessionEventRecord>,
        history: Vec<SessionEventRecord>,
    ) -> ConversationStreamReplayFacts {
        let (_, receiver) = broadcast::channel(8);
        let (_, live_receiver) = broadcast::channel(8);
        ConversationStreamReplayFacts {
            cursor: history.last().map(|record| record.event_id.clone()),
            phase: Phase::CallingTool,
            seed_records: seed_records.clone(),
            replay_frames: build_conversation_replay_frames(&seed_records, &history),
            replay: SessionReplay {
                history,
                receiver,
                live_receiver,
            },
        }
    }

    fn sample_agent_context() -> AgentEventContext {
        AgentEventContext::root_execution("agent-root", "default")
    }

    fn sample_child_notification() -> ChildSessionNotification {
        ChildSessionNotification {
            notification_id: "child-note-1".to_string().into(),
            child_ref: ChildAgentRef {
                identity: ChildExecutionIdentity {
                    agent_id: "agent-child-1".to_string().into(),
                    session_id: "session-root".to_string().into(),
                    sub_run_id: "subrun-child-1".to_string().into(),
                },
                parent: ParentExecutionRef {
                    parent_agent_id: Some("agent-root".to_string().into()),
                    parent_sub_run_id: Some("subrun-root".to_string().into()),
                },
                lineage_kind: ChildSessionLineageKind::Spawn,
                status: AgentLifecycleStatus::Running,
                open_session_id: "session-child-1".to_string().into(),
            },
            kind: ChildSessionNotificationKind::Delivered,
            source_tool_call_id: Some("call-spawn".to_string().into()),
            delivery: Some(ParentDelivery {
                idempotency_key: "delivery-1".to_string(),
                origin: ParentDeliveryOrigin::Explicit,
                terminal_semantics: ParentDeliveryTerminalSemantics::Terminal,
                source_turn_id: Some("turn-1".to_string()),
                payload: ParentDeliveryPayload::Progress(
                    astrcode_core::ProgressParentDeliveryPayload {
                        message: "child finished".to_string(),
                    },
                ),
            }),
        }
    }

    fn record(event_id: &str, event: AgentEvent) -> SessionEventRecord {
        SessionEventRecord {
            event_id: event_id.to_string(),
            event,
        }
    }

    fn stored(storage_seq: u64, event: StorageEvent) -> StoredEvent {
        StoredEvent { storage_seq, event }
    }

    fn storage_event(turn_id: Option<&str>, payload: StorageEventPayload) -> StorageEvent {
        StorageEvent {
            turn_id: turn_id.map(ToString::to_string),
            agent: sample_agent_context(),
            payload,
        }
    }

    struct ReplayOnlyEventStore {
        events: Vec<StoredEvent>,
    }

    impl ReplayOnlyEventStore {
        fn new(events: Vec<StoredEvent>) -> Self {
            Self { events }
        }
    }

    struct StubTurnLease;

    impl astrcode_core::SessionTurnLease for StubTurnLease {}

    #[async_trait]
    impl EventStore for ReplayOnlyEventStore {
        async fn ensure_session(
            &self,
            _session_id: &SessionId,
            _working_dir: &Path,
        ) -> astrcode_core::Result<()> {
            Ok(())
        }

        async fn append(
            &self,
            _session_id: &SessionId,
            _event: &astrcode_core::StorageEvent,
        ) -> astrcode_core::Result<StoredEvent> {
            panic!("append should not be called in replay-only test store");
        }

        async fn replay(&self, _session_id: &SessionId) -> astrcode_core::Result<Vec<StoredEvent>> {
            Ok(self.events.clone())
        }

        async fn try_acquire_turn(
            &self,
            _session_id: &SessionId,
            _turn_id: &str,
        ) -> astrcode_core::Result<SessionTurnAcquireResult> {
            Ok(SessionTurnAcquireResult::Acquired(Box::new(StubTurnLease)))
        }

        async fn list_sessions(&self) -> astrcode_core::Result<Vec<SessionId>> {
            Ok(vec![SessionId::from("session-1".to_string())])
        }

        async fn list_session_metas(&self) -> astrcode_core::Result<Vec<SessionMeta>> {
            Ok(vec![SessionMeta {
                session_id: "session-1".to_string(),
                working_dir: ".".to_string(),
                display_name: "session-1".to_string(),
                title: "session-1".to_string(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
                parent_session_id: None,
                parent_storage_seq: None,
                phase: Phase::Done,
            }])
        }

        async fn delete_session(&self, _session_id: &SessionId) -> astrcode_core::Result<()> {
            Ok(())
        }

        async fn delete_sessions_by_working_dir(
            &self,
            _working_dir: &str,
        ) -> astrcode_core::Result<DeleteProjectResult> {
            Ok(DeleteProjectResult {
                success_count: 0,
                failed_session_ids: Vec::new(),
            })
        }
    }
}
