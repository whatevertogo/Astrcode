use std::collections::BTreeSet;

use astrcode_client::{AstrcodeConversationSlashCandidateDto, AstrcodeSessionListItem};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PaneFocus {
    Transcript,
    #[default]
    Composer,
    Palette,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ComposerState {
    pub input: String,
    pub cursor: usize,
}

impl ComposerState {
    pub fn line_count(&self) -> usize {
        self.input.lines().count().max(1)
    }

    pub fn is_empty(&self) -> bool {
        self.input.is_empty()
    }

    pub fn as_str(&self) -> &str {
        self.input.as_str()
    }

    pub fn insert_char(&mut self, ch: char) {
        self.input.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
    }

    pub fn insert_str(&mut self, value: &str) {
        self.input.insert_str(self.cursor, value);
        self.cursor += value.len();
    }

    pub fn insert_newline(&mut self) {
        self.insert_char('\n');
    }

    pub fn backspace(&mut self) {
        let Some(start) = previous_boundary(self.input.as_str(), self.cursor) else {
            return;
        };
        self.input.drain(start..self.cursor);
        self.cursor = start;
    }

    pub fn delete_forward(&mut self) {
        let Some(end) = next_boundary(self.input.as_str(), self.cursor) else {
            return;
        };
        self.input.drain(self.cursor..end);
    }

    pub fn move_left(&mut self) {
        if let Some(cursor) = previous_boundary(self.input.as_str(), self.cursor) {
            self.cursor = cursor;
        }
    }

    pub fn move_right(&mut self) {
        if let Some(cursor) = next_boundary(self.input.as_str(), self.cursor) {
            self.cursor = cursor;
        }
    }

    pub fn move_home(&mut self) {
        let line_start = self
            .input
            .get(..self.cursor)
            .and_then(|value| value.rfind('\n').map(|index| index + 1))
            .unwrap_or(0);
        self.cursor = line_start;
    }

    pub fn move_end(&mut self) {
        let line_end = self
            .input
            .get(self.cursor..)
            .and_then(|value| value.find('\n').map(|index| self.cursor + index))
            .unwrap_or(self.input.len());
        self.cursor = line_end;
    }

    pub fn replace(&mut self, input: impl Into<String>) {
        self.input = input.into();
        self.cursor = self.input.len();
    }

    pub fn take(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.input)
    }
}

fn previous_boundary(input: &str, cursor: usize) -> Option<usize> {
    if cursor == 0 {
        return None;
    }
    input
        .get(..cursor)?
        .char_indices()
        .last()
        .map(|(index, _)| index)
}

