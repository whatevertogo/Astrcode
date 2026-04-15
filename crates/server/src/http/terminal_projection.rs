use std::collections::HashMap;

use astrcode_application::{
    TerminalChildSummaryFacts, TerminalControlFacts, TerminalFacts, TerminalRehydrateFacts,
    TerminalSlashAction, TerminalSlashCandidateFacts, TerminalStreamReplayFacts,
    terminal::truncate_terminal_summary,
};
use astrcode_core::{
    AgentEvent, AgentLifecycleStatus, ChildAgentRef, ChildSessionLineageKind, SessionEventRecord,
    ToolExecutionResult, ToolOutputStream,
};
use astrcode_protocol::http::{
    AgentLifecycleDto, ChildAgentRefDto, ChildSessionLineageKindDto, PhaseDto,
    TerminalAssistantBlockDto, TerminalBannerDto, TerminalBannerErrorCodeDto, TerminalBlockDto,
    TerminalBlockPatchDto, TerminalBlockStatusDto, TerminalChildHandoffBlockDto,
    TerminalChildHandoffKindDto, TerminalChildSummaryDto, TerminalControlStateDto,
    TerminalCursorDto, TerminalDeltaDto, TerminalErrorBlockDto, TerminalErrorEnvelopeDto,
    TerminalSlashActionKindDto, TerminalSlashCandidateDto, TerminalSlashCandidatesResponseDto,
    TerminalSnapshotResponseDto, TerminalStreamEnvelopeDto, TerminalSystemNoteBlockDto,
    TerminalSystemNoteKindDto, TerminalThinkingBlockDto, TerminalToolCallBlockDto,
    TerminalToolStreamBlockDto, TerminalTranscriptErrorCodeDto, TerminalUserBlockDto,
    ToolOutputStreamDto,
};
use serde_json::Value;

pub(crate) fn project_terminal_snapshot(facts: &TerminalFacts) -> TerminalSnapshotResponseDto {
    let child_lookup = child_summary_lookup(&facts.child_summaries);
    let mut projector = TerminalDeltaProjector::new(child_lookup);
    projector.seed(&facts.transcript.records);

    TerminalSnapshotResponseDto {
        session_id: facts.active_session_id.clone(),
        session_title: facts.session_title.clone(),
        cursor: TerminalCursorDto(
            facts
                .transcript
                .cursor
                .clone()
                .unwrap_or_else(|| "0.0".to_string()),
        ),
        phase: to_phase_dto(facts.control.phase),
        control: project_control_state(&facts.control),
        blocks: projector.blocks,
        child_summaries: facts
            .child_summaries
            .iter()
            .map(project_child_summary)
            .collect(),
        slash_candidates: facts
            .slash_candidates
            .iter()
            .map(project_slash_candidate)
            .collect(),
        banner: None,
    }
}

pub(crate) fn project_terminal_control_delta(control: &TerminalControlFacts) -> TerminalDeltaDto {
    TerminalDeltaDto::UpdateControlState {
        control: project_control_state(control),
    }
}

pub(crate) fn project_terminal_rehydrate_banner(
    rehydrate: &TerminalRehydrateFacts,
) -> TerminalBannerDto {
    TerminalBannerDto {
        error: TerminalErrorEnvelopeDto {
            code: TerminalBannerErrorCodeDto::CursorExpired,
            message: format!(
                "cursor '{}' is no longer valid for session '{}'",
                rehydrate.requested_cursor, rehydrate.session_id
            ),
            rehydrate_required: true,
            details: Some(serde_json::json!({
                "requestedCursor": rehydrate.requested_cursor,
                "latestCursor": rehydrate.latest_cursor,
                "reason": format!("{:?}", rehydrate.reason),
            })),
        },
    }
}

pub(crate) fn project_terminal_rehydrate_envelope(
    rehydrate: &TerminalRehydrateFacts,
) -> TerminalStreamEnvelopeDto {
    TerminalStreamEnvelopeDto {
        session_id: rehydrate.session_id.clone(),
        cursor: TerminalCursorDto(
            rehydrate
                .latest_cursor
                .clone()
                .unwrap_or_else(|| rehydrate.requested_cursor.clone()),
        ),
        delta: TerminalDeltaDto::RehydrateRequired {
            error: project_terminal_rehydrate_banner(rehydrate).error,
        },
    }
}

pub(crate) fn project_terminal_slash_candidates(
    candidates: &[TerminalSlashCandidateFacts],
) -> TerminalSlashCandidatesResponseDto {
    TerminalSlashCandidatesResponseDto {
        items: candidates.iter().map(project_slash_candidate).collect(),
    }
}

pub(crate) fn project_terminal_stream_replay(
    facts: &TerminalStreamReplayFacts,
    last_event_id: Option<&str>,
) -> Vec<TerminalStreamEnvelopeDto> {
    let mut projector = TerminalDeltaProjector::new(child_summary_lookup(&facts.child_summaries));
    let seed_records = transcript_before_cursor(&facts.seed_records, last_event_id);
    projector.seed(seed_records);

    let mut deltas = Vec::new();
    for record in &facts.replay.history {
        for delta in projector.project_record(record) {
            deltas.push(TerminalStreamEnvelopeDto {
                session_id: facts.active_session_id.clone(),
                cursor: TerminalCursorDto(record.event_id.clone()),
                delta,
            });
        }
    }
    deltas
}

pub(crate) fn seeded_terminal_stream_projector(
    facts: &TerminalStreamReplayFacts,
) -> TerminalDeltaProjector {
    let mut projector = TerminalDeltaProjector::new(child_summary_lookup(&facts.child_summaries));
    projector.seed(&facts.seed_records);
    projector
}

fn transcript_before_cursor<'a>(
    records: &'a [SessionEventRecord],
    last_event_id: Option<&str>,
) -> &'a [SessionEventRecord] {
    let Some(last_event_id) = last_event_id else {
        return &[];
    };
    let Some(index) = records
        .iter()
        .position(|record| record.event_id == last_event_id)
    else {
        return &[];
    };
    &records[..=index]
}

fn child_summary_lookup(
    summaries: &[TerminalChildSummaryFacts],
) -> HashMap<String, TerminalChildSummaryDto> {
    summaries
        .iter()
        .map(|summary| {
            (
                summary.node.child_session_id.clone(),
                project_child_summary(summary),
            )
        })
        .collect()
}

fn project_control_state(control: &TerminalControlFacts) -> TerminalControlStateDto {
    let can_submit_prompt = matches!(
        control.phase,
        astrcode_core::Phase::Idle | astrcode_core::Phase::Done | astrcode_core::Phase::Interrupted
    );
    TerminalControlStateDto {
        phase: to_phase_dto(control.phase),
        can_submit_prompt,
        can_request_compact: !control.manual_compact_pending,
        compact_pending: control.manual_compact_pending,
        active_turn_id: control.active_turn_id.clone(),
    }
}

pub(crate) fn project_child_summary(
    summary: &TerminalChildSummaryFacts,
) -> TerminalChildSummaryDto {
    TerminalChildSummaryDto {
        child_session_id: summary.node.child_session_id.clone(),
        child_agent_id: summary.node.agent_id.clone(),
        title: summary
            .title
            .clone()
            .or_else(|| summary.display_name.clone())
            .unwrap_or_else(|| summary.node.child_session_id.clone()),
        lifecycle: to_lifecycle_dto(summary.node.status),
        latest_output_summary: summary.recent_output.clone(),
        child_ref: Some(to_child_ref_dto(summary.node.child_ref())),
    }
}

