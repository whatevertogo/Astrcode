use super::{
    cells::wrap_text,
    theme::{CodexTheme, ThemePalette},
};
use crate::{
    capability::TerminalCapabilities,
    state::{CliState, OverlayState, PaneFocus, WrappedLine, WrappedLineStyle},
};

pub trait BottomPaneView {
    fn desired_height(&self, width: u16) -> u16;
    fn lines(&self, width: u16) -> Vec<WrappedLine>;
}

pub struct ComposerPane<'a> {
    state: &'a CliState,
    theme: CodexTheme,
}

impl<'a> ComposerPane<'a> {
    pub fn new(state: &'a CliState) -> Self {
        Self {
            state,
            theme: CodexTheme::new(state.shell.capabilities),
        }
    }

    fn status_row(&self, width: usize) -> WrappedLine {
        let phase = self
            .state
            .active_phase()
            .map(super::phase_label)
            .unwrap_or("idle");
        let busy = if self.state.stream_view.pending_chunks > 0 {
            format!(
                "stream {:?} / {}",
                self.state.stream_view.mode, self.state.stream_view.pending_chunks
            )
        } else {
            "ready".to_string()
        };
        let mut segments = vec![format!("{} {phase}", self.theme.glyph("●", "*")), busy];
        if let Some(banner) = &self.state.conversation.banner {
            segments.push(format!("error {}", banner.error.message));
        } else if !self.state.interaction.status.message.is_empty() {
            segments.push(self.state.interaction.status.message.clone());
        }

        WrappedLine {
            style: if self.state.conversation.banner.is_some()
                || self.state.interaction.status.is_error
            {
                WrappedLineStyle::Error
            } else {
                WrappedLineStyle::Dim
            },
            content: truncate_text(
                segments.join("  ·  ").as_str(),
                width,
                self.state.shell.capabilities,
            ),
        }
    }

    fn composer_lines(&self, width: usize) -> Vec<WrappedLine> {
        let prefix = self.theme.glyph("›", ">");
        let draft = if self.state.interaction.composer.is_empty() {
            "Find and fix a bug in @filename".to_string()
        } else {
            self.state.interaction.composer.input.clone()
        };
        let body_style = if self.state.interaction.composer.is_empty() {
            WrappedLineStyle::Dim
        } else if matches!(self.state.interaction.pane_focus, PaneFocus::Composer) {
            WrappedLineStyle::Accent
        } else {
            WrappedLineStyle::Plain
        };

        wrap_text(
            draft.as_str(),
            width.saturating_sub(2).max(8),
            self.state.shell.capabilities,
        )
        .into_iter()
        .enumerate()
        .map(|(index, line)| WrappedLine {
            style: body_style,
            content: if index == 0 {
                format!("{prefix} {line}")
            } else {
                format!("  {line}")
            },
        })
        .collect()
    }

    fn slash_popup_lines(
        &self,
        width: usize,
        capabilities: TerminalCapabilities,
    ) -> Vec<WrappedLine> {
        let OverlayState::SlashPalette(palette) = &self.state.interaction.overlay else {
            return Vec::new();
        };

        let mut lines = vec![
            WrappedLine {
                style: WrappedLineStyle::Border,
                content: self.theme.divider().repeat(width.max(16)),
            },
            WrappedLine {
                style: WrappedLineStyle::Warning,
                content: format!(
                    "{} commands  {}",
                    self.theme.glyph("↳", ">"),
                    if palette.query.is_empty() {
                        "<all>"
                    } else {
                        palette.query.as_str()
                    }
                ),
            },
        ];

        if palette.items.is_empty() {
            lines.push(WrappedLine {
                style: WrappedLineStyle::Dim,
                content: "  没有匹配的 commands 或 skills。".to_string(),
            });
            return lines;
        }

        for (index, item) in palette.items.iter().take(6).enumerate() {
            let marker = if index == palette.selected {
                self.theme.glyph("›", ">")
            } else {
                " "
            };
            lines.push(WrappedLine {
                style: if index == palette.selected {
                    WrappedLineStyle::Selection
                } else {
                    WrappedLineStyle::Plain
                },
                content: format!("{marker} {}", item.title),
            });
            for line in wrap_text(
                item.description.as_str(),
                width.saturating_sub(2).max(8),
                capabilities,
            ) {
                lines.push(WrappedLine {
                    style: WrappedLineStyle::Dim,
                    content: format!("  {line}"),
                });
            }
        }
        lines
    }

    fn footer_line(&self, width: usize) -> WrappedLine {
        let focus = match self.state.interaction.pane_focus {
            PaneFocus::Transcript => "transcript",
            PaneFocus::ChildPane => "children",
            PaneFocus::Composer => "composer",
            PaneFocus::Overlay => "overlay",
        };
        let parts = [
            format!("focus {focus}"),
            "Enter send".to_string(),
            "Shift+Enter newline".to_string(),
            "Tab commands".to_string(),
            "F2 logs".to_string(),
        ];
        WrappedLine {
            style: WrappedLineStyle::Footer,
            content: truncate_text(
                parts.join("  ·  ").as_str(),
                width,
                self.state.shell.capabilities,
            ),
        }
    }
}

impl BottomPaneView for ComposerPane<'_> {
    fn desired_height(&self, width: u16) -> u16 {
        self.lines(width).len().try_into().unwrap_or(u16::MAX)
    }

    fn lines(&self, width: u16) -> Vec<WrappedLine> {
        let width = usize::from(width.max(24));
        let mut lines = vec![self.status_row(width)];
        lines.push(WrappedLine {
            style: WrappedLineStyle::Border,
            content: self.theme.divider().repeat(width),
        });
        lines.extend(self.composer_lines(width));
        lines.extend(self.slash_popup_lines(width, self.state.shell.capabilities));
        lines.push(WrappedLine {
            style: WrappedLineStyle::Border,
            content: self.theme.divider().repeat(width),
        });
        lines.push(self.footer_line(width));
        lines
    }
}

fn truncate_text(text: &str, width: usize, capabilities: TerminalCapabilities) -> String {
    if width == 0 {
        return String::new();
    }

    let mut out = String::new();
    let mut current_width = 0;
    for ch in text.chars() {
        let ch = ch.to_string();
        let ch_width = if capabilities.ascii_only() {
            1
        } else {
            unicode_width::UnicodeWidthStr::width(ch.as_str()).max(1)
        };
        if current_width + ch_width > width {
            break;
        }
        current_width += ch_width;
        out.push_str(ch.as_str());
    }
    out
}
