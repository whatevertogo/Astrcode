use ratatui::style::{Color, Modifier, Style};

use crate::{
    capability::{ColorLevel, TerminalCapabilities},
    state::WrappedLineStyle,
};

pub trait ThemePalette {
    fn line_style(&self, style: WrappedLineStyle) -> Style;
    fn glyph(&self, unicode: &'static str, ascii: &'static str) -> &'static str;
    fn divider(&self) -> &'static str;
}

#[derive(Debug, Clone, Copy)]
pub struct CodexTheme {
    capabilities: TerminalCapabilities,
}

impl CodexTheme {
    pub fn new(capabilities: TerminalCapabilities) -> Self {
        Self { capabilities }
    }

    pub fn app_background(self) -> Style {
        Style::default().bg(self.bg())
    }

    pub fn menu_block_style(self) -> Style {
        Style::default().bg(self.surface()).fg(self.text_primary())
    }

    fn bg(self) -> Color {
        match self.capabilities.color {
            ColorLevel::TrueColor => Color::Rgb(26, 24, 22),
            ColorLevel::Ansi16 => Color::Black,
            ColorLevel::None => Color::Reset,
        }
    }

    fn surface(self) -> Color {
        match self.capabilities.color {
            ColorLevel::TrueColor => Color::Rgb(35, 32, 29),
            ColorLevel::Ansi16 => Color::DarkGray,
            ColorLevel::None => Color::Reset,
        }
    }

    fn surface_alt(self) -> Color {
        match self.capabilities.color {
            ColorLevel::TrueColor => Color::Rgb(48, 43, 37),
            ColorLevel::Ansi16 => Color::DarkGray,
            ColorLevel::None => Color::Reset,
        }
    }

    fn accent(self) -> Color {
        match self.capabilities.color {
            ColorLevel::TrueColor => Color::Rgb(224, 128, 82),
            _ => Color::Yellow,
        }
    }

    fn accent_soft(self) -> Color {
        match self.capabilities.color {
            ColorLevel::TrueColor => Color::Rgb(196, 124, 88),
            _ => Color::Yellow,
        }
    }

    fn thinking(self) -> Color {
        match self.capabilities.color {
            ColorLevel::TrueColor => Color::Rgb(241, 151, 104),
            _ => Color::Yellow,
        }
    }

    fn text_primary(self) -> Color {
        match self.capabilities.color {
            ColorLevel::TrueColor => Color::Rgb(237, 229, 219),
            _ => Color::White,
        }
    }

    fn text_secondary(self) -> Color {
        match self.capabilities.color {
            ColorLevel::TrueColor => Color::Rgb(186, 176, 163),
            _ => Color::Gray,
        }
    }

    fn text_muted(self) -> Color {
        match self.capabilities.color {
            ColorLevel::TrueColor => Color::Rgb(136, 126, 114),
            _ => Color::DarkGray,
        }
    }

    fn error(self) -> Color {
        match self.capabilities.color {
            ColorLevel::TrueColor => Color::Rgb(227, 111, 111),
            _ => Color::Red,
        }
    }
}

impl ThemePalette for CodexTheme {
    fn line_style(&self, style: WrappedLineStyle) -> Style {
        let base = Style::default();
        if matches!(self.capabilities.color, ColorLevel::None) {
            return match style {
                WrappedLineStyle::Selection
                | WrappedLineStyle::UserLabel
                | WrappedLineStyle::AssistantLabel
                | WrappedLineStyle::ToolLabel
                | WrappedLineStyle::FooterInput
                | WrappedLineStyle::PaletteTitle => base.add_modifier(Modifier::BOLD),
                WrappedLineStyle::ThinkingLabel => {
                    base.add_modifier(Modifier::BOLD | Modifier::ITALIC)
                },
                WrappedLineStyle::Muted
                | WrappedLineStyle::Divider
                | WrappedLineStyle::FooterStatus
                | WrappedLineStyle::PaletteMeta => base.add_modifier(Modifier::DIM),
                _ => base,
            };
        }

        match style {
            WrappedLineStyle::Plain => base.fg(self.text_primary()),
            WrappedLineStyle::Muted
            | WrappedLineStyle::Divider
            | WrappedLineStyle::FooterStatus
            | WrappedLineStyle::PaletteMeta => base.fg(self.text_muted()),
            WrappedLineStyle::Accent | WrappedLineStyle::PaletteTitle => {
                base.fg(self.accent()).add_modifier(Modifier::BOLD)
            },
            WrappedLineStyle::Selection => base
                .fg(self.text_primary())
                .bg(self.surface_alt())
                .add_modifier(Modifier::BOLD),
            WrappedLineStyle::UserLabel => base.fg(self.accent_soft()).add_modifier(Modifier::BOLD),
            WrappedLineStyle::UserBody => base.fg(self.text_primary()),
            WrappedLineStyle::AssistantLabel => {
                base.fg(self.text_secondary()).add_modifier(Modifier::BOLD)
            },
            WrappedLineStyle::AssistantBody => base.fg(self.text_primary()),
            WrappedLineStyle::ThinkingLabel => base
                .fg(self.thinking())
                .add_modifier(Modifier::ITALIC | Modifier::BOLD),
            WrappedLineStyle::ThinkingBody => base.fg(self.text_secondary()),
            WrappedLineStyle::ToolLabel => base.fg(self.accent_soft()).add_modifier(Modifier::BOLD),
            WrappedLineStyle::ToolBody => base.fg(self.text_secondary()),
            WrappedLineStyle::ErrorText => base.fg(self.error()).add_modifier(Modifier::BOLD),
            WrappedLineStyle::FooterInput => {
                base.fg(self.text_primary()).add_modifier(Modifier::BOLD)
            },
            WrappedLineStyle::PaletteItem => base.fg(self.text_primary()),
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
}
