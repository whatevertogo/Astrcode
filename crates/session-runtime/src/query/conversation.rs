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

mod facts;
#[path = "conversation/projection_support.rs"]
mod projection_support;

pub use facts::*;
use projection_support::*;
pub(crate) use projection_support::{
    build_conversation_replay_frames, project_conversation_snapshot,
};

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

#[cfg(test)]
mod tests;
