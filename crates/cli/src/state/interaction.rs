use astrcode_client::{AstrcodeConversationSlashCandidateDto, AstrcodeSessionListItem};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PaneFocus {
    Transcript,
    ChildPane,
    #[default]
    Composer,
    Overlay,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ComposerState {
    pub input: String,
}

impl ComposerState {
    pub fn line_count(&self) -> usize {
        self.input.lines().count().max(1)
    }

    pub fn is_empty(&self) -> bool {
        self.input.is_empty()
    }
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
    pub items: Vec<AstrcodeConversationSlashCandidateDto>,
    pub selected: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DebugOverlayState {
    pub scroll: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum OverlayState {
    #[default]
    None,
    Resume(ResumeOverlayState),
    SlashPalette(SlashPaletteState),
    DebugLogs(DebugOverlayState),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverlaySelection {
    ResumeSession(String),
    SlashCandidate(AstrcodeConversationSlashCandidateDto),
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
pub struct InteractionState {
    pub status: StatusLine,
    pub scroll_anchor: u16,
    pub follow_transcript_tail: bool,
    pub pane_focus: PaneFocus,
    pub composer: ComposerState,
    pub overlay: OverlayState,
    pub child_pane: ChildPaneState,
}

impl Default for InteractionState {
    fn default() -> Self {
        Self {
            status: StatusLine::default(),
            scroll_anchor: 0,
            follow_transcript_tail: true,
            pane_focus: PaneFocus::default(),
            composer: ComposerState::default(),
            overlay: OverlayState::default(),
            child_pane: ChildPaneState::default(),
        }
    }
}

impl InteractionState {
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

    pub fn append_input(&mut self, value: &str) {
        self.composer.input.push_str(value);
    }

    pub fn insert_newline(&mut self) {
        self.composer.input.push('\n');
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
        self.follow_transcript_tail = false;
        self.scroll_anchor = self.scroll_anchor.saturating_add(1);
    }

    pub fn scroll_down(&mut self) {
        self.scroll_anchor = self.scroll_anchor.saturating_sub(1);
        self.follow_transcript_tail = self.scroll_anchor == 0;
    }

    pub fn reset_scroll(&mut self) {
        self.scroll_anchor = 0;
        self.follow_transcript_tail = true;
    }

    pub fn cycle_focus_forward(&mut self, has_children: bool) {
        self.pane_focus = match self.pane_focus {
            PaneFocus::Transcript => {
                if has_children {
                    PaneFocus::ChildPane
                } else {
                    PaneFocus::Composer
                }
            },
            PaneFocus::ChildPane => PaneFocus::Composer,
            PaneFocus::Composer => PaneFocus::Transcript,
            PaneFocus::Overlay => PaneFocus::Overlay,
        };
    }

    pub fn cycle_focus_backward(&mut self, has_children: bool) {
        self.pane_focus = match self.pane_focus {
            PaneFocus::Transcript => PaneFocus::Composer,
            PaneFocus::ChildPane => PaneFocus::Transcript,
            PaneFocus::Composer => {
                if has_children {
                    PaneFocus::ChildPane
                } else {
                    PaneFocus::Transcript
                }
            },
            PaneFocus::Overlay => PaneFocus::Overlay,
        };
    }

    pub fn child_next(&mut self, child_count: usize) {
        if child_count == 0 {
            return;
        }
        self.child_pane.selected = (self.child_pane.selected + 1) % child_count;
    }

    pub fn child_prev(&mut self, child_count: usize) {
        if child_count == 0 {
            return;
        }
        self.child_pane.selected = (self.child_pane.selected + child_count - 1) % child_count;
    }

    pub fn toggle_child_focus(&mut self, selected_child_session_id: Option<&str>) {
        let Some(selected_child_session_id) = selected_child_session_id else {
            return;
        };
        if self.child_pane.focused_child_session_id.as_deref() == Some(selected_child_session_id) {
            self.child_pane.focused_child_session_id = None;
        } else {
            self.child_pane.focused_child_session_id = Some(selected_child_session_id.to_string());
        }
    }

    pub fn reset_for_snapshot(&mut self) {
        self.reset_scroll();
        self.overlay = OverlayState::None;
        self.pane_focus = PaneFocus::Composer;
        self.child_pane = ChildPaneState::default();
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

    pub fn sync_resume_items(&mut self, items: Vec<AstrcodeSessionListItem>) {
        if let OverlayState::Resume(resume) = &mut self.overlay {
            resume.items = items;
            if resume.selected >= resume.items.len() {
                resume.selected = 0;
            }
        }
    }

    pub fn set_slash_query(
        &mut self,
        query: impl Into<String>,
        items: Vec<AstrcodeConversationSlashCandidateDto>,
    ) {
        self.pane_focus = PaneFocus::Overlay;
        self.overlay = OverlayState::SlashPalette(SlashPaletteState {
            query: query.into(),
            items,
            selected: 0,
        });
    }

    pub fn sync_slash_items(&mut self, items: Vec<AstrcodeConversationSlashCandidateDto>) {
        if let OverlayState::SlashPalette(palette) = &mut self.overlay {
            palette.items = items;
            if palette.selected >= palette.items.len() {
                palette.selected = 0;
            }
        }
    }

    pub fn overlay_query_push(&mut self, ch: char) {
        match &mut self.overlay {
            OverlayState::Resume(resume) => resume.query.push(ch),
            OverlayState::SlashPalette(palette) => palette.query.push(ch),
            OverlayState::DebugLogs(_) => {},
            OverlayState::None => self.push_input(ch),
        }
    }

    pub fn overlay_query_append(&mut self, value: &str) {
        match &mut self.overlay {
            OverlayState::Resume(resume) => resume.query.push_str(value),
            OverlayState::SlashPalette(palette) => palette.query.push_str(value),
            OverlayState::DebugLogs(_) => {},
            OverlayState::None => self.append_input(value),
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
            OverlayState::DebugLogs(_) => {},
            OverlayState::None => self.pop_input(),
        }
    }

    pub fn close_overlay(&mut self) {
        self.overlay = OverlayState::None;
        self.pane_focus = PaneFocus::Composer;
    }

    pub fn has_overlay(&self) -> bool {
        !matches!(self.overlay, OverlayState::None)
    }

    pub fn overlay_next(&mut self) {
        match &mut self.overlay {
            OverlayState::Resume(resume) if !resume.items.is_empty() => {
                resume.selected = (resume.selected + 1) % resume.items.len();
            },
            OverlayState::SlashPalette(palette) if !palette.items.is_empty() => {
                palette.selected = (palette.selected + 1) % palette.items.len();
            },
            OverlayState::DebugLogs(debug) => {
                debug.scroll = debug.scroll.saturating_add(1);
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
            OverlayState::DebugLogs(debug) => {
                debug.scroll = debug.scroll.saturating_sub(1);
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
            OverlayState::DebugLogs(_) => None,
            OverlayState::None => None,
        }
    }

    pub fn toggle_debug_overlay(&mut self) {
        if matches!(self.overlay, OverlayState::DebugLogs(_)) {
            self.close_overlay();
        } else {
            self.pane_focus = PaneFocus::Overlay;
            self.overlay = OverlayState::DebugLogs(DebugOverlayState::default());
        }
    }
}