pub(crate) fn project_terminal_child_summary_deltas(
    previous: &[TerminalChildSummaryFacts],
    current: &[TerminalChildSummaryFacts],
) -> Vec<TerminalDeltaDto> {
    let previous_by_id = previous
        .iter()
        .map(|summary| {
            (
                summary.node.child_session_id.clone(),
                project_child_summary(summary),
            )
        })
        .collect::<HashMap<_, _>>();
    let current_by_id = current
        .iter()
        .map(|summary| {
            (
                summary.node.child_session_id.clone(),
                project_child_summary(summary),
            )
        })
        .collect::<HashMap<_, _>>();

    let mut deltas = Vec::new();
    let mut removed_ids = previous_by_id
        .keys()
        .filter(|child_session_id| !current_by_id.contains_key(*child_session_id))
        .cloned()
        .collect::<Vec<_>>();
    removed_ids.sort();
    for child_session_id in removed_ids {
        deltas.push(TerminalDeltaDto::RemoveChildSummary { child_session_id });
    }

    let mut current_ids = current_by_id.keys().cloned().collect::<Vec<_>>();
    current_ids.sort();
    for child_session_id in current_ids {
        let current_child = current_by_id
            .get(&child_session_id)
            .expect("current child summary should exist");
        if previous_by_id.get(&child_session_id) != Some(current_child) {
            deltas.push(TerminalDeltaDto::UpsertChildSummary {
                child: current_child.clone(),
            });
        }
    }

    deltas
}

fn project_slash_candidate(candidate: &TerminalSlashCandidateFacts) -> TerminalSlashCandidateDto {
    let (action_kind, action_value) = match &candidate.action {
        TerminalSlashAction::CreateSession => (
            TerminalSlashActionKindDto::ExecuteCommand,
            "/new".to_string(),
        ),
        TerminalSlashAction::OpenResume => (
            TerminalSlashActionKindDto::ExecuteCommand,
            "/resume".to_string(),
        ),
        TerminalSlashAction::RequestCompact => (
            TerminalSlashActionKindDto::ExecuteCommand,
            "/compact".to_string(),
        ),
        TerminalSlashAction::OpenSkillPalette => (
            TerminalSlashActionKindDto::ExecuteCommand,
            "/skill".to_string(),
        ),
        TerminalSlashAction::InsertText { text } => {
            (TerminalSlashActionKindDto::InsertText, text.clone())
        },
    };

    let _ = candidate.kind;
    TerminalSlashCandidateDto {
        id: candidate.id.clone(),
        title: candidate.title.clone(),
        description: candidate.description.clone(),
        keywords: candidate.keywords.clone(),
        action_kind,
        action_value,
    }
}

fn to_phase_dto(phase: astrcode_core::Phase) -> PhaseDto {
    match phase {
        astrcode_core::Phase::Idle => PhaseDto::Idle,
        astrcode_core::Phase::Thinking => PhaseDto::Thinking,
        astrcode_core::Phase::CallingTool => PhaseDto::CallingTool,
        astrcode_core::Phase::Streaming => PhaseDto::Streaming,
        astrcode_core::Phase::Interrupted => PhaseDto::Interrupted,
        astrcode_core::Phase::Done => PhaseDto::Done,
    }
}

fn to_lifecycle_dto(status: AgentLifecycleStatus) -> AgentLifecycleDto {
    match status {
        AgentLifecycleStatus::Pending => AgentLifecycleDto::Pending,
        AgentLifecycleStatus::Running => AgentLifecycleDto::Running,
        AgentLifecycleStatus::Idle => AgentLifecycleDto::Idle,
        AgentLifecycleStatus::Terminated => AgentLifecycleDto::Terminated,
    }
}

fn to_stream_dto(stream: ToolOutputStream) -> ToolOutputStreamDto {
    match stream {
        ToolOutputStream::Stdout => ToolOutputStreamDto::Stdout,
        ToolOutputStream::Stderr => ToolOutputStreamDto::Stderr,
    }
}

fn to_child_ref_dto(child_ref: ChildAgentRef) -> ChildAgentRefDto {
    ChildAgentRefDto {
        agent_id: child_ref.agent_id,
        session_id: child_ref.session_id,
        sub_run_id: child_ref.sub_run_id,
        parent_agent_id: child_ref.parent_agent_id,
        parent_sub_run_id: child_ref.parent_sub_run_id,
        lineage_kind: match child_ref.lineage_kind {
            ChildSessionLineageKind::Spawn => ChildSessionLineageKindDto::Spawn,
            ChildSessionLineageKind::Fork => ChildSessionLineageKindDto::Fork,
            ChildSessionLineageKind::Resume => ChildSessionLineageKindDto::Resume,
        },
        status: to_lifecycle_dto(child_ref.status),
        open_session_id: child_ref.open_session_id,
    }
}

#[derive(Default)]
pub(crate) struct TerminalDeltaProjector {
    blocks: Vec<TerminalBlockDto>,
    block_index: HashMap<String, usize>,
    turn_blocks: HashMap<String, TurnBlockRefs>,
    tool_blocks: HashMap<String, ToolBlockRefs>,
    child_lookup: HashMap<String, TerminalChildSummaryDto>,
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
    stdout: Option<String>,
    stderr: Option<String>,
    pending_live_stdout_bytes: usize,
    pending_live_stderr_bytes: usize,
}

#[derive(Clone, Copy)]
enum BlockKind {
    Thinking,
    Assistant,
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

impl TerminalDeltaProjector {
    pub(crate) fn new(child_lookup: HashMap<String, TerminalChildSummaryDto>) -> Self {
        Self {
            child_lookup,
            ..Default::default()
        }
    }

    pub(crate) fn seed(&mut self, history: &[SessionEventRecord]) {
        for record in history {
            let _ = self.project_record(record);
        }
    }

    pub(crate) fn project_record(&mut self, record: &SessionEventRecord) -> Vec<TerminalDeltaDto> {
        self.project_event(
            &record.event,
            ProjectionSource::Durable,
            Some(&record.event_id),
        )
    }

    pub(crate) fn project_live_event(&mut self, event: &AgentEvent) -> Vec<TerminalDeltaDto> {
        self.project_event(event, ProjectionSource::Live, None)
    }

