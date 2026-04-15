use astrcode_client::{
    AstrcodeConversationBannerDto, AstrcodeConversationBlockDto, AstrcodeConversationBlockPatchDto,
    AstrcodeConversationBlockStatusDto, AstrcodeConversationChildSummaryDto,
    AstrcodeConversationControlStateDto, AstrcodeConversationCursorDto,
    AstrcodeConversationDeltaDto, AstrcodeConversationErrorEnvelopeDto,
    AstrcodeConversationSlashCandidateDto, AstrcodeConversationSnapshotResponseDto,
    AstrcodeConversationStreamEnvelopeDto, AstrcodePhaseDto, AstrcodeSessionListItem,
};

use super::{ChildPaneState, RenderState, TranscriptCell};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConversationState {
    pub sessions: Vec<AstrcodeSessionListItem>,
    pub active_session_id: Option<String>,
    pub active_session_title: Option<String>,
    pub cursor: Option<AstrcodeConversationCursorDto>,
    pub control: Option<AstrcodeConversationControlStateDto>,
    pub transcript: Vec<AstrcodeConversationBlockDto>,
    pub transcript_cells: Vec<TranscriptCell>,
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
        self.rebuild_transcript_cells();
        self.child_summaries = snapshot.child_summaries;
        self.slash_candidates = snapshot.slash_candidates;
        self.banner = snapshot.banner;
        render.invalidate_transcript_cache();
    }

    pub fn apply_stream_envelope(
        &mut self,
        envelope: AstrcodeConversationStreamEnvelopeDto,
        render: &mut RenderState,
        child_pane: &mut ChildPaneState,
    ) {
        self.cursor = Some(envelope.cursor);
        self.apply_delta(envelope.delta, render, child_pane);
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

    pub fn selected_child_summary(
        &self,
        child_pane: &ChildPaneState,
    ) -> Option<&AstrcodeConversationChildSummaryDto> {
        self.child_summaries.get(child_pane.selected)
    }

    pub fn focused_child_summary(
        &self,
        child_pane: &ChildPaneState,
    ) -> Option<&AstrcodeConversationChildSummaryDto> {
        let Some(child_session_id) = child_pane.focused_child_session_id.as_deref() else {
            return self.selected_child_summary(child_pane);
        };
        self.child_summaries
            .iter()
            .find(|summary| summary.child_session_id == child_session_id)
    }

    fn apply_delta(
        &mut self,
        delta: AstrcodeConversationDeltaDto,
        render: &mut RenderState,
        child_pane: &mut ChildPaneState,
    ) {
        match delta {
            AstrcodeConversationDeltaDto::AppendBlock { block } => {
                self.transcript_cells
                    .push(TranscriptCell::from_block(&block));
                self.transcript.push(block);
                render.invalidate_transcript_cache();
            },
            AstrcodeConversationDeltaDto::PatchBlock { block_id, patch } => {
                if let Some((index, block)) = self
                    .transcript
                    .iter_mut()
                    .enumerate()
                    .find(|(_, block)| block_id_of(block) == block_id)
                {
                    apply_block_patch(block, patch);
                    self.transcript_cells[index] = TranscriptCell::from_block(block);
                    render.invalidate_transcript_cache();
                }
            },
            AstrcodeConversationDeltaDto::CompleteBlock { block_id, status } => {
                if let Some((index, block)) = self
                    .transcript
                    .iter_mut()
                    .enumerate()
                    .find(|(_, block)| block_id_of(block) == block_id)
                {
                    set_block_status(block, status);
                    self.transcript_cells[index] = TranscriptCell::from_block(block);
                    render.invalidate_transcript_cache();
                }
            },
            AstrcodeConversationDeltaDto::UpdateControlState { control } => {
                self.control = Some(control);
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
                if child_pane.selected >= self.child_summaries.len()
                    && !self.child_summaries.is_empty()
                {
                    child_pane.selected = self.child_summaries.len() - 1;
                }
            },
            AstrcodeConversationDeltaDto::RemoveChildSummary { child_session_id } => {
                self.child_summaries
                    .retain(|child| child.child_session_id != child_session_id);
                if child_pane.selected >= self.child_summaries.len() {
                    child_pane.selected = self.child_summaries.len().saturating_sub(1);
                }
                if child_pane.focused_child_session_id.as_deref() == Some(child_session_id.as_str())
                {
                    child_pane.focused_child_session_id = None;
                }
            },
            AstrcodeConversationDeltaDto::ReplaceSlashCandidates { candidates } => {
                self.slash_candidates = candidates;
            },
            AstrcodeConversationDeltaDto::SetBanner { banner } => {
                self.banner = Some(banner);
            },
            AstrcodeConversationDeltaDto::ClearBanner => {
                self.banner = None;
            },
            AstrcodeConversationDeltaDto::RehydrateRequired { error } => {
                self.set_banner_error(error);
            },
        }
    }

    fn rebuild_transcript_cells(&mut self) {
        self.transcript_cells = self
            .transcript
            .iter()
            .map(TranscriptCell::from_block)
            .collect();
    }
}

