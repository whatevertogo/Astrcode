use std::{path::PathBuf, time::Duration};

use astrcode_client::{
    AstrcodePhaseDto, AstrcodeSessionListItem, AstrcodeTerminalBannerDto, AstrcodeTerminalBlockDto,
    AstrcodeTerminalBlockPatchDto, AstrcodeTerminalBlockStatusDto, AstrcodeTerminalChildSummaryDto,
    AstrcodeTerminalControlStateDto, AstrcodeTerminalCursorDto, AstrcodeTerminalDeltaDto,
    AstrcodeTerminalErrorEnvelopeDto, AstrcodeTerminalSlashCandidateDto,
    AstrcodeTerminalSnapshotResponseDto, AstrcodeTerminalStreamEnvelopeDto,
};

use crate::capability::TerminalCapabilities;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PaneFocus {
    Transcript,
    ChildPane,
    #[default]
    Composer,
    Overlay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StreamRenderMode {
    #[default]
    Smooth,
    CatchUp,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ComposerState {
    pub input: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResumeOverlayState {
    pub query: String,
    pub items: Vec<AstrcodeSessionListItem>,
    pub selected: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SlashPaletteState {
    pub query: String,
    pub items: Vec<AstrcodeTerminalSlashCandidateDto>,
    pub selected: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum OverlayState {
    #[default]
    None,
    Resume(ResumeOverlayState),
    SlashPalette(SlashPaletteState),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverlaySelection {
    ResumeSession(String),
    SlashCandidate(AstrcodeTerminalSlashCandidateDto),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusLine {
    pub message: String,
    pub is_error: bool,
}

impl Default for StatusLine {
    fn default() -> Self {
        Self {
            message: "ready".to_string(),
            is_error: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChildPaneState {
    pub selected: usize,
    pub focused_child_session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrappedLine {
    pub style: WrappedLineStyle,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WrappedLineStyle {
    Plain,
    Muted,
    Accent,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TranscriptRenderCache {
    pub width: u16,
    pub revision: u64,
    pub lines: Vec<WrappedLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RenderState {
    pub viewport_width: u16,
    pub viewport_height: u16,
    pub transcript_revision: u64,
    pub wrap_cache_revision: u64,
    pub transcript_cache: TranscriptRenderCache,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamBackpressureState {
    pub mode: StreamRenderMode,
    pub pending_chunks: usize,
    pub oldest_chunk_age: Duration,
}

impl Default for StreamBackpressureState {
    fn default() -> Self {
        Self {
            mode: StreamRenderMode::Smooth,
            pending_chunks: 0,
            oldest_chunk_age: Duration::ZERO,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliState {
    pub connection_origin: String,
    pub working_dir: Option<PathBuf>,
    pub capabilities: TerminalCapabilities,
    pub sessions: Vec<AstrcodeSessionListItem>,
    pub active_session_id: Option<String>,
    pub active_session_title: Option<String>,
    pub cursor: Option<AstrcodeTerminalCursorDto>,
    pub control: Option<AstrcodeTerminalControlStateDto>,
    pub transcript: Vec<AstrcodeTerminalBlockDto>,
    pub child_summaries: Vec<AstrcodeTerminalChildSummaryDto>,
    pub slash_candidates: Vec<AstrcodeTerminalSlashCandidateDto>,
    pub banner: Option<AstrcodeTerminalBannerDto>,
    pub status: StatusLine,
    pub scroll_anchor: u16,
    pub pane_focus: PaneFocus,
    pub composer: ComposerState,
    pub overlay: OverlayState,
    pub child_pane: ChildPaneState,
    pub render: RenderState,
    pub stream: StreamBackpressureState,
}

impl Default for CliState {
    fn default() -> Self {
        Self {
            connection_origin: String::new(),
            working_dir: None,
            capabilities: TerminalCapabilities::detect(),
            sessions: Vec::new(),
            active_session_id: None,
            active_session_title: None,
            cursor: None,
            control: None,
            transcript: Vec::new(),
            child_summaries: Vec::new(),
            slash_candidates: Vec::new(),
            banner: None,
            status: StatusLine::default(),
            scroll_anchor: 0,
            pane_focus: PaneFocus::Composer,
            composer: ComposerState::default(),
            overlay: OverlayState::None,
            child_pane: ChildPaneState::default(),
            render: RenderState::default(),
            stream: StreamBackpressureState::default(),
        }
    }
}

impl CliState {
    pub fn new(
        connection_origin: String,
        working_dir: Option<PathBuf>,
        capabilities: TerminalCapabilities,
    ) -> Self {
        Self {
            connection_origin,
            working_dir,
            capabilities,
            ..Default::default()
        }
    }

    pub fn set_status(&mut self, message: impl Into<String>) {
        self.status = StatusLine {
            message: message.into(),
            is_error: false,
        };
    }

    pub fn set_error_status(&mut self, message: impl Into<String>) {
        self.status = StatusLine {
            message: message.into(),
            is_error: true,
        };
    }

    pub fn set_stream_mode(&mut self, mode: StreamRenderMode, pending: usize, oldest: Duration) {
        self.stream.mode = mode;
        self.stream.pending_chunks = pending;
        self.stream.oldest_chunk_age = oldest;
    }

    pub fn set_viewport_size(&mut self, width: u16, height: u16) {
        if self.render.viewport_width == width && self.render.viewport_height == height {
            return;
        }
        self.render.viewport_width = width;
        self.render.viewport_height = height;
        self.render.wrap_cache_revision = self.render.wrap_cache_revision.saturating_add(1);
        self.render.transcript_cache = TranscriptRenderCache::default();
        self.scroll_anchor = 0;
    }

    pub fn update_transcript_cache(&mut self, width: u16, lines: Vec<WrappedLine>) {
        self.render.transcript_cache = TranscriptRenderCache {
            width,
            revision: self.render.transcript_revision,
            lines,
        };
    }

    pub fn push_input(&mut self, ch: char) {
        self.composer.input.push(ch);
    }

    pub fn pop_input(&mut self) {
        self.composer.input.pop();
    }

    pub fn replace_input(&mut self, input: impl Into<String>) {
        self.composer.input = input.into();
    }

    pub fn take_input(&mut self) -> String {
        std::mem::take(&mut self.composer.input)
    }

    pub fn scroll_up(&mut self) {
        self.scroll_anchor = self.scroll_anchor.saturating_sub(1);
    }

    pub fn scroll_down(&mut self) {
        self.scroll_anchor = self.scroll_anchor.saturating_add(1);
    }

    pub fn cycle_focus_forward(&mut self) {
        self.pane_focus = match self.pane_focus {
            PaneFocus::Transcript => {
                if self.child_summaries.is_empty() {
                    PaneFocus::Composer
                } else {
                    PaneFocus::ChildPane
                }
            },
            PaneFocus::ChildPane => PaneFocus::Composer,
            PaneFocus::Composer => PaneFocus::Transcript,
            PaneFocus::Overlay => PaneFocus::Overlay,
        };
    }

    pub fn cycle_focus_backward(&mut self) {
        self.pane_focus = match self.pane_focus {
            PaneFocus::Transcript => PaneFocus::Composer,
            PaneFocus::ChildPane => PaneFocus::Transcript,
            PaneFocus::Composer => {
                if self.child_summaries.is_empty() {
                    PaneFocus::Transcript
                } else {
                    PaneFocus::ChildPane
                }
            },
            PaneFocus::Overlay => PaneFocus::Overlay,
        };
    }

    pub fn child_next(&mut self) {
        if self.child_summaries.is_empty() {
            return;
        }
        self.child_pane.selected = (self.child_pane.selected + 1) % self.child_summaries.len();
    }

    pub fn child_prev(&mut self) {
        if self.child_summaries.is_empty() {
            return;
        }
        self.child_pane.selected = (self.child_pane.selected + self.child_summaries.len() - 1)
            % self.child_summaries.len();
    }

    pub fn toggle_child_focus(&mut self) {
        let Some(selected) = self.selected_child_summary() else {
            return;
        };
        if self.child_pane.focused_child_session_id.as_deref()
            == Some(selected.child_session_id.as_str())
        {
            self.child_pane.focused_child_session_id = None;
        } else {
            self.child_pane.focused_child_session_id = Some(selected.child_session_id.clone());
        }
    }

    pub fn selected_child_summary(&self) -> Option<&AstrcodeTerminalChildSummaryDto> {
        self.child_summaries.get(self.child_pane.selected)
    }

    pub fn focused_child_summary(&self) -> Option<&AstrcodeTerminalChildSummaryDto> {
        let Some(child_session_id) = self.child_pane.focused_child_session_id.as_deref() else {
            return self.selected_child_summary();
        };
        self.child_summaries
            .iter()
            .find(|summary| summary.child_session_id == child_session_id)
    }

    pub fn update_sessions(&mut self, sessions: Vec<AstrcodeSessionListItem>) {
        self.sessions = sessions;
        if let OverlayState::Resume(resume) = &mut self.overlay {
            resume.items = self.sessions.clone();
            if resume.selected >= resume.items.len() {
                resume.selected = 0;
            }
        }
    }

    pub fn set_resume_query(
        &mut self,
        query: impl Into<String>,
        items: Vec<AstrcodeSessionListItem>,
    ) {
        self.pane_focus = PaneFocus::Overlay;
        self.overlay = OverlayState::Resume(ResumeOverlayState {
            query: query.into(),
            items,
            selected: 0,
        });
    }

    pub fn set_slash_query(
        &mut self,
        query: impl Into<String>,
        items: Vec<AstrcodeTerminalSlashCandidateDto>,
    ) {
        self.pane_focus = PaneFocus::Overlay;
        self.overlay = OverlayState::SlashPalette(SlashPaletteState {
            query: query.into(),
            items,
            selected: 0,
        });
    }

    pub fn overlay_query_push(&mut self, ch: char) {
        match &mut self.overlay {
            OverlayState::Resume(resume) => resume.query.push(ch),
            OverlayState::SlashPalette(palette) => palette.query.push(ch),
            OverlayState::None => self.push_input(ch),
        }
    }

    pub fn overlay_query_pop(&mut self) {
        match &mut self.overlay {
            OverlayState::Resume(resume) => {
                resume.query.pop();
            },
            OverlayState::SlashPalette(palette) => {
                palette.query.pop();
            },
            OverlayState::None => self.pop_input(),
        }
    }

    pub fn close_overlay(&mut self) {
        self.overlay = OverlayState::None;
        self.pane_focus = PaneFocus::Composer;
    }

    pub fn overlay_next(&mut self) {
        match &mut self.overlay {
            OverlayState::Resume(resume) if !resume.items.is_empty() => {
                resume.selected = (resume.selected + 1) % resume.items.len();
            },
            OverlayState::SlashPalette(palette) if !palette.items.is_empty() => {
                palette.selected = (palette.selected + 1) % palette.items.len();
            },
            _ => {},
        }
    }

    pub fn overlay_prev(&mut self) {
        match &mut self.overlay {
            OverlayState::Resume(resume) if !resume.items.is_empty() => {
                resume.selected =
                    (resume.selected + resume.items.len().saturating_sub(1)) % resume.items.len();
            },
            OverlayState::SlashPalette(palette) if !palette.items.is_empty() => {
                palette.selected = (palette.selected + palette.items.len().saturating_sub(1))
                    % palette.items.len();
            },
            _ => {},
        }
    }

    pub fn selected_overlay(&self) -> Option<OverlaySelection> {
        match &self.overlay {
            OverlayState::Resume(resume) => resume
                .items
                .get(resume.selected)
                .map(|item| OverlaySelection::ResumeSession(item.session_id.clone())),
            OverlayState::SlashPalette(palette) => palette
                .items
                .get(palette.selected)
                .cloned()
                .map(OverlaySelection::SlashCandidate),
            OverlayState::None => None,
        }
    }

    pub fn activate_snapshot(&mut self, snapshot: AstrcodeTerminalSnapshotResponseDto) {
        self.active_session_id = Some(snapshot.session_id.clone());
        self.active_session_title = Some(snapshot.session_title);
        self.cursor = Some(snapshot.cursor);
        self.control = Some(snapshot.control);
        self.transcript = snapshot.blocks;
        self.child_summaries = snapshot.child_summaries;
        self.slash_candidates = snapshot.slash_candidates;
        self.banner = snapshot.banner;
        self.scroll_anchor = 0;
        self.overlay = OverlayState::None;
        self.pane_focus = PaneFocus::Composer;
        self.child_pane.selected = 0;
        self.child_pane.focused_child_session_id = None;
        self.bump_transcript_revision();
    }

    pub fn apply_stream_envelope(&mut self, envelope: AstrcodeTerminalStreamEnvelopeDto) {
        self.cursor = Some(envelope.cursor);
        self.apply_delta(envelope.delta);
    }

    pub fn set_banner_error(&mut self, error: AstrcodeTerminalErrorEnvelopeDto) {
        self.banner = Some(AstrcodeTerminalBannerDto { error });
        self.pane_focus = PaneFocus::Composer;
    }

    pub fn clear_banner(&mut self) {
        self.banner = None;
    }

    pub fn active_phase(&self) -> Option<AstrcodePhaseDto> {
        self.control.as_ref().map(|control| control.phase)
    }

    fn bump_transcript_revision(&mut self) {
        self.render.transcript_revision = self.render.transcript_revision.saturating_add(1);
        self.render.transcript_cache = TranscriptRenderCache::default();
    }

    fn apply_delta(&mut self, delta: AstrcodeTerminalDeltaDto) {
        match delta {
            AstrcodeTerminalDeltaDto::AppendBlock { block } => {
                self.transcript.push(block);
                self.bump_transcript_revision();
            },
            AstrcodeTerminalDeltaDto::PatchBlock { block_id, patch } => {
                if let Some(block) = self
                    .transcript
                    .iter_mut()
                    .find(|block| block_id_of(block) == block_id)
                {
                    apply_block_patch(block, patch);
                    self.bump_transcript_revision();
                }
            },
            AstrcodeTerminalDeltaDto::CompleteBlock { block_id, status } => {
                if let Some(block) = self
                    .transcript
                    .iter_mut()
                    .find(|block| block_id_of(block) == block_id)
                {
                    set_block_status(block, status);
                    self.bump_transcript_revision();
                }
            },
            AstrcodeTerminalDeltaDto::UpdateControlState { control } => {
                self.control = Some(control);
            },
            AstrcodeTerminalDeltaDto::UpsertChildSummary { child } => {
                if let Some(existing) = self
                    .child_summaries
                    .iter_mut()
                    .find(|existing| existing.child_session_id == child.child_session_id)
                {
                    *existing = child;
                } else {
                    self.child_summaries.push(child);
                }
                if self.child_pane.selected >= self.child_summaries.len()
                    && !self.child_summaries.is_empty()
                {
                    self.child_pane.selected = self.child_summaries.len() - 1;
                }
            },
            AstrcodeTerminalDeltaDto::RemoveChildSummary { child_session_id } => {
                self.child_summaries
                    .retain(|child| child.child_session_id != child_session_id);
                if self.child_pane.selected >= self.child_summaries.len() {
                    self.child_pane.selected = self.child_summaries.len().saturating_sub(1);
                }
                if self.child_pane.focused_child_session_id.as_deref()
                    == Some(child_session_id.as_str())
                {
                    self.child_pane.focused_child_session_id = None;
                }
            },
            AstrcodeTerminalDeltaDto::ReplaceSlashCandidates { candidates } => {
                self.slash_candidates = candidates.clone();
                if let OverlayState::SlashPalette(palette) = &mut self.overlay {
                    palette.items = candidates;
                    if palette.selected >= palette.items.len() {
                        palette.selected = 0;
                    }
                }
            },
            AstrcodeTerminalDeltaDto::SetBanner { banner } => {
                self.banner = Some(banner);
            },
            AstrcodeTerminalDeltaDto::ClearBanner => self.banner = None,
            AstrcodeTerminalDeltaDto::RehydrateRequired { error } => self.set_banner_error(error),
        }
    }
}

fn block_id_of(block: &AstrcodeTerminalBlockDto) -> &str {
    match block {
        AstrcodeTerminalBlockDto::User(block) => &block.id,
        AstrcodeTerminalBlockDto::Assistant(block) => &block.id,
        AstrcodeTerminalBlockDto::Thinking(block) => &block.id,
        AstrcodeTerminalBlockDto::ToolCall(block) => &block.id,
        AstrcodeTerminalBlockDto::ToolStream(block) => &block.id,
        AstrcodeTerminalBlockDto::Error(block) => &block.id,
        AstrcodeTerminalBlockDto::SystemNote(block) => &block.id,
        AstrcodeTerminalBlockDto::ChildHandoff(block) => &block.id,
    }
}

fn apply_block_patch(block: &mut AstrcodeTerminalBlockDto, patch: AstrcodeTerminalBlockPatchDto) {
    match patch {
        AstrcodeTerminalBlockPatchDto::AppendMarkdown { markdown } => match block {
            AstrcodeTerminalBlockDto::Assistant(block) => block.markdown.push_str(&markdown),
            AstrcodeTerminalBlockDto::Thinking(block) => block.markdown.push_str(&markdown),
            AstrcodeTerminalBlockDto::SystemNote(block) => block.markdown.push_str(&markdown),
            AstrcodeTerminalBlockDto::User(block) => block.markdown.push_str(&markdown),
            AstrcodeTerminalBlockDto::ToolStream(block) => block.content.push_str(&markdown),
            AstrcodeTerminalBlockDto::ToolCall(_)
            | AstrcodeTerminalBlockDto::Error(_)
            | AstrcodeTerminalBlockDto::ChildHandoff(_) => {},
        },
        AstrcodeTerminalBlockPatchDto::ReplaceMarkdown { markdown } => match block {
            AstrcodeTerminalBlockDto::Assistant(block) => block.markdown = markdown,
            AstrcodeTerminalBlockDto::Thinking(block) => block.markdown = markdown,
            AstrcodeTerminalBlockDto::SystemNote(block) => block.markdown = markdown,
            AstrcodeTerminalBlockDto::User(block) => block.markdown = markdown,
            AstrcodeTerminalBlockDto::ToolStream(block) => block.content = markdown,
            AstrcodeTerminalBlockDto::ToolCall(_)
            | AstrcodeTerminalBlockDto::Error(_)
            | AstrcodeTerminalBlockDto::ChildHandoff(_) => {},
        },
        AstrcodeTerminalBlockPatchDto::AppendToolStream { chunk, .. } => {
            if let AstrcodeTerminalBlockDto::ToolStream(block) = block {
                block.content.push_str(&chunk);
            }
        },
        AstrcodeTerminalBlockPatchDto::ReplaceSummary { summary } => {
            if let AstrcodeTerminalBlockDto::ToolCall(block) = block {
                block.summary = Some(summary);
            }
        },
        AstrcodeTerminalBlockPatchDto::SetStatus { status } => set_block_status(block, status),
    }
}

fn set_block_status(block: &mut AstrcodeTerminalBlockDto, status: AstrcodeTerminalBlockStatusDto) {
    match block {
        AstrcodeTerminalBlockDto::Assistant(block) => block.status = status,
        AstrcodeTerminalBlockDto::Thinking(block) => block.status = status,
        AstrcodeTerminalBlockDto::ToolCall(block) => block.status = status,
        AstrcodeTerminalBlockDto::ToolStream(block) => block.status = status,
        AstrcodeTerminalBlockDto::User(_)
        | AstrcodeTerminalBlockDto::Error(_)
        | AstrcodeTerminalBlockDto::SystemNote(_)
        | AstrcodeTerminalBlockDto::ChildHandoff(_) => {},
    }
}

#[cfg(test)]
mod tests {
    use astrcode_client::{
        AstrcodeConversationCursorDto, AstrcodeTerminalAssistantBlockDto,
        AstrcodeTerminalSlashActionKindDto, AstrcodeTerminalSlashCandidateDto,
    };

    use super::*;
    use crate::capability::{ColorLevel, GlyphMode, TerminalCapabilities};

    fn sample_snapshot() -> AstrcodeTerminalSnapshotResponseDto {
        AstrcodeTerminalSnapshotResponseDto {
            session_id: "session-1".to_string(),
            session_title: "Session 1".to_string(),
            cursor: AstrcodeConversationCursorDto("1.2".to_string()),
            phase: AstrcodePhaseDto::Idle,
            control: AstrcodeTerminalControlStateDto {
                phase: AstrcodePhaseDto::Idle,
                can_submit_prompt: true,
                can_request_compact: true,
                compact_pending: false,
                active_turn_id: None,
            },
            blocks: vec![AstrcodeTerminalBlockDto::Assistant(
                AstrcodeTerminalAssistantBlockDto {
                    id: "assistant-1".to_string(),
                    turn_id: Some("turn-1".to_string()),
                    status: AstrcodeTerminalBlockStatusDto::Streaming,
                    markdown: "hello".to_string(),
                },
            )],
            child_summaries: Vec::new(),
            slash_candidates: vec![AstrcodeTerminalSlashCandidateDto {
                id: "skill-review".to_string(),
                title: "Review".to_string(),
                description: "review skill".to_string(),
                keywords: vec!["review".to_string()],
                action_kind: AstrcodeTerminalSlashActionKindDto::InsertText,
                action_value: "/skill review".to_string(),
            }],
            banner: None,
        }
    }

    fn capabilities() -> TerminalCapabilities {
        TerminalCapabilities {
            color: ColorLevel::TrueColor,
            glyphs: GlyphMode::Unicode,
            alt_screen: true,
            mouse: true,
            bracketed_paste: true,
        }
    }

    #[test]
    fn applies_snapshot_and_stream_deltas() {
        let mut state = CliState::new("http://127.0.0.1:5529".to_string(), None, capabilities());
        state.activate_snapshot(sample_snapshot());
        state.apply_stream_envelope(AstrcodeTerminalStreamEnvelopeDto {
            session_id: "session-1".to_string(),
            cursor: AstrcodeConversationCursorDto("1.3".to_string()),
            delta: AstrcodeTerminalDeltaDto::PatchBlock {
                block_id: "assistant-1".to_string(),
                patch: AstrcodeTerminalBlockPatchDto::AppendMarkdown {
                    markdown: " world".to_string(),
                },
            },
        });

        let AstrcodeTerminalBlockDto::Assistant(block) = &state.transcript[0] else {
            panic!("assistant block should remain present");
        };
        assert_eq!(block.markdown, "hello world");
        assert_eq!(
            state.cursor.as_ref().map(|cursor| cursor.0.as_str()),
            Some("1.3")
        );
    }

    #[test]
    fn replace_markdown_patch_overwrites_streamed_content() {
        let mut state = CliState::new("http://127.0.0.1:5529".to_string(), None, capabilities());
        state.activate_snapshot(sample_snapshot());
        state.apply_stream_envelope(AstrcodeTerminalStreamEnvelopeDto {
            session_id: "session-1".to_string(),
            cursor: AstrcodeConversationCursorDto("1.4".to_string()),
            delta: AstrcodeTerminalDeltaDto::PatchBlock {
                block_id: "assistant-1".to_string(),
                patch: AstrcodeTerminalBlockPatchDto::ReplaceMarkdown {
                    markdown: "replaced".to_string(),
                },
            },
        });

        let AstrcodeTerminalBlockDto::Assistant(block) = &state.transcript[0] else {
            panic!("assistant block should remain present");
        };
        assert_eq!(block.markdown, "replaced");
    }

    #[test]
    fn overlay_selection_tracks_resume_and_slash_items() {
        let mut state = CliState::new("http://127.0.0.1:5529".to_string(), None, capabilities());
        state.set_slash_query(
            "review",
            vec![AstrcodeTerminalSlashCandidateDto {
                id: "skill-review".to_string(),
                title: "Review".to_string(),
                description: "review skill".to_string(),
                keywords: vec!["review".to_string()],
                action_kind: AstrcodeTerminalSlashActionKindDto::InsertText,
                action_value: "/skill review".to_string(),
            }],
        );

        assert!(matches!(
            state.selected_overlay(),
            Some(OverlaySelection::SlashCandidate(_))
        ));
        state.close_overlay();
        assert!(matches!(state.overlay, OverlayState::None));
    }

    #[test]
    fn resize_invalidates_wrap_cache() {
        let mut state = CliState::new("http://127.0.0.1:5529".to_string(), None, capabilities());
        state.update_transcript_cache(
            80,
            vec![WrappedLine {
                style: WrappedLineStyle::Plain,
                content: "cached".to_string(),
            }],
        );
        state.set_viewport_size(100, 40);

        assert_eq!(state.render.transcript_cache.lines.len(), 0);
        assert_eq!(state.scroll_anchor, 0);
    }
}
