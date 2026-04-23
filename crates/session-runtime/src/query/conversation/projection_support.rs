use std::collections::HashSet;

use super::*;

mod plan_projection;

impl ConversationStreamProjector {
    pub fn new(last_sent_cursor: Option<String>, facts: &ConversationStreamReplayFacts) -> Self {
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

    pub fn last_sent_cursor(&self) -> Option<&str> {
        self.last_sent_cursor.as_deref()
    }

    pub fn step_progress(&self) -> &ConversationStepProgressFacts {
        &self.step_progress
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
        ConversationDeltaFacts::AppendBlock { block } => Some(block_id(block.as_ref())),
        ConversationDeltaFacts::PatchBlock { block_id, .. }
        | ConversationDeltaFacts::CompleteBlock { block_id, .. } => Some(block_id.as_str()),
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
        ConversationBlockFacts::SystemNote(_) => None,
        ConversationBlockFacts::ChildHandoff(_) => None,
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
    if let ConversationDeltaFacts::AppendBlock { block } = delta {
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

impl ConversationStreamProjector {
    fn observe_durable_delta_step(&mut self, delta: &ConversationDeltaFacts) {
        observe_durable_delta_step(&mut self.step_progress, delta);
    }

    fn observe_live_event_step(&mut self, event: &AgentEvent) {
        let turn_id = match event {
            AgentEvent::ThinkingDelta { turn_id, .. }
            | AgentEvent::ModelDelta { turn_id, .. }
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