    fn project_event(
        &mut self,
        event: &AgentEvent,
        source: ProjectionSource,
        durable_event_id: Option<&str>,
    ) -> Vec<TerminalDeltaDto> {
        match event {
            AgentEvent::UserMessage {
                turn_id, content, ..
            } if source.is_durable() => {
                let block_id = format!("turn:{turn_id}:user");
                self.append_user_block(&block_id, turn_id, content)
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
            } => self.start_tool_call(turn_id, tool_call_id, tool_name, Some(input), source),
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
            } => self.complete_tool_call(turn_id.as_str(), result, source),
            AgentEvent::CompactApplied {
                turn_id, summary, ..
            } if source.is_durable() => {
                let block_id = format!(
                    "system:compact:{}",
                    turn_id
                        .clone()
                        .or_else(|| durable_event_id.map(ToString::to_string))
                        .unwrap_or_else(|| "session".to_string())
                );
                self.append_system_note(&block_id, TerminalSystemNoteKindDto::Compact, summary)
            },
            AgentEvent::ChildSessionNotification { notification, .. } => {
                if source.is_durable() {
                    self.append_child_handoff(notification)
                } else {
                    Vec::new()
                }
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
            | AgentEvent::AgentMailboxQueued { .. }
            | AgentEvent::AgentMailboxBatchStarted { .. }
            | AgentEvent::AgentMailboxBatchAcked { .. }
            | AgentEvent::AgentMailboxDiscarded { .. }
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
    ) -> Vec<TerminalDeltaDto> {
        if self.block_index.contains_key(block_id) {
            return Vec::new();
        }
        self.push_block(TerminalBlockDto::User(TerminalUserBlockDto {
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
    ) -> Vec<TerminalDeltaDto> {
        let block_id = self
            .turn_blocks
            .entry(turn_id.to_string())
            .or_default()
            .current_or_next_block_id(turn_id, kind);
        if let Some(index) = self.block_index.get(&block_id).copied() {
            self.append_markdown(index, delta);
            return vec![TerminalDeltaDto::PatchBlock {
                block_id,
                patch: TerminalBlockPatchDto::AppendMarkdown {
                    markdown: delta.to_string(),
                },
            }];
        }

        let block = match kind {
            BlockKind::Thinking => TerminalBlockDto::Thinking(TerminalThinkingBlockDto {
                id: block_id.clone(),
                turn_id: Some(turn_id.to_string()),
                status: TerminalBlockStatusDto::Streaming,
                markdown: delta.to_string(),
            }),
            BlockKind::Assistant => TerminalBlockDto::Assistant(TerminalAssistantBlockDto {
                id: block_id,
                turn_id: Some(turn_id.to_string()),
                status: TerminalBlockStatusDto::Streaming,
                markdown: delta.to_string(),
            }),
        };
        self.push_block(block)
    }

    fn finalize_assistant_block(
        &mut self,
        turn_id: &str,
        content: &str,
        reasoning_content: Option<&str>,
    ) -> Vec<TerminalDeltaDto> {
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

        if let (Some(reasoning_content), Some(thinking_id)) = (
            reasoning_content.filter(|value| !value.trim().is_empty()),
            thinking_id,
        ) {
            deltas.extend(self.ensure_full_markdown_block(
                &thinking_id,
                turn_id,
                reasoning_content,
                BlockKind::Thinking,
            ));
            if let Some(delta) = self.complete_block(&thinking_id, TerminalBlockStatusDto::Complete)
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
        if let Some(delta) = self.complete_block(&assistant_id, TerminalBlockStatusDto::Complete) {
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
    ) -> Vec<TerminalDeltaDto> {
        if let Some(index) = self.block_index.get(block_id).copied() {
            let existing = self.block_markdown(index);
            self.replace_markdown(index, content);
            if content.starts_with(&existing) {
                let suffix = &content[existing.len()..];
                if suffix.is_empty() {
                    return Vec::new();
                }
                return vec![TerminalDeltaDto::PatchBlock {
                    block_id: block_id.to_string(),
                    patch: TerminalBlockPatchDto::AppendMarkdown {
                        markdown: suffix.to_string(),
                    },
                }];
            }
            return vec![TerminalDeltaDto::PatchBlock {
                block_id: block_id.to_string(),
                patch: TerminalBlockPatchDto::ReplaceMarkdown {
                    markdown: content.to_string(),
                },
            }];
        }

        let block = match kind {
            BlockKind::Thinking => TerminalBlockDto::Thinking(TerminalThinkingBlockDto {
                id: block_id.to_string(),
                turn_id: Some(turn_id.to_string()),
                status: TerminalBlockStatusDto::Streaming,
                markdown: content.to_string(),
            }),
            BlockKind::Assistant => TerminalBlockDto::Assistant(TerminalAssistantBlockDto {
                id: block_id.to_string(),
                turn_id: Some(turn_id.to_string()),
                status: TerminalBlockStatusDto::Streaming,
                markdown: content.to_string(),
            }),
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
    ) -> Vec<TerminalDeltaDto> {
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
        self.push_block(TerminalBlockDto::ToolCall(TerminalToolCallBlockDto {
            id: block_id,
            turn_id: Some(turn_id.to_string()),
            tool_call_id: Some(tool_call_id.to_string()),
            tool_name: tool_name.to_string(),
            status: TerminalBlockStatusDto::Streaming,
            input: input.cloned(),
            summary: None,
        }))
    }

    fn append_tool_stream(
        &mut self,
        turn_id: &str,
        tool_call_id: &str,
        tool_name: &str,
        stream: ToolOutputStream,
        delta: &str,
        source: ProjectionSource,
    ) -> Vec<TerminalDeltaDto> {
        let mut deltas = self.start_tool_call(turn_id, tool_call_id, tool_name, None, source);
        let refs = self
            .tool_blocks
            .entry(tool_call_id.to_string())
            .or_default();
        let chunk = refs.reconcile_tool_chunk(stream, delta, source);
        if chunk.is_empty() {
            return deltas;
        }
        let block_id = match stream {
            ToolOutputStream::Stdout => refs
                .stdout
                .get_or_insert_with(|| format!("tool:{tool_call_id}:stdout"))
                .clone(),
            ToolOutputStream::Stderr => refs
                .stderr
                .get_or_insert_with(|| format!("tool:{tool_call_id}:stderr"))
                .clone(),
        };
        if let Some(index) = self.block_index.get(&block_id).copied() {
            self.append_tool_stream_content(index, &chunk);
            deltas.push(TerminalDeltaDto::PatchBlock {
                block_id,
                patch: TerminalBlockPatchDto::AppendToolStream {
                    stream: to_stream_dto(stream),
                    chunk,
                },
            });
            return deltas;
        }
        deltas.extend(
            self.push_block(TerminalBlockDto::ToolStream(TerminalToolStreamBlockDto {
                id: block_id,
                parent_tool_call_id: Some(tool_call_id.to_string()),
                stream: to_stream_dto(stream),
                status: TerminalBlockStatusDto::Streaming,
                content: chunk,
            })),
        );
        deltas
    }

    fn complete_tool_call(
        &mut self,
        turn_id: &str,
        result: &ToolExecutionResult,
        source: ProjectionSource,
    ) -> Vec<TerminalDeltaDto> {
        let mut deltas = self.start_tool_call(
            turn_id,
            &result.tool_call_id,
            &result.tool_name,
            None,
            source,
        );
        let status = if result.ok {
            TerminalBlockStatusDto::Complete
        } else {
            TerminalBlockStatusDto::Failed
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
        let refs = refs.clone();

        if let Some(call_block_id) = refs.call {
            if let Some(index) = self.block_index.get(&call_block_id).copied() {
                if self.replace_tool_summary(index, &summary) {
                    deltas.push(TerminalDeltaDto::PatchBlock {
                        block_id: call_block_id.clone(),
                        patch: TerminalBlockPatchDto::ReplaceSummary {
                            summary: summary.clone(),
                        },
                    });
                }
                if let Some(delta) = self.complete_block(&call_block_id, status) {
                    deltas.push(delta);
                }
            }
        }

        if let Some(stdout) = refs.stdout {
            if let Some(delta) = self.complete_block(&stdout, status) {
                deltas.push(delta);
            }
        }
        if let Some(stderr) = refs.stderr {
            if let Some(delta) = self.complete_block(&stderr, status) {
                deltas.push(delta);
            }
        }

        deltas
    }

    fn append_system_note(
        &mut self,
        block_id: &str,
        note_kind: TerminalSystemNoteKindDto,
        markdown: &str,
    ) -> Vec<TerminalDeltaDto> {
        if self.block_index.contains_key(block_id) {
            return Vec::new();
        }
        self.push_block(TerminalBlockDto::SystemNote(TerminalSystemNoteBlockDto {
            id: block_id.to_string(),
            note_kind,
            markdown: markdown.to_string(),
        }))
    }

    fn append_child_handoff(
        &mut self,
        notification: &astrcode_core::ChildSessionNotification,
    ) -> Vec<TerminalDeltaDto> {
        let block_id = format!("child:{}", notification.notification_id);
        if self.block_index.contains_key(&block_id) {
            return Vec::new();
        }
        let child = self
            .child_lookup
            .get(&notification.child_ref.open_session_id)
            .cloned()
            .unwrap_or_else(|| TerminalChildSummaryDto {
                child_session_id: notification.child_ref.open_session_id.clone(),
                child_agent_id: notification.child_ref.agent_id.clone(),
                title: notification.child_ref.open_session_id.clone(),
                lifecycle: to_lifecycle_dto(notification.status),
                latest_output_summary: notification
                    .delivery
                    .as_ref()
                    .map(|delivery| delivery.payload.message().to_string()),
                child_ref: Some(to_child_ref_dto(notification.child_ref.clone())),
            });
        self.push_block(TerminalBlockDto::ChildHandoff(
            TerminalChildHandoffBlockDto {
                id: block_id,
                handoff_kind: match notification.kind {
                    astrcode_core::ChildSessionNotificationKind::Started
                    | astrcode_core::ChildSessionNotificationKind::Resumed => {
                        TerminalChildHandoffKindDto::Delegated
                    },
                    astrcode_core::ChildSessionNotificationKind::ProgressSummary
                    | astrcode_core::ChildSessionNotificationKind::Waiting => {
                        TerminalChildHandoffKindDto::Progress
                    },
                    astrcode_core::ChildSessionNotificationKind::Delivered
                    | astrcode_core::ChildSessionNotificationKind::Closed
                    | astrcode_core::ChildSessionNotificationKind::Failed => {
                        TerminalChildHandoffKindDto::Returned
                    },
                },
                child,
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
    ) -> Vec<TerminalDeltaDto> {
        if code == "interrupted" {
            return Vec::new();
        }
        let block_id = format!("turn:{}:error", turn_id.unwrap_or("session"));
        if self.block_index.contains_key(&block_id) {
            return Vec::new();
        }
        self.push_block(TerminalBlockDto::Error(TerminalErrorBlockDto {
            id: block_id,
            turn_id: turn_id.map(ToString::to_string),
            code: classify_transcript_error(message),
            message: message.to_string(),
        }))
    }

    fn complete_turn(&mut self, turn_id: &str) -> Vec<TerminalDeltaDto> {
        let Some(refs) = self.turn_blocks.get(turn_id).cloned() else {
            return Vec::new();
        };
        let mut deltas = Vec::new();
        for block_id in refs.all_block_ids() {
            if let Some(delta) = self.complete_block(&block_id, TerminalBlockStatusDto::Complete) {
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
                if let Some(delta) = self.complete_block(call, TerminalBlockStatusDto::Complete) {
                    deltas.push(delta);
                }
            }
            if let Some(stdout) = &tool.stdout {
                if let Some(delta) = self.complete_block(stdout, TerminalBlockStatusDto::Complete) {
                    deltas.push(delta);
                }
            }
            if let Some(stderr) = &tool.stderr {
                if let Some(delta) = self.complete_block(stderr, TerminalBlockStatusDto::Complete) {
                    deltas.push(delta);
                }
            }
        }
        deltas
    }

    fn push_block(&mut self, block: TerminalBlockDto) -> Vec<TerminalDeltaDto> {
        let id = block_id(&block).to_string();
        self.block_index.insert(id, self.blocks.len());
        self.blocks.push(block.clone());
        vec![TerminalDeltaDto::AppendBlock { block }]
    }

    fn complete_block(
        &mut self,
        block_id: &str,
        status: TerminalBlockStatusDto,
    ) -> Option<TerminalDeltaDto> {
        if let Some(index) = self.block_index.get(block_id).copied() {
            if self.block_status(index) == Some(status) {
                return None;
            }
            self.set_status(index, status);
            return Some(TerminalDeltaDto::CompleteBlock {
                block_id: block_id.to_string(),
                status,
            });
        }
        None
    }

    fn append_markdown(&mut self, index: usize, markdown: &str) {
        match &mut self.blocks[index] {
            TerminalBlockDto::Thinking(block) => block.markdown.push_str(markdown),
            TerminalBlockDto::Assistant(block) => block.markdown.push_str(markdown),
            _ => {},
        }
    }

    fn replace_markdown(&mut self, index: usize, markdown: &str) {
        match &mut self.blocks[index] {
            TerminalBlockDto::Thinking(block) => block.markdown = markdown.to_string(),
            TerminalBlockDto::Assistant(block) => block.markdown = markdown.to_string(),
            _ => {},
        }
    }

    fn append_tool_stream_content(&mut self, index: usize, chunk: &str) {
        if let TerminalBlockDto::ToolStream(block) = &mut self.blocks[index] {
            block.content.push_str(chunk);
        }
    }

    fn replace_tool_summary(&mut self, index: usize, summary: &str) -> bool {
        if let TerminalBlockDto::ToolCall(block) = &mut self.blocks[index] {
            if block.summary.as_deref() == Some(summary) {
                return false;
            }
            block.summary = Some(summary.to_string());
            return true;
        }
        false
    }

    fn set_status(&mut self, index: usize, status: TerminalBlockStatusDto) {
        match &mut self.blocks[index] {
            TerminalBlockDto::Thinking(block) => block.status = status,
            TerminalBlockDto::Assistant(block) => block.status = status,
            TerminalBlockDto::ToolCall(block) => block.status = status,
            TerminalBlockDto::ToolStream(block) => block.status = status,
            _ => {},
        }
    }

    fn block_markdown(&self, index: usize) -> String {
        match &self.blocks[index] {
            TerminalBlockDto::Thinking(block) => block.markdown.clone(),
            TerminalBlockDto::Assistant(block) => block.markdown.clone(),
            _ => String::new(),
        }
    }

    fn block_status(&self, index: usize) -> Option<TerminalBlockStatusDto> {
        match &self.blocks[index] {
            TerminalBlockDto::Thinking(block) => Some(block.status),
            TerminalBlockDto::Assistant(block) => Some(block.status),
            TerminalBlockDto::ToolCall(block) => Some(block.status),
            TerminalBlockDto::ToolStream(block) => Some(block.status),
            _ => None,
        }
    }
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

fn block_id(block: &TerminalBlockDto) -> &str {
    match block {
        TerminalBlockDto::User(block) => &block.id,
        TerminalBlockDto::Assistant(block) => &block.id,
        TerminalBlockDto::Thinking(block) => &block.id,
        TerminalBlockDto::ToolCall(block) => &block.id,
        TerminalBlockDto::ToolStream(block) => &block.id,
        TerminalBlockDto::Error(block) => &block.id,
        TerminalBlockDto::SystemNote(block) => &block.id,
        TerminalBlockDto::ChildHandoff(block) => &block.id,
    }
}

fn tool_result_summary(result: &ToolExecutionResult) -> String {
    if result.ok {
        if !result.output.trim().is_empty() {
            truncate_terminal_summary(&result.output)
        } else {
            format!("{} completed", result.tool_name)
        }
    } else if let Some(error) = &result.error {
        truncate_terminal_summary(error)
    } else if !result.output.trim().is_empty() {
        truncate_terminal_summary(&result.output)
    } else {
        format!("{} failed", result.tool_name)
    }
}

fn classify_transcript_error(message: &str) -> TerminalTranscriptErrorCodeDto {
    let lower = message.to_lowercase();
    if lower.contains("context window") || lower.contains("token limit") {
        TerminalTranscriptErrorCodeDto::ContextWindowExceeded
    } else if lower.contains("rate limit") {
        TerminalTranscriptErrorCodeDto::RateLimit
    } else if lower.contains("tool") {
        TerminalTranscriptErrorCodeDto::ToolFatal
    } else {
        TerminalTranscriptErrorCodeDto::ProviderError
    }
}

#[cfg(test)]
mod tests {
    use astrcode_application::{
        ComposerOptionKind, SessionReplay, SessionTranscriptSnapshot, TerminalChildSummaryFacts,
        TerminalControlFacts, TerminalFacts, TerminalRehydrateFacts, TerminalRehydrateReason,
        TerminalSlashAction, TerminalSlashCandidateFacts, TerminalStreamReplayFacts,
    };
    use astrcode_core::{
        AgentEventContext, AgentLifecycleStatus, ChildSessionLineageKind, ChildSessionNode,
        ChildSessionNotification, ChildSessionNotificationKind, ChildSessionStatusSource,
        CompactTrigger, ParentDelivery, ParentDeliveryOrigin, ParentDeliveryPayload,
        ParentDeliveryTerminalSemantics, Phase, ProgressParentDeliveryPayload, SessionEventRecord,
        ToolExecutionResult, ToolOutputStream,
    };
    use serde_json::json;
    use tokio::sync::broadcast;

    use super::{
        AgentEvent, TerminalDeltaProjector, TerminalTranscriptErrorCodeDto,
        classify_transcript_error, project_terminal_control_delta,
        project_terminal_rehydrate_banner, project_terminal_snapshot,
        project_terminal_stream_replay,
    };

    #[test]
    fn project_terminal_snapshot_freezes_terminal_block_mapping() {
        let snapshot = project_terminal_snapshot(&sample_terminal_facts());

        assert_eq!(
            serde_json::to_value(snapshot).expect("snapshot should encode"),
            json!({
                "sessionId": "session-root",
                "sessionTitle": "Terminal session",
                "cursor": "1.11",
                "phase": "streaming",
                "control": {
                    "phase": "streaming",
                    "canSubmitPrompt": false,
                    "canRequestCompact": true,
                    "compactPending": false,
                    "activeTurnId": "turn-1"
                },
                "blocks": [
                    {
                        "kind": "user",
                        "id": "turn:turn-1:user",
                        "turnId": "turn-1",
                        "markdown": "实现 terminal v1"
                    },
                    {
                        "kind": "thinking",
                        "id": "turn:turn-1:thinking",
                        "turnId": "turn-1",
                        "status": "complete",
                        "markdown": "先整理协议"
                    },
                    {
                        "kind": "assistant",
                        "id": "turn:turn-1:assistant",
                        "turnId": "turn-1",
                        "status": "complete",
                        "markdown": "正在实现 terminal v1。"
                    },
                    {
                        "kind": "tool_call",
                        "id": "tool:call-1:call",
                        "turnId": "turn-1",
                        "toolCallId": "call-1",
                        "toolName": "shell_command",
                        "status": "complete",
                        "input": {
                            "command": "rg terminal"
                        },
                        "summary": "read files"
                    },
                    {
                        "kind": "tool_stream",
                        "id": "tool:call-1:stdout",
                        "parentToolCallId": "call-1",
                        "stream": "stdout",
                        "status": "complete",
                        "content": "read files"
                    },
                    {
                        "kind": "system_note",
                        "id": "system:compact:turn-1",
                        "noteKind": "compact",
                        "markdown": "已压缩上下文"
                    },
                    {
                        "kind": "child_handoff",
                        "id": "child:child-note-1",
                        "handoffKind": "returned",
                        "child": child_summary_json("子任务已完成"),
                        "message": "子任务已完成"
                    },
                    {
                        "kind": "error",
                        "id": "turn:turn-1:error",
                        "turnId": "turn-1",
                        "code": "rate_limit",
                        "message": "provider rate limit on this turn"
                    }
                ],
                "childSummaries": [child_summary_json("子任务已完成")],
                "slashCandidates": [
                    {
                        "id": "slash-compact",
                        "title": "/compact",
                        "description": "压缩当前会话",
                        "keywords": ["compact", "summary"],
                        "actionKind": "execute_command",
                        "actionValue": "/compact"
                    },
                    {
                        "id": "slash-skill-review",
                        "title": "Review skill",
                        "description": "插入 review skill",
                        "keywords": ["skill", "review"],
                        "actionKind": "insert_text",
                        "actionValue": "/skill review"
                    }
                ]
            })
        );
    }

    #[test]
    fn project_terminal_stream_replay_freezes_patch_and_completion_semantics() {
        let deltas = project_terminal_stream_replay(&sample_stream_replay_facts(), Some("1.2"));

        assert_eq!(
            serde_json::to_value(deltas).expect("stream deltas should encode"),
            json!([
                {
                    "sessionId": "session-root",
                    "cursor": "1.3",
                    "kind": "patch_block",
                    "blockId": "turn:turn-1:thinking",
                    "patch": {
                        "kind": "append_markdown",
                        "markdown": "整理"
                    }
                },
                {
                    "sessionId": "session-root",
                    "cursor": "1.4",
                    "kind": "append_block",
                    "block": {
                        "kind": "assistant",
                        "id": "turn:turn-1:assistant",
                        "turnId": "turn-1",
                        "status": "streaming",
                        "markdown": "执行中"
                    }
                },
                {
                    "sessionId": "session-root",
                    "cursor": "1.5",
                    "kind": "append_block",
                    "block": {
                        "kind": "tool_call",
                        "id": "tool:call-1:call",
                        "turnId": "turn-1",
                        "toolCallId": "call-1",
                        "toolName": "shell_command",
                        "status": "streaming",
                        "input": {
                            "command": "pwd"
                        }
                    }
                },
                {
                    "sessionId": "session-root",
                    "cursor": "1.6",
                    "kind": "append_block",
                    "block": {
                        "kind": "tool_stream",
                        "id": "tool:call-1:stdout",
                        "parentToolCallId": "call-1",
                        "stream": "stdout",
                        "status": "streaming",
                        "content": "line 1\n"
                    }
                },
                {
                    "sessionId": "session-root",
                    "cursor": "1.7",
                    "kind": "patch_block",
                    "blockId": "tool:call-1:call",
                    "patch": {
                        "kind": "replace_summary",
                        "summary": "line 1"
                    }
                },
                {
                    "sessionId": "session-root",
                    "cursor": "1.7",
                    "kind": "complete_block",
                    "blockId": "tool:call-1:call",
                    "status": "complete"
                },
                {
                    "sessionId": "session-root",
                    "cursor": "1.7",
                    "kind": "complete_block",
                    "blockId": "tool:call-1:stdout",
                    "status": "complete"
                },
                {
                    "sessionId": "session-root",
                    "cursor": "1.8",
                    "kind": "append_block",
                    "block": {
                        "kind": "child_handoff",
                        "id": "child:child-note-2",
                        "handoffKind": "progress",
                        "child": child_summary_json("子任务进行中"),
                        "message": "子任务进行中"
                    }
                },
                {
                    "sessionId": "session-root",
                    "cursor": "1.9",
                    "kind": "append_block",
                    "block": {
                        "kind": "error",
                        "id": "turn:turn-1:error",
                        "turnId": "turn-1",
                        "code": "tool_fatal",
                        "message": "tool fatal: shell exited"
                    }
                },
                {
                    "sessionId": "session-root",
                    "cursor": "1.10",
                    "kind": "complete_block",
                    "blockId": "turn:turn-1:thinking",
                    "status": "complete"
                },
                {
                    "sessionId": "session-root",
                    "cursor": "1.10",
                    "kind": "complete_block",
                    "blockId": "turn:turn-1:assistant",
                    "status": "complete"
                }
            ])
        );
    }

    #[test]
    fn finalize_assistant_block_emits_replace_markdown_when_final_content_diverges() {
        let mut projector = TerminalDeltaProjector::default();
        let thinking_delta = record(
            "1.1",
            AgentEvent::ThinkingDelta {
                turn_id: "turn-1".to_string(),
                agent: sample_agent_context(),
                delta: "旧前缀".to_string(),
            },
        );
        projector.seed(&[thinking_delta]);

        let assistant_message = record(
            "1.2",
            AgentEvent::AssistantMessage {
                turn_id: "turn-1".to_string(),
                agent: sample_agent_context(),
                content: "最终回复".to_string(),
                reasoning_content: Some("全新推理".to_string()),
            },
        );

        assert_eq!(
            serde_json::to_value(projector.project_record(&assistant_message))
                .expect("deltas should encode"),
            json!([
                {
                    "kind": "patch_block",
                    "blockId": "turn:turn-1:thinking",
                    "patch": {
                        "kind": "replace_markdown",
                        "markdown": "全新推理"
                    }
                },
                {
                    "kind": "complete_block",
                    "blockId": "turn:turn-1:thinking",
                    "status": "complete"
                },
                {
                    "kind": "append_block",
                    "block": {
                        "kind": "assistant",
                        "id": "turn:turn-1:assistant",
                        "turnId": "turn-1",
                        "status": "streaming",
                        "markdown": "最终回复"
                    }
                },
                {
                    "kind": "complete_block",
                    "blockId": "turn:turn-1:assistant",
                    "status": "complete"
                }
            ])
        );
    }

    #[test]
    fn live_events_project_streaming_reasoning_and_assistant_blocks() {
        let mut projector = TerminalDeltaProjector::default();
        let agent = sample_agent_context();

        assert_eq!(
            serde_json::to_value(projector.project_live_event(&AgentEvent::ThinkingDelta {
                turn_id: "turn-1".to_string(),
                agent: agent.clone(),
                delta: "先".to_string(),
            }))
            .expect("thinking live deltas should encode"),
            json!([
                {
                    "kind": "append_block",
                    "block": {
                        "kind": "thinking",
                        "id": "turn:turn-1:thinking",
                        "turnId": "turn-1",
                        "status": "streaming",
                        "markdown": "先"
                    }
                }
            ])
        );

        assert_eq!(
            serde_json::to_value(projector.project_live_event(&AgentEvent::ThinkingDelta {
                turn_id: "turn-1".to_string(),
                agent: agent.clone(),
                delta: "整理协议".to_string(),
            }))
            .expect("thinking append deltas should encode"),
            json!([
                {
                    "kind": "patch_block",
                    "blockId": "turn:turn-1:thinking",
                    "patch": {
                        "kind": "append_markdown",
                        "markdown": "整理协议"
                    }
                }
            ])
        );

        assert_eq!(
            serde_json::to_value(projector.project_live_event(&AgentEvent::ModelDelta {
                turn_id: "turn-1".to_string(),
                agent,
                delta: "正在输出".to_string(),
            }))
            .expect("assistant live deltas should encode"),
            json!([
                {
                    "kind": "append_block",
                    "block": {
                        "kind": "assistant",
                        "id": "turn:turn-1:assistant",
                        "turnId": "turn-1",
                        "status": "streaming",
                        "markdown": "正在输出"
                    }
                }
            ])
        );
    }

    #[test]
    fn live_tool_streams_render_immediately_and_durable_replay_does_not_duplicate_chunks() {
        let mut projector = TerminalDeltaProjector::default();
        let agent = sample_agent_context();

        assert_eq!(
            serde_json::to_value(projector.project_live_event(&AgentEvent::ToolCallStart {
                turn_id: "turn-1".to_string(),
                agent: agent.clone(),
                tool_call_id: "call-1".to_string(),
                tool_name: "web".to_string(),
                input: serde_json::json!({}),
            }))
            .expect("tool call start should encode"),
            json!([
                {
                    "kind": "append_block",
                    "block": {
                        "kind": "tool_call",
                        "id": "tool:call-1:call",
                        "turnId": "turn-1",
                        "toolCallId": "call-1",
                        "toolName": "web",
                        "status": "streaming",
                        "input": {}
                    }
                }
            ])
        );

        assert_eq!(
            serde_json::to_value(projector.project_live_event(&AgentEvent::ToolCallDelta {
                turn_id: "turn-1".to_string(),
                agent: agent.clone(),
                tool_call_id: "call-1".to_string(),
                tool_name: "web".to_string(),
                stream: ToolOutputStream::Stdout,
                delta: "first chunk\n".to_string(),
            }))
            .expect("tool stream delta should encode"),
            json!([
                {
                    "kind": "append_block",
                    "block": {
                        "kind": "tool_stream",
                        "id": "tool:call-1:stdout",
                        "parentToolCallId": "call-1",
                        "stream": "stdout",
                        "status": "streaming",
                        "content": "first chunk\n"
                    }
                }
            ])
        );

        assert_eq!(
            serde_json::to_value(projector.project_live_event(&AgentEvent::ToolCallResult {
                turn_id: "turn-1".to_string(),
                agent: sample_agent_context(),
                result: ToolExecutionResult {
                    tool_call_id: "call-1".to_string(),
                    tool_name: "web".to_string(),
                    ok: true,
                    output: "first chunk\n".to_string(),
                    error: None,
                    metadata: None,
                    duration_ms: 0,
                    truncated: false,
                },
            }))
            .expect("live tool result should encode"),
            json!([
                {
                    "kind": "patch_block",
                    "blockId": "tool:call-1:call",
                    "patch": {
                        "kind": "replace_summary",
                        "summary": "first chunk"
                    }
                },
                {
                    "kind": "complete_block",
                    "blockId": "tool:call-1:call",
                    "status": "complete"
                },
                {
                    "kind": "complete_block",
                    "blockId": "tool:call-1:stdout",
                    "status": "complete"
                }
            ])
        );

        assert_eq!(
            serde_json::to_value(projector.project_record(&record(
                "1.1",
                AgentEvent::ToolCallStart {
                    turn_id: "turn-1".to_string(),
                    agent: sample_agent_context(),
                    tool_call_id: "call-1".to_string(),
                    tool_name: "web".to_string(),
                    input: serde_json::json!({}),
                }
            )))
            .expect("durable tool call start should encode"),
            json!([])
        );

        assert_eq!(
            serde_json::to_value(projector.project_record(&record(
                "1.2",
                AgentEvent::ToolCallDelta {
                    turn_id: "turn-1".to_string(),
                    agent: sample_agent_context(),
                    tool_call_id: "call-1".to_string(),
                    tool_name: "web".to_string(),
                    stream: ToolOutputStream::Stdout,
                    delta: "first chunk\n".to_string(),
                }
            )))
            .expect("durable delta after live completion should still skip duplicated chunk"),
            json!([])
        );

        assert_eq!(
            serde_json::to_value(projector.project_record(&record(
                "1.3",
                AgentEvent::ToolCallResult {
                    turn_id: "turn-1".to_string(),
                    agent: sample_agent_context(),
                    result: ToolExecutionResult {
                        tool_call_id: "call-1".to_string(),
                        tool_name: "web".to_string(),
                        ok: true,
                        output: "first chunk\n".to_string(),
                        error: None,
                        metadata: None,
                        duration_ms: 0,
                        truncated: false,
                    },
                }
            )))
            .expect("durable result should collapse to no-op after matching live completion"),
            json!([])
        );
    }

    #[test]
    fn durable_multi_step_turn_keeps_final_assistant_after_tool_blocks() {
        let mut projector = TerminalDeltaProjector::default();
        let agent = sample_agent_context();

        projector.seed(&[
            record(
                "1.1",
                AgentEvent::AssistantMessage {
                    turn_id: "turn-1".to_string(),
                    agent: agent.clone(),
                    content: "好的，让我先浏览一下项目。".to_string(),
                    reasoning_content: None,
                },
            ),
            record(
                "1.2",
                AgentEvent::ToolCallStart {
                    turn_id: "turn-1".to_string(),
                    agent: agent.clone(),
                    tool_call_id: "call-1".to_string(),
                    tool_name: "listDir".to_string(),
                    input: json!({ "path": "." }),
                },
            ),
            record(
                "1.3",
                AgentEvent::ToolCallResult {
                    turn_id: "turn-1".to_string(),
                    agent: agent.clone(),
                    result: ToolExecutionResult {
                        tool_call_id: "call-1".to_string(),
                        tool_name: "listDir".to_string(),
                        ok: true,
                        output: "[{\"name\":\"crates\"}]".to_string(),
                        error: None,
                        metadata: None,
                        duration_ms: 1,
                        truncated: false,
                    },
                },
            ),
            record(
                "1.4",
                AgentEvent::AssistantMessage {
                    turn_id: "turn-1".to_string(),
                    agent,
                    content: "现在我对项目有了全面的了解。".to_string(),
                    reasoning_content: None,
                },
            ),
        ]);

        assert_eq!(
            serde_json::to_value(&projector.blocks).expect("blocks should encode"),
            json!([
                {
                    "kind": "assistant",
                    "id": "turn:turn-1:assistant",
                    "turnId": "turn-1",
                    "status": "complete",
                    "markdown": "好的，让我先浏览一下项目。"
                },
                {
                    "kind": "tool_call",
                    "id": "tool:call-1:call",
                    "turnId": "turn-1",
                    "toolCallId": "call-1",
                    "toolName": "listDir",
                    "status": "complete",
                    "input": {
                        "path": "."
                    },
                    "summary": "[{\"name\":\"crates\"}]"
                },
                {
                    "kind": "assistant",
                    "id": "turn:turn-1:assistant:2",
                    "turnId": "turn-1",
                    "status": "complete",
                    "markdown": "现在我对项目有了全面的了解。"
                }
            ])
        );
    }

    #[test]
    fn classify_transcript_error_covers_all_supported_buckets() {
        assert_eq!(
            classify_transcript_error("context window exceeded"),
            TerminalTranscriptErrorCodeDto::ContextWindowExceeded
        );
        assert_eq!(
            classify_transcript_error("provider rate limit hit"),
            TerminalTranscriptErrorCodeDto::RateLimit
        );
        assert_eq!(
            classify_transcript_error("tool process exited with failure"),
            TerminalTranscriptErrorCodeDto::ToolFatal
        );
        assert_eq!(
            classify_transcript_error("unexpected provider response"),
            TerminalTranscriptErrorCodeDto::ProviderError
        );
    }

    #[test]
    fn project_terminal_control_and_rehydrate_errors_use_terminal_contract() {
        let control_delta = project_terminal_control_delta(&TerminalControlFacts {
            phase: Phase::Idle,
            active_turn_id: None,
            manual_compact_pending: true,
        });
        assert_eq!(
            serde_json::to_value(control_delta).expect("control delta should encode"),
            json!({
                "kind": "update_control_state",
                "control": {
                    "phase": "idle",
                    "canSubmitPrompt": true,
                    "canRequestCompact": false,
                    "compactPending": true
                }
            })
        );

        let banner = project_terminal_rehydrate_banner(&TerminalRehydrateFacts {
            session_id: "session-root".to_string(),
            requested_cursor: "1.2".to_string(),
            latest_cursor: Some("1.10".to_string()),
            reason: TerminalRehydrateReason::CursorExpired,
        });
        assert_eq!(
            serde_json::to_value(banner).expect("banner should encode"),
            json!({
                "error": {
                    "code": "cursor_expired",
                    "message": "cursor '1.2' is no longer valid for session 'session-root'",
                    "rehydrateRequired": true,
                    "details": {
                        "requestedCursor": "1.2",
                        "latestCursor": "1.10",
                        "reason": "CursorExpired"
                    }
                }
            })
        );
    }

    fn sample_terminal_facts() -> TerminalFacts {
        TerminalFacts {
            active_session_id: "session-root".to_string(),
            session_title: "Terminal session".to_string(),
            transcript: SessionTranscriptSnapshot {
                records: vec![
                    record(
                        "1.1",
                        AgentEvent::UserMessage {
                            turn_id: "turn-1".to_string(),
                            agent: sample_agent_context(),
                            content: "实现 terminal v1".to_string(),
                        },
                    ),
                    record(
                        "1.2",
                        AgentEvent::ThinkingDelta {
                            turn_id: "turn-1".to_string(),
                            agent: sample_agent_context(),
                            delta: "先整理协议".to_string(),
                        },
                    ),
                    record(
                        "1.3",
                        AgentEvent::ModelDelta {
                            turn_id: "turn-1".to_string(),
                            agent: sample_agent_context(),
                            delta: "正在实现".to_string(),
                        },
                    ),
                    record(
                        "1.4",
                        AgentEvent::AssistantMessage {
                            turn_id: "turn-1".to_string(),
                            agent: sample_agent_context(),
                            content: "正在实现 terminal v1。".to_string(),
                            reasoning_content: Some("先整理协议".to_string()),
                        },
                    ),
                    record(
                        "1.5",
                        AgentEvent::ToolCallStart {
                            turn_id: "turn-1".to_string(),
                            agent: sample_agent_context(),
                            tool_call_id: "call-1".to_string(),
                            tool_name: "shell_command".to_string(),
                            input: json!({ "command": "rg terminal" }),
                        },
                    ),
                    record(
                        "1.6",
                        AgentEvent::ToolCallDelta {
                            turn_id: "turn-1".to_string(),
                            agent: sample_agent_context(),
                            tool_call_id: "call-1".to_string(),
                            tool_name: "shell_command".to_string(),
                            stream: ToolOutputStream::Stdout,
                            delta: "read files".to_string(),
                        },
                    ),
                    record(
                        "1.7",
                        AgentEvent::ToolCallResult {
                            turn_id: "turn-1".to_string(),
                            agent: sample_agent_context(),
                            result: ToolExecutionResult {
                                tool_call_id: "call-1".to_string(),
                                tool_name: "shell_command".to_string(),
                                ok: true,
                                output: "read files".to_string(),
                                error: None,
                                metadata: None,
                                duration_ms: 12,
                                truncated: false,
                            },
                        },
                    ),
                    record(
                        "1.8",
                        AgentEvent::CompactApplied {
                            turn_id: Some("turn-1".to_string()),
                            agent: sample_agent_context(),
                            trigger: CompactTrigger::Manual,
                            summary: "已压缩上下文".to_string(),
                            preserved_recent_turns: 4,
                        },
                    ),
                    record(
                        "1.9",
                        AgentEvent::ChildSessionNotification {
                            turn_id: Some("turn-1".to_string()),
                            agent: sample_agent_context(),
                            notification: sample_child_notification(
                                "child-note-1",
                                ChildSessionNotificationKind::Delivered,
                                AgentLifecycleStatus::Idle,
                                "子任务已完成",
                            ),
                        },
                    ),
                    record(
                        "1.10",
                        AgentEvent::Error {
                            turn_id: Some("turn-1".to_string()),
                            agent: sample_agent_context(),
                            code: "provider_error".to_string(),
                            message: "provider rate limit on this turn".to_string(),
                        },
                    ),
                    record(
                        "1.11",
                        AgentEvent::TurnDone {
                            turn_id: "turn-1".to_string(),
                            agent: sample_agent_context(),
                        },
                    ),
                ],
                cursor: Some("1.11".to_string()),
                phase: Phase::Streaming,
            },
            control: TerminalControlFacts {
                phase: Phase::Streaming,
                active_turn_id: Some("turn-1".to_string()),
                manual_compact_pending: false,
            },
            child_summaries: vec![sample_child_summary_facts("子任务已完成")],
            slash_candidates: vec![
                TerminalSlashCandidateFacts {
                    kind: ComposerOptionKind::Command,
                    id: "slash-compact".to_string(),
                    title: "/compact".to_string(),
                    description: "压缩当前会话".to_string(),
                    keywords: vec!["compact".to_string(), "summary".to_string()],
                    badges: Vec::new(),
                    action: TerminalSlashAction::RequestCompact,
                },
                TerminalSlashCandidateFacts {
                    kind: ComposerOptionKind::Skill,
                    id: "slash-skill-review".to_string(),
                    title: "Review skill".to_string(),
                    description: "插入 review skill".to_string(),
                    keywords: vec!["skill".to_string(), "review".to_string()],
                    badges: Vec::new(),
                    action: TerminalSlashAction::InsertText {
                        text: "/skill review".to_string(),
                    },
                },
            ],
        }
    }

    fn sample_stream_replay_facts() -> TerminalStreamReplayFacts {
        let (_, receiver) = broadcast::channel(8);
        let (_, live_receiver) = broadcast::channel(8);

        TerminalStreamReplayFacts {
            active_session_id: "session-root".to_string(),
            seed_records: vec![
                record(
                    "1.1",
                    AgentEvent::UserMessage {
                        turn_id: "turn-1".to_string(),
                        agent: sample_agent_context(),
                        content: "实现 terminal v1".to_string(),
                    },
                ),
                record(
                    "1.2",
                    AgentEvent::ThinkingDelta {
                        turn_id: "turn-1".to_string(),
                        agent: sample_agent_context(),
                        delta: "先".to_string(),
                    },
                ),
            ],
            replay: SessionReplay {
                history: vec![
                    record(
                        "1.3",
                        AgentEvent::ThinkingDelta {
                            turn_id: "turn-1".to_string(),
                            agent: sample_agent_context(),
                            delta: "整理".to_string(),
                        },
                    ),
                    record(
                        "1.4",
                        AgentEvent::ModelDelta {
                            turn_id: "turn-1".to_string(),
                            agent: sample_agent_context(),
                            delta: "执行中".to_string(),
                        },
                    ),
                    record(
                        "1.5",
                        AgentEvent::ToolCallStart {
                            turn_id: "turn-1".to_string(),
                            agent: sample_agent_context(),
                            tool_call_id: "call-1".to_string(),
                            tool_name: "shell_command".to_string(),
                            input: json!({ "command": "pwd" }),
                        },
                    ),
                    record(
                        "1.6",
                        AgentEvent::ToolCallDelta {
                            turn_id: "turn-1".to_string(),
                            agent: sample_agent_context(),
                            tool_call_id: "call-1".to_string(),
                            tool_name: "shell_command".to_string(),
                            stream: ToolOutputStream::Stdout,
                            delta: "line 1\n".to_string(),
                        },
                    ),
                    record(
                        "1.7",
                        AgentEvent::ToolCallResult {
                            turn_id: "turn-1".to_string(),
                            agent: sample_agent_context(),
                            result: ToolExecutionResult {
                                tool_call_id: "call-1".to_string(),
                                tool_name: "shell_command".to_string(),
                                ok: true,
                                output: "line 1\n".to_string(),
                                error: None,
                                metadata: None,
                                duration_ms: 8,
                                truncated: false,
                            },
                        },
                    ),
                    record(
                        "1.8",
                        AgentEvent::ChildSessionNotification {
                            turn_id: Some("turn-1".to_string()),
                            agent: sample_agent_context(),
                            notification: sample_child_notification(
                                "child-note-2",
                                ChildSessionNotificationKind::Waiting,
                                AgentLifecycleStatus::Running,
                                "子任务进行中",
                            ),
                        },
                    ),
                    record(
                        "1.9",
                        AgentEvent::Error {
                            turn_id: Some("turn-1".to_string()),
                            agent: sample_agent_context(),
                            code: "tool_error".to_string(),
                            message: "tool fatal: shell exited".to_string(),
                        },
                    ),
                    record(
                        "1.10",
                        AgentEvent::TurnDone {
                            turn_id: "turn-1".to_string(),
                            agent: sample_agent_context(),
                        },
                    ),
                ],
                receiver,
                live_receiver,
            },
            control: TerminalControlFacts {
                phase: Phase::Streaming,
                active_turn_id: Some("turn-1".to_string()),
                manual_compact_pending: false,
            },
            child_summaries: vec![sample_child_summary_facts("子任务进行中")],
            slash_candidates: vec![
                TerminalSlashCandidateFacts {
                    kind: ComposerOptionKind::Command,
                    id: "slash-compact".to_string(),
                    title: "/compact".to_string(),
                    description: "压缩当前会话".to_string(),
                    keywords: vec!["compact".to_string(), "summary".to_string()],
                    badges: Vec::new(),
                    action: TerminalSlashAction::RequestCompact,
                },
                TerminalSlashCandidateFacts {
                    kind: ComposerOptionKind::Skill,
                    id: "slash-skill-review".to_string(),
                    title: "Review skill".to_string(),
                    description: "插入 review skill".to_string(),
                    keywords: vec!["skill".to_string(), "review".to_string()],
                    badges: Vec::new(),
                    action: TerminalSlashAction::InsertText {
                        text: "/skill review".to_string(),
                    },
                },
            ],
        }
    }

    fn sample_child_summary_facts(recent_output: &str) -> TerminalChildSummaryFacts {
        TerminalChildSummaryFacts {
            node: sample_child_node(),
            phase: Phase::Streaming,
            title: Some("Repo inspector".to_string()),
            display_name: Some("repo-inspector".to_string()),
            recent_output: Some(recent_output.to_string()),
        }
    }

    fn sample_child_node() -> ChildSessionNode {
        ChildSessionNode {
            agent_id: "agent-child".to_string(),
            session_id: "session-root".to_string(),
            child_session_id: "session-child".to_string(),
            sub_run_id: "subrun-child".to_string(),
            parent_session_id: "session-root".to_string(),
            parent_agent_id: Some("agent-root".to_string()),
            parent_sub_run_id: Some("subrun-root".to_string()),
            parent_turn_id: "turn-1".to_string(),
            lineage_kind: ChildSessionLineageKind::Spawn,
            status: AgentLifecycleStatus::Running,
            status_source: ChildSessionStatusSource::Durable,
            created_by_tool_call_id: Some("call-1".to_string()),
            lineage_snapshot: None,
        }
    }

    fn sample_child_notification(
        notification_id: &str,
        kind: ChildSessionNotificationKind,
        status: AgentLifecycleStatus,
        message: &str,
    ) -> ChildSessionNotification {
        ChildSessionNotification {
            notification_id: notification_id.to_string(),
            child_ref: sample_child_node().child_ref(),
            kind,
            status,
            source_tool_call_id: Some("call-1".to_string()),
            delivery: Some(ParentDelivery {
                idempotency_key: notification_id.to_string(),
                origin: ParentDeliveryOrigin::Explicit,
                terminal_semantics: match kind {
                    ChildSessionNotificationKind::Started
                    | ChildSessionNotificationKind::ProgressSummary
                    | ChildSessionNotificationKind::Waiting
                    | ChildSessionNotificationKind::Resumed => {
                        ParentDeliveryTerminalSemantics::NonTerminal
                    },
                    ChildSessionNotificationKind::Delivered
                    | ChildSessionNotificationKind::Closed
                    | ChildSessionNotificationKind::Failed => {
                        ParentDeliveryTerminalSemantics::Terminal
                    },
                },
                source_turn_id: Some("turn-1".to_string()),
                payload: ParentDeliveryPayload::Progress(ProgressParentDeliveryPayload {
                    message: message.to_string(),
                }),
            }),
        }
    }

    fn sample_agent_context() -> AgentEventContext {
        AgentEventContext::root_execution("agent-root", "default")
    }

    fn record(event_id: &str, event: AgentEvent) -> SessionEventRecord {
        SessionEventRecord {
            event_id: event_id.to_string(),
            event,
        }
    }

    fn child_summary_json(latest_output_summary: &str) -> serde_json::Value {
        json!({
            "childSessionId": "session-child",
            "childAgentId": "agent-child",
            "title": "Repo inspector",
            "lifecycle": "running",
            "latestOutputSummary": latest_output_summary,
            "childRef": {
                "agentId": "agent-child",
                "sessionId": "session-root",
                "subRunId": "subrun-child",
                "parentAgentId": "agent-root",
                "parentSubRunId": "subrun-root",
                "lineageKind": "spawn",
                "status": "running",
                "openSessionId": "session-child"
            }
        })
    }
}
