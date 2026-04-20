use std::collections::{BTreeSet, HashMap};

use astrcode_client::{
    AstrcodeConversationBannerDto, AstrcodeConversationBlockDto, AstrcodeConversationBlockPatchDto,
    AstrcodeConversationBlockStatusDto, AstrcodeConversationChildSummaryDto,
    AstrcodeConversationControlStateDto, AstrcodeConversationCursorDto,
    AstrcodeConversationDeltaDto, AstrcodeConversationErrorEnvelopeDto,
    AstrcodeConversationSlashCandidateDto, AstrcodeConversationSnapshotResponseDto,
    AstrcodeConversationStreamEnvelopeDto, AstrcodePhaseDto, AstrcodeSessionListItem,
};

use super::{RenderState, TranscriptCell};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConversationState {
    pub sessions: Vec<AstrcodeSessionListItem>,
    pub active_session_id: Option<String>,
    pub active_session_title: Option<String>,
    pub cursor: Option<AstrcodeConversationCursorDto>,
    pub control: Option<AstrcodeConversationControlStateDto>,
    pub transcript: Vec<AstrcodeConversationBlockDto>,
    pub transcript_index: HashMap<String, usize>,
    pub child_summaries: Vec<AstrcodeConversationChildSummaryDto>,
    pub slash_candidates: Vec<AstrcodeConversationSlashCandidateDto>,
    pub banner: Option<AstrcodeConversationBannerDto>,
}

impl ConversationState {
    pub fn update_sessions(&mut self, sessions: Vec<AstrcodeSessionListItem>) {
        self.sessions = sessions;
    }

    pub fn activate_snapshot(
        &mut self,
        snapshot: AstrcodeConversationSnapshotResponseDto,
        render: &mut RenderState,
    ) {
        self.active_session_id = Some(snapshot.session_id);
        self.active_session_title = Some(snapshot.session_title);
        self.cursor = Some(snapshot.cursor);
        self.control = Some(snapshot.control);
        self.transcript = snapshot.blocks;
        self.rebuild_transcript_index();
        self.child_summaries = snapshot.child_summaries;
        self.slash_candidates = snapshot.slash_candidates;
        self.banner = snapshot.banner;
        render.mark_dirty();
    }

    pub fn apply_stream_envelope(
        &mut self,
        envelope: AstrcodeConversationStreamEnvelopeDto,
        render: &mut RenderState,
        expanded_ids: &BTreeSet<String>,
    ) -> bool {
        self.cursor = Some(envelope.cursor);
        self.apply_delta(envelope.delta, render, expanded_ids)
    }

    pub fn set_banner_error(&mut self, error: AstrcodeConversationErrorEnvelopeDto) {
        self.banner = Some(AstrcodeConversationBannerDto { error });
    }

    pub fn clear_banner(&mut self) {
        self.banner = None;
    }

    pub fn active_phase(&self) -> Option<AstrcodePhaseDto> {
        self.control.as_ref().map(|control| control.phase)
    }

