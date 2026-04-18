use std::{io, io::Write};

use ratatui::{backend::Backend, layout::Rect, text::Line};

use super::{
    custom_terminal::{Frame, Terminal},
    insert_history::insert_history_lines,
};
use crate::model::reducer::CommittedSlice;

const MAX_BOTTOM_PANE_HEIGHT: u16 = 6;

pub fn bottom_pane_height_for_terminal(total_height: u16) -> u16 {
    if total_height <= 8 {
        4
    } else if total_height <= 12 {
        5
    } else {
        MAX_BOTTOM_PANE_HEIGHT
    }
}

#[derive(Debug)]
pub struct TerminalHost<B>
where
    B: Backend<Error = io::Error> + Write,
{
    terminal: Terminal<B>,
    pending_history_lines: Vec<Line<'static>>,
    deferred_history_lines: Vec<Line<'static>>,
    last_known_size: Rect,
    overlay_open: bool,
}

impl<B> TerminalHost<B>
where
    B: Backend<Error = io::Error> + Write,
{
    pub fn with_backend(backend: B) -> io::Result<Self> {
        let terminal = Terminal::with_options(backend)?;
        Ok(Self::new(terminal))
    }

    pub fn new(terminal: Terminal<B>) -> Self {
        let size = terminal.size().expect("terminal size should be readable");
        Self {
            terminal,
            pending_history_lines: Vec::new(),
            deferred_history_lines: Vec::new(),
            last_known_size: Rect::new(0, 0, size.width, size.height),
            overlay_open: false,
        }
    }

    pub fn terminal(&self) -> &Terminal<B> {
        &self.terminal
    }

    pub fn terminal_mut(&mut self) -> &mut Terminal<B> {
        &mut self.terminal
    }

    pub fn on_new_commits(&mut self, commits: Vec<CommittedSlice>) -> bool {
        if commits.is_empty() {
            return false;
        }
        for slice in commits {
            self.pending_history_lines
                .extend(slice.lines.iter().cloned());
        }
        true
    }

    pub fn on_resize(&mut self, width: u16, height: u16) -> io::Result<bool> {
        let next_size = Rect::new(0, 0, width, height);
        if self.last_known_size == next_size {
            return Ok(false);
        }
        self.last_known_size = next_size;
        self.terminal.autoresize()?;
        Ok(true)
    }

    pub fn draw_frame<F>(
        &mut self,
        viewport_height: u16,
        overlay_open: bool,
        render: F,
    ) -> io::Result<()>
    where
        F: FnOnce(&mut Frame<'_>, Rect),
    {
        self.update_inline_viewport(viewport_height)?;

        if overlay_open {
            if !self.pending_history_lines.is_empty() {
                self.deferred_history_lines
                    .append(&mut self.pending_history_lines);
            }
        } else {
            if self.overlay_open && !self.deferred_history_lines.is_empty() {
                self.pending_history_lines
                    .append(&mut self.deferred_history_lines);
            }
            self.flush_pending_history()?;
        }
        self.overlay_open = overlay_open;

        self.terminal.draw(|frame| {
            let area = frame.area();
            render(frame, area);
        })
    }

    fn update_inline_viewport(&mut self, height: u16) -> io::Result<()> {
        let size = self.terminal.size()?;
        let mut area = self.terminal.viewport_area;
        area.height = height.min(size.height).max(1);
        area.width = size.width;
        area.y = size.height.saturating_sub(area.height);
        if area != self.terminal.viewport_area {
            self.terminal.clear()?;
            self.terminal.set_viewport_area(area);
        }
        Ok(())
    }

    fn flush_pending_history(&mut self) -> io::Result<()> {
        if self.pending_history_lines.is_empty() {
            return Ok(());
        }
        let lines = std::mem::take(&mut self.pending_history_lines);
        insert_history_lines(&mut self.terminal, lines)?;
        self.terminal.invalidate_viewport();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::bottom_pane_height_for_terminal;

    #[test]
    fn bottom_pane_height_stays_small() {
        assert_eq!(bottom_pane_height_for_terminal(8), 4);
        assert_eq!(bottom_pane_height_for_terminal(12), 5);
        assert_eq!(bottom_pane_height_for_terminal(24), 6);
    }
}
