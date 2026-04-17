use astrcode_client::AstrcodePhaseDto;

use super::{
    ThemePalette,
    cells::{RenderableCell, TranscriptCellView, synthetic_thinking_lines},
};
use crate::state::{CliState, TranscriptCellStatus, WrappedLine, WrappedLineStyle};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptRenderOutput {
    pub lines: Vec<WrappedLine>,
    pub selected_line_range: Option<(usize, usize)>,
}

pub fn transcript_lines(
    state: &CliState,
    width: u16,
    theme: &dyn ThemePalette,
) -> TranscriptRenderOutput {
    let width = usize::from(width.max(28));
    let mut lines = Vec::new();
    let mut selected_line_range = None;
    let transcript_cells = state.transcript_cells();
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

    lines.extend(super::hero_lines(state, width as u16, theme));

    for (index, cell) in transcript_cells.iter().enumerate() {
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
        let rendered = cell.render_lines(width, state.shell.capabilities, theme, &view);
        lines.extend(rendered);
        if view.selected {
            let line_end = lines.len().saturating_sub(1);
            selected_line_range = Some((line_start, line_end));
        }
    }

    if should_render_synthetic_thinking(state) {
        let presentation = state.thinking_playback.present(
            &state.thinking_pool,
            state
                .conversation
                .control
                .as_ref()
                .and_then(|control| control.active_turn_id.as_deref())
                .unwrap_or("active-thinking"),
            "",
            TranscriptCellStatus::Streaming,
            false,
        );
        lines.extend(synthetic_thinking_lines(theme, &presentation));
    }

    TranscriptRenderOutput {
        lines,
        selected_line_range,
    }
}

fn should_render_synthetic_thinking(state: &CliState) -> bool {
    let Some(control) = &state.conversation.control else {
        return false;
    };
    if control.active_turn_id.is_none() {
        return false;
    }
    if !matches!(
        control.phase,
        AstrcodePhaseDto::Thinking | AstrcodePhaseDto::CallingTool | AstrcodePhaseDto::Streaming
    ) {
        return false;
    }

    !state
        .transcript_cells()
        .iter()
        .any(|cell| match &cell.kind {
            crate::state::TranscriptCellKind::Thinking { status, .. } => {
                matches!(
                    status,
                    TranscriptCellStatus::Streaming | TranscriptCellStatus::Complete
                )
            },
            crate::state::TranscriptCellKind::Assistant { status, body } => {
                matches!(status, TranscriptCellStatus::Streaming) && !body.trim().is_empty()
            },
            crate::state::TranscriptCellKind::ToolCall { status, .. } => {
                matches!(status, TranscriptCellStatus::Streaming)
            },
            _ => false,
        })
}