    fn apply_delta(
        &mut self,
        delta: AstrcodeConversationDeltaDto,
        render: &mut RenderState,
        _expanded_ids: &BTreeSet<String>,
    ) -> bool {
        match delta {
            AstrcodeConversationDeltaDto::AppendBlock { block } => {
                self.transcript.push(block);
                if let Some(block) = self.transcript.last() {
                    self.transcript_index
                        .insert(block_id_of(block).to_string(), self.transcript.len() - 1);
                }
                render.mark_dirty();
                false
            },
            AstrcodeConversationDeltaDto::PatchBlock { block_id, patch } => {
                if let Some((index, block)) = self.find_block_mut(block_id.as_str()) {
                    let changed = apply_block_patch(block, patch);
                    let _ = index;
                    if changed {
                        render.mark_dirty();
                    }
                } else {
                    debug_missing_block("patch", block_id.as_str());
                }
                false
            },
            AstrcodeConversationDeltaDto::CompleteBlock { block_id, status } => {
                if let Some((index, block)) = self.find_block_mut(block_id.as_str()) {
                    let changed = set_block_status(block, status);
                    let _ = index;
                    if changed {
                        render.mark_dirty();
                    }
                } else {
                    debug_missing_block("complete", block_id.as_str());
                }
                false
            },
            AstrcodeConversationDeltaDto::UpdateControlState { control } => {
                if self.control.as_ref() != Some(&control) {
                    self.control = Some(control);
                    render.mark_dirty();
                }
                false
            },
            AstrcodeConversationDeltaDto::UpsertChildSummary { child } => {
                if let Some(existing) = self
                    .child_summaries
                    .iter_mut()
                    .find(|existing| existing.child_session_id == child.child_session_id)
                {
                    *existing = child;
                } else {
                    self.child_summaries.push(child);
                }
                false
            },
            AstrcodeConversationDeltaDto::RemoveChildSummary { child_session_id } => {
                self.child_summaries
                    .retain(|child| child.child_session_id != child_session_id);
                false
            },
            AstrcodeConversationDeltaDto::ReplaceSlashCandidates { candidates } => {
                self.slash_candidates = candidates;
                true
            },
            AstrcodeConversationDeltaDto::SetBanner { banner } => {
                if self.banner.as_ref() != Some(&banner) {
                    self.banner = Some(banner);
                    render.mark_dirty();
                }
                false
            },
            AstrcodeConversationDeltaDto::ClearBanner => {
                if self.banner.take().is_some() {
                    render.mark_dirty();
                }
                false
            },
            AstrcodeConversationDeltaDto::RehydrateRequired { error } => {
                self.set_banner_error(error);
                false
            },
        }
    }

    fn rebuild_transcript_index(&mut self) {
        self.transcript_index = self
            .transcript
            .iter()
            .enumerate()
            .map(|(index, block)| (block_id_of(block).to_string(), index))
            .collect();
    }

    fn find_block_mut(
        &mut self,
        block_id: &str,
    ) -> Option<(usize, &mut AstrcodeConversationBlockDto)> {
        let index = *self.transcript_index.get(block_id)?;
        self.transcript.get_mut(index).map(|block| (index, block))
    }

    pub fn project_transcript_cells(&self, expanded_ids: &BTreeSet<String>) -> Vec<TranscriptCell> {
        self.transcript
            .iter()
            .map(|block| TranscriptCell::from_block(block, expanded_ids))
            .collect()
    }

    pub fn project_transcript_cell(
        &self,
        index: usize,
        expanded_ids: &BTreeSet<String>,
    ) -> Option<TranscriptCell> {
        self.transcript
            .get(index)
            .map(|block| TranscriptCell::from_block(block, expanded_ids))
    }
}

fn block_id_of(block: &AstrcodeConversationBlockDto) -> &str {
    match block {
        AstrcodeConversationBlockDto::User(block) => &block.id,
        AstrcodeConversationBlockDto::Assistant(block) => &block.id,
        AstrcodeConversationBlockDto::Thinking(block) => &block.id,
        AstrcodeConversationBlockDto::Plan(block) => &block.id,
        AstrcodeConversationBlockDto::ToolCall(block) => &block.id,
        AstrcodeConversationBlockDto::Error(block) => &block.id,
        AstrcodeConversationBlockDto::SystemNote(block) => &block.id,
        AstrcodeConversationBlockDto::ChildHandoff(block) => &block.id,
    }
}

