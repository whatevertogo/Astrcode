use super::*;

mod plan_projection;

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

pub(super) fn block_id(block: &ConversationBlockFacts) -> &str {
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

pub(super) fn should_suppress_tool_call_block(tool_name: &str, _input: Option<&Value>) -> bool {
    matches!(tool_name, "upsertSessionPlan" | "exitPlanMode")
}

pub(super) fn plan_block_from_tool_result(
    turn_id: &str,
    result: &ToolExecutionResult,
) -> Option<ConversationPlanBlockFacts> {
    plan_projection::plan_block_from_tool_result(turn_id, result)
}

pub(super) fn tool_result_summary(result: &ToolExecutionResult) -> String {
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

pub(super) fn classify_transcript_error(message: &str) -> ConversationTranscriptErrorKind {
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
