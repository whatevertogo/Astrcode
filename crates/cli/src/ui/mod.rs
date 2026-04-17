pub mod cells;
mod footer;
mod hero;
mod palette;
mod text;
mod theme;
mod transcript;

pub use footer::{FooterRenderOutput, footer_lines};
pub use hero::hero_lines;
pub use palette::{palette_lines, palette_visible};
use ratatui::text::{Line, Span};
pub use text::truncate_to_width;
pub use theme::{CodexTheme, ThemePalette};
pub use transcript::transcript_lines;

use crate::state::WrappedLine;

pub fn line_to_ratatui(line: &WrappedLine, theme: &CodexTheme) -> Line<'static> {
    Line::from(Span::styled(
        line.content.clone(),
        theme.line_style(line.style),
    ))
}