fn block_id_of(block: &AstrcodeConversationBlockDto) -> &str {
    match block {
        AstrcodeConversationBlockDto::User(block) => &block.id,
        AstrcodeConversationBlockDto::Assistant(block) => &block.id,
        AstrcodeConversationBlockDto::Thinking(block) => &block.id,
        AstrcodeConversationBlockDto::ToolCall(block) => &block.id,
        AstrcodeConversationBlockDto::ToolStream(block) => &block.id,
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
            AstrcodeConversationBlockDto::ToolStream(block) => block.content.push_str(&markdown),
            AstrcodeConversationBlockDto::ToolCall(_)
            | AstrcodeConversationBlockDto::Error(_)
            | AstrcodeConversationBlockDto::ChildHandoff(_) => {},
        },
        AstrcodeConversationBlockPatchDto::ReplaceMarkdown { markdown } => match block {
            AstrcodeConversationBlockDto::Assistant(block) => block.markdown = markdown,
            AstrcodeConversationBlockDto::Thinking(block) => block.markdown = markdown,
            AstrcodeConversationBlockDto::SystemNote(block) => block.markdown = markdown,
            AstrcodeConversationBlockDto::User(block) => block.markdown = markdown,
            AstrcodeConversationBlockDto::ToolStream(block) => block.content = markdown,
            AstrcodeConversationBlockDto::ToolCall(_)
            | AstrcodeConversationBlockDto::Error(_)
            | AstrcodeConversationBlockDto::ChildHandoff(_) => {},
        },
        AstrcodeConversationBlockPatchDto::AppendToolStream { chunk, .. } => {
            if let AstrcodeConversationBlockDto::ToolStream(block) = block {
                block.content.push_str(&chunk);
            }
        },
        AstrcodeConversationBlockPatchDto::ReplaceSummary { summary } => {
            if let AstrcodeConversationBlockDto::ToolCall(block) = block {
                block.summary = Some(summary);
            }
        },
        AstrcodeConversationBlockPatchDto::SetStatus { status } => set_block_status(block, status),
    }
}

fn set_block_status(
    block: &mut AstrcodeConversationBlockDto,
    status: AstrcodeConversationBlockStatusDto,
) {
    match block {
        AstrcodeConversationBlockDto::Assistant(block) => block.status = status,
        AstrcodeConversationBlockDto::Thinking(block) => block.status = status,
        AstrcodeConversationBlockDto::ToolCall(block) => block.status = status,
        AstrcodeConversationBlockDto::ToolStream(block) => block.status = status,
        AstrcodeConversationBlockDto::User(_)
        | AstrcodeConversationBlockDto::Error(_)
        | AstrcodeConversationBlockDto::SystemNote(_)
        | AstrcodeConversationBlockDto::ChildHandoff(_) => {},
    }
}