fn apply_block_patch(
    block: &mut AstrcodeConversationBlockDto,
    patch: AstrcodeConversationBlockPatchDto,
) -> bool {
    match patch {
        AstrcodeConversationBlockPatchDto::AppendMarkdown { markdown } => match block {
            AstrcodeConversationBlockDto::Assistant(block) => {
                normalize_markdown_append(&mut block.markdown, &markdown)
            },
            AstrcodeConversationBlockDto::Thinking(block) => {
                normalize_markdown_append(&mut block.markdown, &markdown)
            },
            AstrcodeConversationBlockDto::SystemNote(block) => {
                normalize_markdown_append(&mut block.markdown, &markdown)
            },
            AstrcodeConversationBlockDto::User(block) => {
                normalize_markdown_append(&mut block.markdown, &markdown)
            },
            AstrcodeConversationBlockDto::Plan(_) => false,
            AstrcodeConversationBlockDto::ToolCall(_)
            | AstrcodeConversationBlockDto::Error(_)
            | AstrcodeConversationBlockDto::ChildHandoff(_) => false,
        },
        AstrcodeConversationBlockPatchDto::ReplaceMarkdown { markdown } => match block {
            AstrcodeConversationBlockDto::Assistant(block) => {
                replace_if_changed(&mut block.markdown, markdown)
            },
            AstrcodeConversationBlockDto::Thinking(block) => {
                replace_if_changed(&mut block.markdown, markdown)
            },
            AstrcodeConversationBlockDto::SystemNote(block) => {
                replace_if_changed(&mut block.markdown, markdown)
            },
            AstrcodeConversationBlockDto::User(block) => {
                replace_if_changed(&mut block.markdown, markdown)
            },
            AstrcodeConversationBlockDto::Plan(_) => false,
            AstrcodeConversationBlockDto::ToolCall(_)
            | AstrcodeConversationBlockDto::Error(_)
            | AstrcodeConversationBlockDto::ChildHandoff(_) => false,
        },
        AstrcodeConversationBlockPatchDto::AppendToolStream { stream, chunk } => {
            if let AstrcodeConversationBlockDto::ToolCall(block) = block {
                if enum_wire_name(&stream).as_deref() == Some("stderr") {
                    if chunk.is_empty() {
                        return false;
                    }
                    block.streams.stderr.push_str(&chunk);
                } else {
                    if chunk.is_empty() {
                        return false;
                    }
                    block.streams.stdout.push_str(&chunk);
                }
                true
            } else {
                false
            }
        },
        AstrcodeConversationBlockPatchDto::ReplaceSummary { summary } => {
            if let AstrcodeConversationBlockDto::ToolCall(block) = block {
                replace_option_if_changed(&mut block.summary, summary)
            } else {
                false
            }
        },
        AstrcodeConversationBlockPatchDto::ReplaceMetadata { metadata } => {
            if let AstrcodeConversationBlockDto::ToolCall(block) = block {
                replace_option_if_changed(&mut block.metadata, metadata)
            } else {
                false
            }
        },
        AstrcodeConversationBlockPatchDto::ReplaceError { error } => {
            if let AstrcodeConversationBlockDto::ToolCall(block) = block {
                replace_if_changed(&mut block.error, error)
            } else {
                false
            }
        },
        AstrcodeConversationBlockPatchDto::ReplaceDuration { duration_ms } => {
            if let AstrcodeConversationBlockDto::ToolCall(block) = block {
                replace_option_if_changed(&mut block.duration_ms, duration_ms)
            } else {
                false
            }
        },
        AstrcodeConversationBlockPatchDto::ReplaceChildRef { child_ref } => {
            if let AstrcodeConversationBlockDto::ToolCall(block) = block {
                replace_option_if_changed(&mut block.child_ref, child_ref)
            } else {
                false
            }
        },
        AstrcodeConversationBlockPatchDto::SetTruncated { truncated } => {
            if let AstrcodeConversationBlockDto::ToolCall(block) = block {
                if block.truncated != truncated {
                    block.truncated = truncated;
                    true
                } else {
                    false
                }
            } else {
                false
            }
        },
        AstrcodeConversationBlockPatchDto::SetStatus { status } => set_block_status(block, status),
    }
}

fn normalize_markdown_append(current: &mut String, incoming: &str) -> bool {
    if incoming.is_empty() {
        return false;
    }

    if current.is_empty() {
        current.push_str(incoming);
        return true;
    }

    if incoming.starts_with(current.as_str()) {
        if current != incoming {
            *current = incoming.to_string();
            return true;
        }
        return false;
    }

    if current.ends_with(incoming) {
        return false;
    }

    if let Some(overlap) = longest_suffix_prefix_overlap(current.as_str(), incoming) {
        current.push_str(&incoming[overlap..]);
        return overlap < incoming.len();
    }

    current.push_str(incoming);
    true
}

fn longest_suffix_prefix_overlap(current: &str, incoming: &str) -> Option<usize> {
    let max_overlap = current.len().min(incoming.len());
    incoming
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(incoming.len()))
        .filter(|index| *index > 0 && *index <= max_overlap)
        .rev()
        .find(|index| current.ends_with(&incoming[..*index]))
}

fn enum_wire_name<T>(value: &T) -> Option<String>
where
    T: serde::Serialize,
{
    serde_json::to_value(value)
        .ok()?
        .as_str()
        .map(|value| value.trim().to_string())
}

