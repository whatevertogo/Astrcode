mod conversation;
mod debug;
mod interaction;
mod render;
mod shell;
mod thinking;
mod transcript_cell;

use std::{path::PathBuf, time::Duration};

use astrcode_client::{
    AstrcodeConversationErrorEnvelopeDto, AstrcodeConversationSlashCandidateDto,
    AstrcodeConversationSnapshotResponseDto, AstrcodeConversationStreamEnvelopeDto,
    AstrcodePhaseDto, AstrcodeSessionListItem,
};
pub use conversation::ConversationState;
pub use debug::DebugChannelState;
pub use interaction::{
    ComposerState, InteractionState, PaletteSelection, PaletteState, PaneFocus, ResumePaletteState,
    SlashPaletteState, StatusLine,
};
pub use render::{
    RenderState, StreamViewState, TranscriptRenderCache, WrappedLine, WrappedLineStyle,
};
pub use shell::ShellState;
pub use thinking::{ThinkingPlaybackDriver, ThinkingPresentationState, ThinkingSnippetPool};
pub use transcript_cell::{TranscriptCell, TranscriptCellKind, TranscriptCellStatus};

use crate::capability::TerminalCapabilities;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StreamRenderMode {
    #[default]
    Smooth,
    CatchUp,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CliState {
    pub shell: ShellState,
    pub conversation: ConversationState,
    pub interaction: InteractionState,
    pub render: RenderState,
    pub stream_view: StreamViewState,
    pub debug: DebugChannelState,
    pub thinking_pool: ThinkingSnippetPool,
    pub thinking_playback: ThinkingPlaybackDriver,
}

impl CliState {
    pub fn new(
        connection_origin: String,
        working_dir: Option<PathBuf>,
        capabilities: TerminalCapabilities,
    ) -> Self {
        Self {
            shell: ShellState::new(connection_origin, working_dir, capabilities),
            thinking_pool: ThinkingSnippetPool::default(),
            thinking_playback: ThinkingPlaybackDriver::default(),
            ..Default::default()
        }
    }

    pub fn set_status(&mut self, message: impl Into<String>) {
        self.interaction.set_status(message);
        self.render.mark_footer_dirty();
    }

    pub fn set_error_status(&mut self, message: impl Into<String>) {
        self.interaction.set_error_status(message);
        self.render.mark_footer_dirty();
    }

    pub fn set_stream_mode(
        &mut self,
        mode: StreamRenderMode,
        pending: usize,
        oldest: Duration,
    ) -> bool {
        let changed = self.stream_view.mode != mode || self.stream_view.pending_chunks != pending;
        self.stream_view.update(mode, pending, oldest);
        if changed {
            self.render.mark_footer_dirty();
        }
        changed
    }

    pub fn set_viewport_size(&mut self, width: u16, height: u16) {
        self.render.set_viewport_size(width, height);
    }

    pub fn update_transcript_cache(
        &mut self,
        width: u16,
        lines: Vec<WrappedLine>,
        selected_line_range: Option<(usize, usize)>,
    ) {
        self.render
            .update_transcript_cache(width, lines, selected_line_range);
    }

    pub fn push_input(&mut self, ch: char) {
        self.interaction.push_input(ch);
        self.render.mark_footer_dirty();
    }

    pub fn append_input(&mut self, value: &str) {
        self.interaction.append_input(value);
        self.render.mark_footer_dirty();
    }

    pub fn insert_newline(&mut self) {
        self.interaction.insert_newline();
        self.render.mark_footer_dirty();
    }

    pub fn pop_input(&mut self) {
        self.interaction.pop_input();
        self.render.mark_footer_dirty();
    }

    pub fn delete_input(&mut self) {
        self.interaction.delete_input();
        self.render.mark_footer_dirty();
    }

    pub fn move_cursor_left(&mut self) {
        self.interaction.move_cursor_left();
        self.render.mark_footer_dirty();
    }

    pub fn move_cursor_right(&mut self) {
        self.interaction.move_cursor_right();
        self.render.mark_footer_dirty();
    }

    pub fn move_cursor_home(&mut self) {
        self.interaction.move_cursor_home();
        self.render.mark_footer_dirty();
    }

    pub fn move_cursor_end(&mut self) {
        self.interaction.move_cursor_end();
        self.render.mark_footer_dirty();
    }

    pub fn replace_input(&mut self, input: impl Into<String>) {
        self.interaction.replace_input(input);
        self.render.mark_footer_dirty();
    }

    pub fn take_input(&mut self) -> String {
        let input = self.interaction.take_input();
        self.render.mark_footer_dirty();
        input
    }

    pub fn scroll_up(&mut self) {
        self.interaction.scroll_up();
        self.render.mark_transcript_dirty();
    }

    pub fn scroll_down(&mut self) {
        self.interaction.scroll_down();
        self.render.mark_transcript_dirty();
    }

    pub fn scroll_up_by(&mut self, lines: u16) {
        self.interaction.scroll_up_by(lines);
        self.render.mark_transcript_dirty();
    }

    pub fn scroll_down_by(&mut self, lines: u16) {
        self.interaction.scroll_down_by(lines);
        self.render.mark_transcript_dirty();
    }

    pub fn cycle_focus_forward(&mut self) {
        self.interaction.cycle_focus_forward();
        self.render.invalidate_transcript_cache();
        self.render.mark_footer_dirty();
        self.render.mark_palette_dirty();
    }

    pub fn cycle_focus_backward(&mut self) {
        self.interaction.cycle_focus_backward();
        self.render.invalidate_transcript_cache();
        self.render.mark_footer_dirty();
        self.render.mark_palette_dirty();
    }

    pub fn transcript_next(&mut self) {
        self.interaction
            .transcript_next(self.conversation.transcript.len());
        self.render.invalidate_transcript_cache();
    }

    pub fn transcript_prev(&mut self) {
        self.interaction
            .transcript_prev(self.conversation.transcript.len());
        self.render.invalidate_transcript_cache();
    }

    pub fn transcript_cells(&self) -> Vec<TranscriptCell> {
        self.conversation
            .project_transcript_cells(&self.interaction.transcript.expanded_cells)
    }

    pub fn selected_transcript_cell(&self) -> Option<TranscriptCell> {
        self.conversation.project_transcript_cell(
            self.interaction.transcript.selected_cell,
            &self.interaction.transcript.expanded_cells,
        )
    }

    pub fn is_cell_expanded(&self, cell_id: &str) -> bool {
        self.interaction.is_cell_expanded(cell_id)
    }

    pub fn selected_cell_is_thinking(&self) -> bool {
        self.selected_transcript_cell()
            .is_some_and(|cell| matches!(cell.kind, TranscriptCellKind::Thinking { .. }))
    }

    pub fn toggle_selected_cell_expanded(&mut self) {
        if let Some(cell_id) = self.selected_transcript_cell().map(|cell| cell.id.clone()) {
            self.interaction.toggle_cell_expanded(cell_id.as_str());
            self.render.invalidate_transcript_cache();
        }
    }

    pub fn clear_surface_state(&mut self) {
        let invalidate = matches!(self.interaction.pane_focus, PaneFocus::Transcript);
        self.interaction.clear_surface_state();
        if invalidate {
            self.render.invalidate_transcript_cache();
        }
    }

    pub fn update_sessions(&mut self, sessions: Vec<AstrcodeSessionListItem>) {
        self.conversation.update_sessions(sessions);
        self.interaction
            .sync_resume_items(self.conversation.sessions.clone());
        self.render.invalidate_transcript_cache();
        self.render.mark_palette_dirty();
    }

    pub fn set_resume_query(
        &mut self,
        query: impl Into<String>,
        items: Vec<AstrcodeSessionListItem>,
    ) {
        self.interaction.set_resume_palette(query, items);
        self.render.invalidate_footer_cache();
        self.render.invalidate_palette_cache();
    }

    pub fn set_slash_query(
        &mut self,
        query: impl Into<String>,
        items: Vec<AstrcodeConversationSlashCandidateDto>,
    ) {
        self.interaction.set_slash_palette(query, items);
        self.render.invalidate_footer_cache();
        self.render.invalidate_palette_cache();
    }

    pub fn close_palette(&mut self) {
        self.interaction.close_palette();
        self.render.invalidate_footer_cache();
        self.render.invalidate_palette_cache();
    }

    pub fn palette_next(&mut self) {
        self.interaction.palette_next();
        self.render.mark_footer_dirty();
        self.render.mark_palette_dirty();
    }

    pub fn palette_prev(&mut self) {
        self.interaction.palette_prev();
        self.render.mark_footer_dirty();
        self.render.mark_palette_dirty();
    }

    pub fn selected_palette(&self) -> Option<PaletteSelection> {
        self.interaction.selected_palette()
    }

    pub fn activate_snapshot(&mut self, snapshot: AstrcodeConversationSnapshotResponseDto) {
        self.conversation
            .activate_snapshot(snapshot, &mut self.render);
        self.interaction.reset_for_snapshot();
        self.interaction
            .sync_transcript_cells(self.conversation.transcript.len());
        self.thinking_playback
            .sync_session(self.conversation.active_session_id.as_deref());
        self.render.invalidate_transcript_cache();
        self.render.invalidate_footer_cache();
        self.render.invalidate_palette_cache();
    }

    pub fn apply_stream_envelope(&mut self, envelope: AstrcodeConversationStreamEnvelopeDto) {
        let expanded_ids = &self.interaction.transcript.expanded_cells;
        let slash_candidates_changed =
            self.conversation
                .apply_stream_envelope(envelope, &mut self.render, expanded_ids);
        self.interaction
            .sync_transcript_cells(self.conversation.transcript.len());
        if slash_candidates_changed {
            self.interaction
                .sync_slash_items(self.conversation.slash_candidates.clone());
        }
        self.render.invalidate_transcript_cache();
        self.render.mark_footer_dirty();
        if slash_candidates_changed {
            self.render.mark_palette_dirty();
        }
    }

    pub fn set_banner_error(&mut self, error: AstrcodeConversationErrorEnvelopeDto) {
        self.conversation.set_banner_error(error);
        self.interaction.set_focus(PaneFocus::Composer);
        self.render.invalidate_transcript_cache();
        self.render.mark_footer_dirty();
    }

    pub fn clear_banner(&mut self) {
        self.conversation.clear_banner();
        self.render.invalidate_transcript_cache();
    }

    pub fn active_phase(&self) -> Option<AstrcodePhaseDto> {
        self.conversation.active_phase()
    }

    pub fn push_debug_line(&mut self, line: impl Into<String>) {
        self.debug.push(line);
    }

    pub fn advance_thinking_playback(&mut self) -> bool {
        if self.should_animate_thinking_playback() {
            self.thinking_playback.advance();
            self.render.invalidate_transcript_cache();
            return true;
        }
        false
    }

    fn should_animate_thinking_playback(&self) -> bool {
        if self.transcript_cells().iter().any(|cell| {
            matches!(
                cell.kind,
                TranscriptCellKind::Thinking {
                    status: TranscriptCellStatus::Streaming,
                    ..
                }
            )
        }) {
            return true;
        }

        let Some(control) = &self.conversation.control else {
            return false;
        };
        if control.active_turn_id.is_none() {
            return false;
        }
        if !matches!(
            control.phase,
            AstrcodePhaseDto::Thinking
                | AstrcodePhaseDto::CallingTool
                | AstrcodePhaseDto::Streaming
        ) {
            return false;
        }

        !self.transcript_cells().iter().any(|cell| match &cell.kind {
            TranscriptCellKind::Thinking { status, .. } => {
                matches!(
                    status,
                    TranscriptCellStatus::Streaming | TranscriptCellStatus::Complete
                )
            },
            TranscriptCellKind::Assistant { status, body } => {
                matches!(status, TranscriptCellStatus::Streaming) && !body.trim().is_empty()
            },
            TranscriptCellKind::ToolCall { status, .. } => {
                matches!(status, TranscriptCellStatus::Streaming)
            },
            _ => false,
        })
    }
}

#[cfg(test)]
mod tests {
    use astrcode_client::{
        AstrcodeConversationAssistantBlockDto, AstrcodeConversationBlockDto,
        AstrcodeConversationBlockPatchDto, AstrcodeConversationBlockStatusDto,
        AstrcodeConversationControlStateDto, AstrcodeConversationCursorDto,
        AstrcodeConversationDeltaDto, AstrcodeConversationSlashActionKindDto,
    };

    use super::*;
    use crate::capability::{ColorLevel, GlyphMode};

    fn sample_snapshot() -> AstrcodeConversationSnapshotResponseDto {
        AstrcodeConversationSnapshotResponseDto {
            session_id: "session-1".to_string(),
            session_title: "Session 1".to_string(),
            cursor: AstrcodeConversationCursorDto("1.2".to_string()),
            phase: AstrcodePhaseDto::Idle,
            control: AstrcodeConversationControlStateDto {
                phase: AstrcodePhaseDto::Idle,
                can_submit_prompt: true,
                can_request_compact: true,
                compact_pending: false,
                compacting: false,
                active_turn_id: None,
                last_compact_meta: None,
            },
            blocks: vec![AstrcodeConversationBlockDto::Assistant(
                AstrcodeConversationAssistantBlockDto {
                    id: "assistant-1".to_string(),
                    turn_id: Some("turn-1".to_string()),
                    status: AstrcodeConversationBlockStatusDto::Streaming,
                    markdown: "hello".to_string(),
                },
            )],
            child_summaries: Vec::new(),
            slash_candidates: vec![AstrcodeConversationSlashCandidateDto {
                id: "skill-review".to_string(),
                title: "Review".to_string(),
                description: "review skill".to_string(),
                keywords: vec!["review".to_string()],
                action_kind: AstrcodeConversationSlashActionKindDto::InsertText,
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
        state.apply_stream_envelope(AstrcodeConversationStreamEnvelopeDto {
            session_id: "session-1".to_string(),
            cursor: AstrcodeConversationCursorDto("1.3".to_string()),
            delta: AstrcodeConversationDeltaDto::PatchBlock {
                block_id: "assistant-1".to_string(),
                patch: AstrcodeConversationBlockPatchDto::AppendMarkdown {
                    markdown: " world".to_string(),
                },
            },
        });

        let AstrcodeConversationBlockDto::Assistant(block) = &state.conversation.transcript[0]
        else {
            panic!("assistant block should remain present");
        };
        assert_eq!(block.markdown, "hello world");
        assert_eq!(
            state
                .conversation
                .cursor
                .as_ref()
                .map(|cursor| cursor.0.as_str()),
            Some("1.3")
        );
    }

    #[test]
    fn replace_markdown_patch_overwrites_streamed_content() {
        let mut state = CliState::new("http://127.0.0.1:5529".to_string(), None, capabilities());
        state.activate_snapshot(sample_snapshot());
        state.apply_stream_envelope(AstrcodeConversationStreamEnvelopeDto {
            session_id: "session-1".to_string(),
            cursor: AstrcodeConversationCursorDto("1.4".to_string()),
            delta: AstrcodeConversationDeltaDto::PatchBlock {
                block_id: "assistant-1".to_string(),
                patch: AstrcodeConversationBlockPatchDto::ReplaceMarkdown {
                    markdown: "replaced".to_string(),
                },
            },
        });

        let AstrcodeConversationBlockDto::Assistant(block) = &state.conversation.transcript[0]
        else {
            panic!("assistant block should remain present");
        };
        assert_eq!(block.markdown, "replaced");
    }

    #[test]
    fn palette_selection_tracks_resume_and_slash_items() {
        let mut state = CliState::new("http://127.0.0.1:5529".to_string(), None, capabilities());
        state.set_slash_query(
            "review",
            vec![AstrcodeConversationSlashCandidateDto {
                id: "skill-review".to_string(),
                title: "Review".to_string(),
                description: "review skill".to_string(),
                keywords: vec!["review".to_string()],
                action_kind: AstrcodeConversationSlashActionKindDto::InsertText,
                action_value: "/skill review".to_string(),
            }],
        );

        assert!(matches!(
            state.selected_palette(),
            Some(PaletteSelection::SlashCandidate(_))
        ));
        state.set_resume_query("repo", Vec::new());
        assert!(matches!(state.interaction.palette, PaletteState::Resume(_)));
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
            None,
        );
        state.scroll_up();
        state.set_viewport_size(100, 40);

        assert_eq!(state.render.transcript_cache.lines.len(), 0);
        assert_eq!(state.interaction.scroll_anchor, 1);
        assert!(!state.interaction.follow_transcript_tail);
    }

    #[test]
    fn manual_scroll_disables_follow_until_returning_to_tail() {
        let mut state = CliState::new("http://127.0.0.1:5529".to_string(), None, capabilities());

        state.scroll_up();
        assert_eq!(state.interaction.scroll_anchor, 1);
        assert!(!state.interaction.follow_transcript_tail);

        state.scroll_down();
        assert_eq!(state.interaction.scroll_anchor, 0);
        assert!(state.interaction.follow_transcript_tail);
    }

    #[test]
    fn ticking_advances_streaming_thinking() {
        let mut state = CliState::new("http://127.0.0.1:5529".to_string(), None, capabilities());
        state.conversation.control = Some(AstrcodeConversationControlStateDto {
            phase: AstrcodePhaseDto::Thinking,
            can_submit_prompt: true,
            can_request_compact: true,
            compact_pending: false,
            compacting: false,
            active_turn_id: Some("turn-1".to_string()),
            last_compact_meta: None,
        });
        let frame = state.thinking_playback.frame;
        state.advance_thinking_playback();
        assert!(state.thinking_playback.frame > frame);
    }
}