fn next_boundary(input: &str, cursor: usize) -> Option<usize> {
    if cursor >= input.len() {
        return None;
    }
    let ch = input.get(cursor..)?.chars().next()?;
    Some(cursor + ch.len_utf8())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashPaletteState {
    pub query: String,
    pub items: Vec<AstrcodeConversationSlashCandidateDto>,
    pub selected: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumePaletteState {
    pub query: String,
    pub items: Vec<AstrcodeSessionListItem>,
    pub selected: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PaletteState {
    #[default]
    Closed,
    Slash(SlashPaletteState),
    Resume(ResumePaletteState),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteSelection {
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
pub struct TranscriptState {
    pub selected_cell: usize,
    pub expanded_cells: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteractionState {
    pub status: StatusLine,
    pub scroll_anchor: u16,
    pub follow_transcript_tail: bool,
    pub selection_drives_scroll: bool,
    pub pane_focus: PaneFocus,
    pub last_non_palette_focus: PaneFocus,
    pub composer: ComposerState,
    pub palette: PaletteState,
    pub transcript: TranscriptState,
}

impl Default for InteractionState {
    fn default() -> Self {
        Self {
            status: StatusLine::default(),
            scroll_anchor: 0,
            follow_transcript_tail: true,
            selection_drives_scroll: true,
            pane_focus: PaneFocus::default(),
            last_non_palette_focus: PaneFocus::default(),
            composer: ComposerState::default(),
            palette: PaletteState::default(),
            transcript: TranscriptState::default(),
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
        self.set_focus(PaneFocus::Composer);
        self.composer.insert_char(ch);
    }

    pub fn append_input(&mut self, value: &str) {
        self.set_focus(PaneFocus::Composer);
        self.composer.insert_str(value);
    }

    pub fn insert_newline(&mut self) {
        self.set_focus(PaneFocus::Composer);
        self.composer.insert_newline();
    }

    pub fn pop_input(&mut self) {
        self.composer.backspace();
    }

    pub fn delete_input(&mut self) {
        self.composer.delete_forward();
    }

    pub fn move_cursor_left(&mut self) {
        self.composer.move_left();
    }

    pub fn move_cursor_right(&mut self) {
        self.composer.move_right();
    }

    pub fn move_cursor_home(&mut self) {
        self.composer.move_home();
    }

    pub fn move_cursor_end(&mut self) {
        self.composer.move_end();
    }

    pub fn replace_input(&mut self, input: impl Into<String>) {
        self.set_focus(PaneFocus::Composer);
        self.composer.replace(input);
    }

    pub fn take_input(&mut self) -> String {
        self.composer.take()
    }

    pub fn scroll_up(&mut self) {
        self.scroll_up_by(1);
    }

    pub fn scroll_up_by(&mut self, lines: u16) {
        self.follow_transcript_tail = false;
        self.selection_drives_scroll = false;
        self.scroll_anchor = self.scroll_anchor.saturating_add(lines.max(1));
    }

    pub fn scroll_down(&mut self) {
        self.scroll_down_by(1);
    }

    pub fn scroll_down_by(&mut self, lines: u16) {
        self.scroll_anchor = self.scroll_anchor.saturating_sub(lines.max(1));
        self.follow_transcript_tail = self.scroll_anchor == 0;
        self.selection_drives_scroll = self.follow_transcript_tail;
    }

    pub fn reset_scroll(&mut self) {
        self.scroll_anchor = 0;
        self.follow_transcript_tail = true;
        self.selection_drives_scroll = true;
    }

    pub fn cycle_focus_forward(&mut self) {
        self.set_focus(match self.pane_focus {
            PaneFocus::Transcript => PaneFocus::Composer,
            PaneFocus::Composer => PaneFocus::Transcript,
            PaneFocus::Palette => PaneFocus::Palette,
        });
    }

    pub fn cycle_focus_backward(&mut self) {
        self.set_focus(match self.pane_focus {
            PaneFocus::Transcript => PaneFocus::Composer,
            PaneFocus::Composer => PaneFocus::Transcript,
            PaneFocus::Palette => PaneFocus::Palette,
        });
    }

    pub fn set_focus(&mut self, focus: PaneFocus) {
        self.pane_focus = focus;
        if !matches!(focus, PaneFocus::Palette) {
            self.last_non_palette_focus = focus;
        }
    }

    pub fn transcript_next(&mut self, cell_count: usize) {
        if cell_count == 0 {
            return;
        }
        self.set_focus(PaneFocus::Transcript);
        self.transcript.selected_cell = (self.transcript.selected_cell + 1) % cell_count;
        self.follow_transcript_tail = false;
        self.selection_drives_scroll = true;
    }

    pub fn transcript_prev(&mut self, cell_count: usize) {
        if cell_count == 0 {
            return;
        }
        self.set_focus(PaneFocus::Transcript);
        self.transcript.selected_cell =
            (self.transcript.selected_cell + cell_count - 1) % cell_count;
        self.follow_transcript_tail = false;
        self.selection_drives_scroll = true;
    }

    pub fn sync_transcript_cells(&mut self, cell_count: usize) {
        if cell_count == 0 {
            self.transcript.selected_cell = 0;
            self.transcript.expanded_cells.clear();
            return;
        }
        if self.transcript.selected_cell >= cell_count {
            self.transcript.selected_cell = cell_count - 1;
        }
    }

    pub fn toggle_cell_expanded(&mut self, cell_id: &str) {
        if !self.transcript.expanded_cells.insert(cell_id.to_string()) {
            self.transcript.expanded_cells.remove(cell_id);
        }
    }

    pub fn is_cell_expanded(&self, cell_id: &str) -> bool {
        self.transcript.expanded_cells.contains(cell_id)
    }

    pub fn reset_for_snapshot(&mut self) {
        self.reset_scroll();
        self.palette = PaletteState::Closed;
        self.transcript = TranscriptState::default();
        self.set_focus(PaneFocus::Composer);
    }

    pub fn set_resume_palette(
        &mut self,
        query: impl Into<String>,
        items: Vec<AstrcodeSessionListItem>,
    ) {
        self.palette = PaletteState::Resume(ResumePaletteState {
            query: query.into(),
            items,
            selected: 0,
        });
        self.pane_focus = PaneFocus::Palette;
    }

    pub fn sync_resume_items(&mut self, items: Vec<AstrcodeSessionListItem>) {
        if let PaletteState::Resume(resume) = &mut self.palette {
            resume.items = items;
            if resume.selected >= resume.items.len() {
                resume.selected = 0;
            }
        }
    }

    pub fn set_slash_palette(
        &mut self,
        query: impl Into<String>,
        items: Vec<AstrcodeConversationSlashCandidateDto>,
    ) {
        self.palette = PaletteState::Slash(SlashPaletteState {
            query: query.into(),
            items,
            selected: 0,
        });
        self.pane_focus = PaneFocus::Palette;
    }

    pub fn sync_slash_items(&mut self, items: Vec<AstrcodeConversationSlashCandidateDto>) {
        if let PaletteState::Slash(palette) = &mut self.palette {
            palette.items = items;
            if palette.selected >= palette.items.len() {
                palette.selected = 0;
            }
        }
    }

    pub fn close_palette(&mut self) {
        self.palette = PaletteState::Closed;
        self.set_focus(self.last_non_palette_focus);
    }

    pub fn has_palette(&self) -> bool {
        !matches!(self.palette, PaletteState::Closed)
    }

    pub fn palette_next(&mut self) {
        match &mut self.palette {
            PaletteState::Resume(resume) if !resume.items.is_empty() => {
                resume.selected = (resume.selected + 1) % resume.items.len();
            },
            PaletteState::Slash(palette) if !palette.items.is_empty() => {
                palette.selected = (palette.selected + 1) % palette.items.len();
            },
            PaletteState::Closed | PaletteState::Resume(_) | PaletteState::Slash(_) => {},
        }
    }

    pub fn palette_prev(&mut self) {
        match &mut self.palette {
            PaletteState::Resume(resume) if !resume.items.is_empty() => {
                resume.selected =
                    (resume.selected + resume.items.len().saturating_sub(1)) % resume.items.len();
            },
            PaletteState::Slash(palette) if !palette.items.is_empty() => {
                palette.selected = (palette.selected + palette.items.len().saturating_sub(1))
                    % palette.items.len();
            },
            PaletteState::Closed | PaletteState::Resume(_) | PaletteState::Slash(_) => {},
        }
    }

    pub fn selected_palette(&self) -> Option<PaletteSelection> {
        match &self.palette {
            PaletteState::Resume(resume) => resume
                .items
                .get(resume.selected)
                .map(|item| PaletteSelection::ResumeSession(item.session_id.clone())),
            PaletteState::Slash(palette) => palette
                .items
                .get(palette.selected)
                .cloned()
                .map(PaletteSelection::SlashCandidate),
            PaletteState::Closed => None,
        }
    }

    pub fn clear_surface_state(&mut self) {
        match self.pane_focus {
            PaneFocus::Transcript => {
                self.reset_scroll();
                self.transcript.expanded_cells.clear();
            },
            PaneFocus::Composer => {
                self.status = StatusLine::default();
            },
            PaneFocus::Palette => self.close_palette(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_flow_cycles_two_surfaces() {
        let mut state = InteractionState::default();
        state.set_focus(PaneFocus::Transcript);
        state.cycle_focus_forward();
        assert_eq!(state.pane_focus, PaneFocus::Composer);
        state.cycle_focus_forward();
        assert_eq!(state.pane_focus, PaneFocus::Transcript);
    }

    #[test]
    fn close_palette_restores_previous_focus() {
        let mut state = InteractionState::default();
        state.set_focus(PaneFocus::Transcript);
        state.set_slash_palette("", Vec::new());
        assert_eq!(state.pane_focus, PaneFocus::Palette);
        state.close_palette();
        assert_eq!(state.pane_focus, PaneFocus::Transcript);
    }

    #[test]
    fn transcript_expansion_toggles_by_cell_id() {
        let mut state = InteractionState::default();
        state.toggle_cell_expanded("assistant-1");
        assert!(state.is_cell_expanded("assistant-1"));
        state.toggle_cell_expanded("assistant-1");
        assert!(!state.is_cell_expanded("assistant-1"));
    }

    #[test]
    fn composer_backspace_respects_cursor_position() {
        let mut state = InteractionState::default();
        state.replace_input("abcd");
        state.move_cursor_left();
        state.move_cursor_left();
        state.pop_input();
        assert_eq!(state.composer.as_str(), "acd");
        assert_eq!(state.composer.cursor, 1);
    }

    #[test]
    fn composer_delete_forward_respects_cursor_position() {
        let mut state = InteractionState::default();
        state.replace_input("abcd");
        state.move_cursor_left();
        state.move_cursor_left();
        state.delete_input();
        assert_eq!(state.composer.as_str(), "abd");
        assert_eq!(state.composer.cursor, 2);
    }

    #[test]
    fn manual_scroll_disables_selection_driven_scroll() {
        let mut state = InteractionState::default();
        state.transcript_next(4);
        assert!(state.selection_drives_scroll);
        state.scroll_up();
        assert!(!state.selection_drives_scroll);
    }
}