#[cfg(debug_assertions)]
fn debug_missing_block(operation: &str, block_id: &str) {
    eprintln!("astrcode-cli: ignored {operation} delta for unknown block '{block_id}'");
}

#[cfg(not(debug_assertions))]
fn debug_missing_block(_operation: &str, _block_id: &str) {}

fn set_block_status(
    block: &mut AstrcodeConversationBlockDto,
    status: AstrcodeConversationBlockStatusDto,
) -> bool {
    match block {
        AstrcodeConversationBlockDto::Assistant(block) => {
            replace_if_changed(&mut block.status, status)
        },
        AstrcodeConversationBlockDto::Thinking(block) => {
            replace_if_changed(&mut block.status, status)
        },
        AstrcodeConversationBlockDto::Plan(_) => false,
        AstrcodeConversationBlockDto::ToolCall(block) => {
            replace_if_changed(&mut block.status, status)
        },
        AstrcodeConversationBlockDto::User(_)
        | AstrcodeConversationBlockDto::Error(_)
        | AstrcodeConversationBlockDto::SystemNote(_)
        | AstrcodeConversationBlockDto::ChildHandoff(_) => false,
    }
}

fn replace_if_changed<T: PartialEq>(slot: &mut T, next: T) -> bool {
    if *slot == next {
        false
    } else {
        *slot = next;
        true
    }
}

fn replace_option_if_changed<T: PartialEq>(slot: &mut Option<T>, next: T) -> bool {
    if slot.as_ref() == Some(&next) {
        false
    } else {
        *slot = Some(next);
        true
    }
}

#[cfg(test)]
mod tests {
    use astrcode_client::{
        AstrcodeConversationAssistantBlockDto, AstrcodeConversationBlockDto,
        AstrcodeConversationBlockPatchDto, AstrcodeConversationBlockStatusDto,
        AstrcodeConversationCursorDto, AstrcodeConversationDeltaDto,
        AstrcodeConversationStreamEnvelopeDto,
    };

    use super::{ConversationState, normalize_markdown_append};
    use crate::state::RenderState;

    #[test]
    fn append_markdown_replaces_with_cumulative_body() {
        let mut current = "你好".to_string();
        normalize_markdown_append(&mut current, "你好，世界");
        assert_eq!(current, "你好，世界");
    }

    #[test]
    fn append_markdown_ignores_replayed_suffix() {
        let mut current = "你好，世界".to_string();
        normalize_markdown_append(&mut current, "世界");
        assert_eq!(current, "你好，世界");
    }

    #[test]
    fn append_markdown_appends_only_non_overlapping_suffix() {
        let mut current = "你好，世".to_string();
        normalize_markdown_append(&mut current, "世界");
        assert_eq!(current, "你好，世界");
    }

    #[test]
    fn append_markdown_keeps_true_incremental_append() {
        let mut current = "你好".to_string();
        normalize_markdown_append(&mut current, "，世界");
        assert_eq!(current, "你好，世界");
    }

    #[test]
    fn duplicate_markdown_replay_does_not_mark_surface_dirty() {
        let mut conversation = ConversationState {
            transcript: vec![AstrcodeConversationBlockDto::Assistant(
                AstrcodeConversationAssistantBlockDto {
                    id: "assistant-1".to_string(),
                    turn_id: Some("turn-1".to_string()),
                    status: AstrcodeConversationBlockStatusDto::Streaming,
                    markdown: "你好，世界".to_string(),
                },
            )],
            transcript_index: [("assistant-1".to_string(), 0)].into_iter().collect(),
            ..Default::default()
        };
        let mut render = RenderState::default();
        render.take_frame_dirty();

        conversation.apply_stream_envelope(
            AstrcodeConversationStreamEnvelopeDto {
                session_id: "session-1".to_string(),
                cursor: AstrcodeConversationCursorDto("1.1".to_string()),
                delta: AstrcodeConversationDeltaDto::PatchBlock {
                    block_id: "assistant-1".to_string(),
                    patch: AstrcodeConversationBlockPatchDto::AppendMarkdown {
                        markdown: "世界".to_string(),
                    },
                },
            },
            &mut render,
            &Default::default(),
        );

        assert!(!render.take_frame_dirty());
    }
}
