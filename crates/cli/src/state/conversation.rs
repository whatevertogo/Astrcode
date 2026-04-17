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
        render.invalidate_transcript_cache();
    }

    pub fn apply_stream_envelope(
        &mut self,
        envelope: AstrcodeConversationStreamEnvelopeDto,
        render: &mut RenderState,
        expanded_ids: &BTreeSet<String>,
    ) {
        self.cursor = Some(envelope.cursor);
        self.apply_delta(envelope.delta, render, expanded_ids);
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
    ) {
        match delta {
            AstrcodeConversationDeltaDto::AppendBlock { block } => {
                self.transcript.push(block);
                if let Some(block) = self.transcript.last() {
                    self.transcript_index
                        .insert(block_id_of(block).to_string(), self.transcript.len() - 1);
                }
                render.invalidate_transcript_cache();
            },
            AstrcodeConversationDeltaDto::PatchBlock { block_id, patch } => {
                if let Some((index, block)) = self.find_block_mut(block_id.as_str()) {
                    apply_block_patch(block, patch);
                    let _ = index;
                    render.invalidate_transcript_cache();
                } else {
                    debug_missing_block("patch", block_id.as_str());
                }
            },
            AstrcodeConversationDeltaDto::CompleteBlock { block_id, status } => {
                if let Some((index, block)) = self.find_block_mut(block_id.as_str()) {
                    set_block_status(block, status);
                    let _ = index;
                    render.invalidate_transcript_cache();
                } else {
                    debug_missing_block("complete", block_id.as_str());
                }
            },
            AstrcodeConversationDeltaDto::UpdateControlState { control } => {
                self.control = Some(control);
                render.invalidate_transcript_cache();
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
            },
            AstrcodeConversationDeltaDto::RemoveChildSummary { child_session_id } => {
                self.child_summaries
                    .retain(|child| child.child_session_id != child_session_id);
            },
            AstrcodeConversationDeltaDto::ReplaceSlashCandidates { candidates } => {
                self.slash_candidates = candidates;
            },
            AstrcodeConversationDeltaDto::SetBanner { banner } => {
                self.banner = Some(banner);
                render.invalidate_transcript_cache();
            },
            AstrcodeConversationDeltaDto::ClearBanner => {
                self.banner = None;
                render.invalidate_transcript_cache();
            },
            AstrcodeConversationDeltaDto::RehydrateRequired { error } => {
                self.set_banner_error(error);
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
}

fn block_id_of(block: &AstrcodeConversationBlockDto) -> &str {
    match block {
        AstrcodeConversationBlockDto::User(block) => &block.id,
        AstrcodeConversationBlockDto::Assistant(block) => &block.id,
        AstrcodeConversationBlockDto::Thinking(block) => &block.id,
        AstrcodeConversationBlockDto::ToolCall(block) => &block.id,
        AstrcodeConversationBlockDto::Error(block) => &block.id,
        AstrcodeConversationBlockDto::SystemNote(block) => &block.id,
        AstrcodeConversationBlockDto::ChildHandoff(block) => &block.id,
    }
}

fn apply_block_patch(
    block: &mut AstrcodeConversationBlockDto,
    patch: AstrcodeConversationBlockPatchDto,
) {
    match patch {
        AstrcodeConversationBlockPatchDto::AppendMarkdown { markdown } => match block {
            AstrcodeConversationBlockDto::Assistant(block) => block.markdown.push_str(&markdown),
            AstrcodeConversationBlockDto::Thinking(block) => block.markdown.push_str(&markdown),
            AstrcodeConversationBlockDto::SystemNote(block) => block.markdown.push_str(&markdown),
            AstrcodeConversationBlockDto::User(block) => block.markdown.push_str(&markdown),
            AstrcodeConversationBlockDto::ToolCall(_)
            | AstrcodeConversationBlockDto::Error(_)
            | AstrcodeConversationBlockDto::ChildHandoff(_) => {},
        },
        AstrcodeConversationBlockPatchDto::ReplaceMarkdown { markdown } => match block {
            AstrcodeConversationBlockDto::Assistant(block) => block.markdown = markdown,
            AstrcodeConversationBlockDto::Thinking(block) => block.markdown = markdown,
            AstrcodeConversationBlockDto::SystemNote(block) => block.markdown = markdown,
            AstrcodeConversationBlockDto::User(block) => block.markdown = markdown,
            AstrcodeConversationBlockDto::ToolCall(_)
            | AstrcodeConversationBlockDto::Error(_)
            | AstrcodeConversationBlockDto::ChildHandoff(_) => {},
        },
        AstrcodeConversationBlockPatchDto::AppendToolStream { stream, chunk } => {
            if let AstrcodeConversationBlockDto::ToolCall(block) = block {
                if enum_wire_name(&stream).as_deref() == Some("stderr") {
                    block.streams.stderr.push_str(&chunk);
                } else {
                    block.streams.stdout.push_str(&chunk);
                }
            }
        },
        AstrcodeConversationBlockPatchDto::ReplaceSummary { summary } => {
            if let AstrcodeConversationBlockDto::ToolCall(block) = block {
                block.summary = Some(summary);
            }
        },
        AstrcodeConversationBlockPatchDto::ReplaceMetadata { metadata } => {
            if let AstrcodeConversationBlockDto::ToolCall(block) = block {
                block.metadata = Some(metadata);
            }
        },
        AstrcodeConversationBlockPatchDto::ReplaceError { error } => {
            if let AstrcodeConversationBlockDto::ToolCall(block) = block {
                block.error = error;
            }
        },
        AstrcodeConversationBlockPatchDto::ReplaceDuration { duration_ms } => {
            if let AstrcodeConversationBlockDto::ToolCall(block) = block {
                block.duration_ms = Some(duration_ms);
            }
        },
        AstrcodeConversationBlockPatchDto::ReplaceChildRef { child_ref } => {
            if let AstrcodeConversationBlockDto::ToolCall(block) = block {
                block.child_ref = Some(child_ref);
            }
        },
        AstrcodeConversationBlockPatchDto::SetTruncated { truncated } => {
            if let AstrcodeConversationBlockDto::ToolCall(block) = block {
                block.truncated = truncated;
            }
        },
        AstrcodeConversationBlockPatchDto::SetStatus { status } => set_block_status(block, status),
    }
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

fn debug_missing_block(operation: &str, block_id: &str) {
    #[cfg(debug_assertions)]
    eprintln!("astrcode-cli: ignored {operation} delta for unknown block '{block_id}'");
}

fn set_block_status(
    block: &mut AstrcodeConversationBlockDto,
    status: AstrcodeConversationBlockStatusDto,
) {
    match block {
        AstrcodeConversationBlockDto::Assistant(block) => block.status = status,
        AstrcodeConversationBlockDto::Thinking(block) => block.status = status,
        AstrcodeConversationBlockDto::ToolCall(block) => block.status = status,
        AstrcodeConversationBlockDto::User(_)
        | AstrcodeConversationBlockDto::Error(_)
        | AstrcodeConversationBlockDto::SystemNote(_)
        | AstrcodeConversationBlockDto::ChildHandoff(_) => {},
    }
}
