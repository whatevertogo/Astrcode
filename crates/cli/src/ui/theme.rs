use ratatui::style::{Color, Modifier, Style};

use crate::{
    capability::{ColorLevel, TerminalCapabilities},
    state::WrappedLineStyle,
};

pub trait ThemePalette {
    fn line_style(&self, style: WrappedLineStyle) -> Style;
    fn glyph(&self, unicode: &'static str, ascii: &'static str) -> &'static str;
    fn divider(&self) -> &'static str;
    fn vertical_divider(&self) -> &'static str;
}

#[derive(Debug, Clone, Copy)]
pub struct CodexTheme {
    capabilities: TerminalCapabilities,
}

impl CodexTheme {
    pub fn new(capabilities: TerminalCapabilities) -> Self {
        Self { capabilities }
    }

    pub fn muted_block_style(self) -> Style {
        match self.capabilities.color {
            ColorLevel::TrueColor => Style::default().bg(Color::Rgb(26, 28, 33)),
            ColorLevel::Ansi16 => Style::default().bg(Color::Black),
            ColorLevel::None => Style::default(),
        }
    }

    pub fn overlay_border_style(self) -> Style {
        match self.capabilities.color {
            ColorLevel::TrueColor => Style::default().fg(Color::Rgb(90, 96, 110)),
            ColorLevel::Ansi16 => Style::default().fg(Color::DarkGray),
            ColorLevel::None => Style::default(),
        }
    }

    fn accent(self) -> Color {
        match self.capabilities.color {
            ColorLevel::TrueColor => Color::Rgb(72, 196, 255),
            _ => Color::Cyan,
        }
    }

    fn magenta(self) -> Color {
        match self.capabilities.color {
            ColorLevel::TrueColor => Color::Rgb(221, 183, 255),
            _ => Color::Magenta,
        }
    }

    fn dim(self) -> Color {
        match self.capabilities.color {
            ColorLevel::TrueColor => Color::Rgb(123, 129, 142),
            _ => Color::DarkGray,
        }
    }

    fn warning(self) -> Color {
        match self.capabilities.color {
            ColorLevel::TrueColor => Color::Rgb(245, 201, 104),
            _ => Color::Yellow,
        }
    }

    fn error(self) -> Color {
        match self.capabilities.color {
            ColorLevel::TrueColor => Color::Rgb(255, 122, 122),
            _ => Color::Red,
        }
    }

    fn success(self) -> Color {
        match self.capabilities.color {
            ColorLevel::TrueColor => Color::Rgb(121, 214, 121),
            _ => Color::Green,
        }
    }
}

impl ThemePalette for CodexTheme {
    fn line_style(&self, style: WrappedLineStyle) -> Style {
        let base = Style::default();
        if matches!(self.capabilities.color, ColorLevel::None) {
            return match style {
                WrappedLineStyle::Accent
                | WrappedLineStyle::Success
                | WrappedLineStyle::Warning
                | WrappedLineStyle::Error
                | WrappedLineStyle::Selection
                | WrappedLineStyle::Header => base.add_modifier(Modifier::BOLD),
                WrappedLineStyle::Dim | WrappedLineStyle::Footer | WrappedLineStyle::Border => {
                    base.add_modifier(Modifier::DIM)
                },
                WrappedLineStyle::User => base.add_modifier(Modifier::REVERSED),
                WrappedLineStyle::Plain => base,
            };
        }

        match style {
            WrappedLineStyle::Plain => base.fg(Color::White),
            WrappedLineStyle::Dim | WrappedLineStyle::Border => {
                base.fg(self.dim()).add_modifier(Modifier::DIM)
            },
            WrappedLineStyle::Footer => base.fg(self.dim()),
            WrappedLineStyle::Accent => base.fg(self.accent()).add_modifier(Modifier::BOLD),
            WrappedLineStyle::Success => base.fg(self.success()).add_modifier(Modifier::BOLD),
            WrappedLineStyle::Warning => base.fg(self.warning()),
            WrappedLineStyle::Error => base.fg(self.error()).add_modifier(Modifier::BOLD),
            WrappedLineStyle::User => base.fg(Color::White).bg(match self.capabilities.color {
                ColorLevel::TrueColor => Color::Rgb(35, 40, 52),
                _ => Color::Black,
            }),
            WrappedLineStyle::Selection => base
                .fg(Color::White)
                .bg(match self.capabilities.color {
                    ColorLevel::TrueColor => Color::Rgb(34, 74, 99),
                    _ => Color::DarkGray,
                })
                .add_modifier(Modifier::BOLD),
            WrappedLineStyle::Header => base.fg(self.magenta()).add_modifier(Modifier::BOLD),
        }
    }

    fn glyph(&self, unicode: &'static str, ascii: &'static str) -> &'static str {
        if self.capabilities.ascii_only() {
            ascii
        } else {
            unicode
        }
    }

    fn divider(&self) -> &'static str {
        self.glyph("─", "-")
    }

    fn vertical_divider(&self) -> &'static str {
        self.glyph("│", "|")
    }
}
