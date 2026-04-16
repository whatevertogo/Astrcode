use super::{
    ThemePalette,
    cells::{RenderableCell, TranscriptCellView},
};
use crate::state::{CliState, WrappedLine, WrappedLineStyle};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptRenderOutput {
    pub lines: Vec<WrappedLine>,
    pub selected_line_range: Option<(usize, usize)>,
}

pub fn transcript_lines(state: &CliState, width: u16) -> TranscriptRenderOutput {
    let theme = super::CodexTheme::new(state.shell.capabilities);
    let width = usize::from(width.max(28));
    let mut lines = Vec::new();
    let mut selected_line_range = None;
    if let Some(banner) = &state.conversation.banner {
        lines.push(WrappedLine {
            style: WrappedLineStyle::ErrorText,
            content: format!("{} {}", theme.glyph("!", "!"), banner.error.message),
        });
        lines.push(WrappedLine {
            style: WrappedLineStyle::Muted,
            content: "  stream 需要重新同步，继续操作前建议等待恢复。".to_string(),
        });
        lines.push(WrappedLine {
            style: WrappedLineStyle::Plain,
            content: String::new(),
        });
    }
    if state.conversation.transcript_cells.is_empty() {
        lines.push(WrappedLine {
            style: WrappedLineStyle::Muted,
            content: format!("{} Astrcode workspace", theme.glyph("•", "*")),
        });
        lines.push(WrappedLine {
            style: WrappedLineStyle::Muted,
            content: "  输入消息开始，或输入 / commands。".to_string(),
        });
        lines.push(WrappedLine {
            style: WrappedLineStyle::Muted,
            content: "  Tab 切换 transcript / composer，Ctrl+O 展开 thinking。".to_string(),
        });
        return TranscriptRenderOutput {
            lines,
            selected_line_range: None,
        };
    }

    for (index, cell) in state.conversation.transcript_cells.iter().enumerate() {
        let line_start = lines.len();
        let view = TranscriptCellView {
            selected: matches!(
                state.interaction.pane_focus,
                crate::state::PaneFocus::Transcript
            ) && state.interaction.transcript.selected_cell == index,
            expanded: state.is_cell_expanded(cell.id.as_str()) || cell.expanded,
            thinking: match &cell.kind {
                crate::state::TranscriptCellKind::Thinking { body, status } => {
                    Some(state.thinking_playback.present(
                        &state.thinking_pool,
                        cell.id.as_str(),
                        body.as_str(),
                        *status,
                        state.is_cell_expanded(cell.id.as_str()) || cell.expanded,
                    ))
                },
                _ => None,
            },
        };
        let rendered = cell.render_lines(width, state.shell.capabilities, &theme, &view);
        lines.extend(rendered);
        if view.selected {
            let line_end = lines.len().saturating_sub(1);
            selected_line_range = Some((line_start, line_end));
        }
    }

    TranscriptRenderOutput {
        lines,
        selected_line_range,
    }
}
