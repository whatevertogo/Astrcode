mod conversation;
mod debug;
mod interaction;
mod render;
mod shell;
mod transcript_cell;

use std::{path::PathBuf, time::Duration};

use astrcode_client::{
    AstrcodeConversationChildSummaryDto, AstrcodeConversationErrorEnvelopeDto,
    AstrcodeConversationSlashCandidateDto, AstrcodeConversationSnapshotResponseDto,
    AstrcodeConversationStreamEnvelopeDto, AstrcodePhaseDto, AstrcodeSessionListItem,
};
pub use conversation::ConversationState;
pub use debug::DebugChannelState;
pub use interaction::{
    ChildPaneState, ComposerState, DebugOverlayState, InteractionState, OverlaySelection,
    OverlayState, PaneFocus, ResumeOverlayState, SlashPaletteState, StatusLine,
};
pub use render::{RenderState, StreamViewState, TranscriptRenderCache};
pub use shell::ShellState;
pub use transcript_cell::{TranscriptCell, TranscriptCellKind, TranscriptCellStatus};

use crate::capability::TerminalCapabilities;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StreamRenderMode {
    #[default]
    Smooth,
    CatchUp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrappedLine {
    pub style: WrappedLineStyle,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WrappedLineStyle {
    Plain,
    Dim,
    Accent,
    Success,
    Warning,
    Error,
    User,
    Header,
    Footer,
    Selection,
    Border,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CliState {
    pub shell: ShellState,
    pub conversation: ConversationState,
    pub interaction: InteractionState,
    pub render: RenderState,
    pub stream_view: StreamViewState,
    pub debug: DebugChannelState,
}

impl CliState {
    pub fn new(
        connection_origin: String,
        working_dir: Option<PathBuf>,
        capabilities: TerminalCapabilities,
    ) -> Self {
        Self {
            shell: ShellState::new(connection_origin, working_dir, capabilities),
            ..Default::default()
        }
    }

    pub fn set_status(&mut self, message: impl Into<String>) {
        self.interaction.set_status(message);
    }

    pub fn set_error_status(&mut self, message: impl Into<String>) {
        self.interaction.set_error_status(message);
    }

    pub fn set_stream_mode(&mut self, mode: StreamRenderMode, pending: usize, oldest: Duration) {
        self.stream_view.update(mode, pending, oldest);
    }

    pub fn set_viewport_size(&mut self, width: u16, height: u16) {
        self.render.set_viewport_size(width, height);
    }

    pub fn update_transcript_cache(&mut self, width: u16, lines: Vec<WrappedLine>) {
        self.render.update_transcript_cache(width, lines);
    }

    pub fn push_input(&mut self, ch: char) {
        self.interaction.push_input(ch);
    }

    pub fn append_input(&mut self, value: &str) {
        self.interaction.append_input(value);
    }

    pub fn insert_newline(&mut self) {
        self.interaction.insert_newline();
    }

    pub fn pop_input(&mut self) {
        self.interaction.pop_input();
    }

    pub fn replace_input(&mut self, input: impl Into<String>) {
        self.interaction.replace_input(input);
    }

    pub fn take_input(&mut self) -> String {
        self.interaction.take_input()
    }

    pub fn scroll_up(&mut self) {
        self.interaction.scroll_up();
    }

    pub fn scroll_down(&mut self) {
        self.interaction.scroll_down();
    }

    pub fn cycle_focus_forward(&mut self) {
        self.interaction
            .cycle_focus_forward(!self.conversation.child_summaries.is_empty());
    }

    pub fn cycle_focus_backward(&mut self) {
        self.interaction
            .cycle_focus_backward(!self.conversation.child_summaries.is_empty());
    }

    pub fn child_next(&mut self) {
        self.interaction
            .child_next(self.conversation.child_summaries.len());
    }

    pub fn child_prev(&mut self) {
        self.interaction
            .child_prev(self.conversation.child_summaries.len());
    }

    pub fn toggle_child_focus(&mut self) {
        let selected_child_session_id = self
            .selected_child_summary()
            .map(|summary| summary.child_session_id.clone());
        self.interaction
            .toggle_child_focus(selected_child_session_id.as_deref());
    }

    pub fn selected_child_summary(&self) -> Option<&AstrcodeConversationChildSummaryDto> {
        self.conversation
            .selected_child_summary(&self.interaction.child_pane)
    }

    pub fn focused_child_summary(&self) -> Option<&AstrcodeConversationChildSummaryDto> {
        self.conversation
            .focused_child_summary(&self.interaction.child_pane)
    }

    pub fn update_sessions(&mut self, sessions: Vec<AstrcodeSessionListItem>) {
        self.conversation.update_sessions(sessions);
        self.interaction
            .sync_resume_items(self.conversation.sessions.clone());
    }

    pub fn set_resume_query(
        &mut self,
        query: impl Into<String>,
        items: Vec<AstrcodeSessionListItem>,
    ) {
        self.interaction.set_resume_query(query, items);
    }

    pub fn set_slash_query(
        &mut self,
        query: impl Into<String>,
        items: Vec<AstrcodeConversationSlashCandidateDto>,
    ) {
        self.interaction.set_slash_query(query, items);
    }

    pub fn overlay_query_push(&mut self, ch: char) {
        self.interaction.overlay_query_push(ch);
    }

    pub fn overlay_query_append(&mut self, value: &str) {
        self.interaction.overlay_query_append(value);
    }

    pub fn overlay_query_pop(&mut self) {
        self.interaction.overlay_query_pop();
    }

    pub fn close_overlay(&mut self) {
        self.interaction.close_overlay();
    }

    pub fn overlay_next(&mut self) {
        self.interaction.overlay_next();
    }

    pub fn overlay_prev(&mut self) {
        self.interaction.overlay_prev();
    }

    pub fn selected_overlay(&self) -> Option<OverlaySelection> {
        self.interaction.selected_overlay()
    }

    pub fn activate_snapshot(&mut self, snapshot: AstrcodeConversationSnapshotResponseDto) {
        self.conversation
            .activate_snapshot(snapshot, &mut self.render);
        self.interaction.reset_for_snapshot();
    }

    pub fn apply_stream_envelope(&mut self, envelope: AstrcodeConversationStreamEnvelopeDto) {
        self.conversation.apply_stream_envelope(
            envelope,
            &mut self.render,
            &mut self.interaction.child_pane,
        );
        self.interaction
            .sync_slash_items(self.conversation.slash_candidates.clone());
    }

    pub fn set_banner_error(&mut self, error: AstrcodeConversationErrorEnvelopeDto) {
        self.conversation.set_banner_error(error);
        self.interaction.pane_focus = PaneFocus::Composer;
    }

    pub fn clear_banner(&mut self) {
        self.conversation.clear_banner();
    }

    pub fn active_phase(&self) -> Option<AstrcodePhaseDto> {
        self.conversation.active_phase()
    }

    pub fn push_debug_line(&mut self, line: impl Into<String>) {
        self.debug.push(line);
    }

    pub fn toggle_debug_overlay(&mut self) {
        self.interaction.toggle_debug_overlay();
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
                active_turn_id: None,
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
    fn overlay_selection_tracks_resume_and_slash_items() {
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
            state.selected_overlay(),
            Some(OverlaySelection::SlashCandidate(_))
        ));
        state.close_overlay();
        assert!(matches!(state.interaction.overlay, OverlayState::None));
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
    fn activating_snapshot_resets_transcript_follow_state() {
        let mut state = CliState::new("http://127.0.0.1:5529".to_string(), None, capabilities());
        state.scroll_up();

        state.activate_snapshot(sample_snapshot());

        assert_eq!(state.interaction.scroll_anchor, 0);
        assert!(state.interaction.follow_transcript_tail);
    }
}
