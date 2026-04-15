use std::path::PathBuf;

use astrcode_client::{
    AstrcodePhaseDto, AstrcodeSessionListItem, AstrcodeTerminalBannerDto, AstrcodeTerminalBlockDto,
    AstrcodeTerminalBlockPatchDto, AstrcodeTerminalBlockStatusDto, AstrcodeTerminalChildSummaryDto,
    AstrcodeTerminalControlStateDto, AstrcodeTerminalCursorDto, AstrcodeTerminalDeltaDto,
    AstrcodeTerminalErrorEnvelopeDto, AstrcodeTerminalSlashCandidateDto,
    AstrcodeTerminalSnapshotResponseDto, AstrcodeTerminalStreamEnvelopeDto,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PaneFocus {
    Transcript,
    #[default]
    Composer,
    Overlay,
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
pub struct CliState {
    pub connection_origin: String,
    pub working_dir: Option<PathBuf>,
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
}

impl CliState {
    pub fn new(connection_origin: String, working_dir: Option<PathBuf>) -> Self {
        Self {
            connection_origin,
            working_dir,
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

    pub fn overlay_query(&self) -> Option<&str> {
        match &self.overlay {
            OverlayState::Resume(resume) => Some(resume.query.as_str()),
            OverlayState::SlashPalette(palette) => Some(palette.query.as_str()),
            OverlayState::None => None,
        }
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

    fn apply_delta(&mut self, delta: AstrcodeTerminalDeltaDto) {
        match delta {
            AstrcodeTerminalDeltaDto::AppendBlock { block } => self.transcript.push(block),
            AstrcodeTerminalDeltaDto::PatchBlock { block_id, patch } => {
                if let Some(block) = self
                    .transcript
                    .iter_mut()
                    .find(|block| block_id_of(block) == block_id)
                {
                    apply_block_patch(block, patch);
                }
            },
            AstrcodeTerminalDeltaDto::CompleteBlock { block_id, status } => {
                if let Some(block) = self
                    .transcript
                    .iter_mut()
                    .find(|block| block_id_of(block) == block_id)
                {
                    set_block_status(block, status);
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
            },
            AstrcodeTerminalDeltaDto::RemoveChildSummary { child_session_id } => {
                self.child_summaries
                    .retain(|child| child.child_session_id != child_session_id);
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
            AstrcodeTerminalDeltaDto::RehydrateRequired { error } => {
                self.set_banner_error(error);
            },
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

    #[test]
    fn applies_snapshot_and_stream_deltas() {
        let mut state = CliState::new("http://127.0.0.1:5529".to_string(), None);
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
        let mut state = CliState::new("http://127.0.0.1:5529".to_string(), None);
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
        let mut state = CliState::new("http://127.0.0.1:5529".to_string(), None);
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
}
