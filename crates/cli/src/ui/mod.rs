pub mod cells;
mod footer;
mod palette;
mod theme;
mod transcript;

pub use footer::footer_lines;
pub use palette::{palette_lines, palette_visible};
use ratatui::text::{Line, Span};
pub use theme::{CodexTheme, ThemePalette};
pub use transcript::transcript_lines;

use crate::{capability::TerminalCapabilities, state::WrappedLine};

pub fn line_to_ratatui(line: &WrappedLine, capabilities: TerminalCapabilities) -> Line<'static> {
    let theme = CodexTheme::new(capabilities);
    Line::from(Span::styled(
        line.content.clone(),
        theme.line_style(line.style),
    ))
}
