use super::{cells::wrap_text, theme::ThemePalette};
use crate::{
    capability::TerminalCapabilities,
    state::{
        DebugChannelState, DebugOverlayState, OverlayState, ResumeOverlayState, WrappedLine,
        WrappedLineStyle,
    },
};

pub trait OverlayView {
    fn title(&self) -> &'static str;
    fn lines(
        &self,
        width: usize,
        capabilities: TerminalCapabilities,
        theme: &dyn ThemePalette,
    ) -> Vec<WrappedLine>;
}

impl OverlayView for ResumeOverlayState {
    fn title(&self) -> &'static str {
        "Resume Session"
    }

    fn lines(
        &self,
        width: usize,
        capabilities: TerminalCapabilities,
        theme: &dyn ThemePalette,
    ) -> Vec<WrappedLine> {
        let mut lines = vec![WrappedLine {
            style: WrappedLineStyle::Dim,
            content: format!(
                "{} query {}",
                theme.glyph("·", "-"),
                if self.query.is_empty() {
                    "<all>"
                } else {
                    self.query.as_str()
                }
            ),
        }];
        if self.items.is_empty() {
            lines.push(WrappedLine {
                style: WrappedLineStyle::Dim,
                content: "没有匹配的会话。".to_string(),
            });
            return lines;
        }

        for (index, item) in self.items.iter().take(10).enumerate() {
            lines.push(WrappedLine {
                style: if index == self.selected {
                    WrappedLineStyle::Selection
                } else {
                    WrappedLineStyle::Plain
                },
                content: format!(
                    "{} {}",
                    if index == self.selected {
                        theme.glyph("›", ">")
                    } else {
                        " "
                    },
                    item.title
                ),
            });
            for line in wrap_text(
                item.working_dir.as_str(),
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
}

impl DebugOverlayState {
    pub fn lines(
        &self,
        width: usize,
        capabilities: TerminalCapabilities,
        theme: &dyn ThemePalette,
        debug: &DebugChannelState,
    ) -> Vec<WrappedLine> {
        let mut lines = vec![
            WrappedLine {
                style: WrappedLineStyle::Dim,
                content: format!(
                    "{} launcher/server debug output  ·  Esc close",
                    theme.glyph("·", "-")
                ),
            },
            WrappedLine {
                style: WrappedLineStyle::Border,
                content: theme.divider().repeat(width.max(16)),
            },
        ];

        if debug.is_empty() {
            lines.push(WrappedLine {
                style: WrappedLineStyle::Dim,
                content: "暂无 debug logs。".to_string(),
            });
            return lines;
        }

        let entries = debug
            .entries()
            .rev()
            .skip(self.scroll)
            .take(18)
            .collect::<Vec<_>>();
        for entry in entries.into_iter().rev() {
            for line in wrap_text(entry, width.saturating_sub(2).max(8), capabilities) {
                lines.push(WrappedLine {
                    style: WrappedLineStyle::Plain,
                    content: line,
                });
            }
        }
        lines
    }
}

pub fn overlay_title(overlay: &OverlayState) -> Option<&'static str> {
    match overlay {
        OverlayState::None | OverlayState::SlashPalette(_) => None,
        OverlayState::Resume(state) => Some(state.title()),
        OverlayState::DebugLogs(_) => Some("Debug Logs"),
    }
}
