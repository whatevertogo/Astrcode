use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    widgets::{Block, Clear, Paragraph, Wrap},
};

use crate::{
    state::{CliState, PaneFocus},
    ui::{self, CodexTheme, ThemePalette},
};

pub fn render(frame: &mut Frame<'_>, state: &mut CliState) {
    state.set_viewport_size(frame.area().width, frame.area().height);
    let theme = CodexTheme::new(state.shell.capabilities);
    frame.render_widget(Block::default().style(theme.app_background()), frame.area());

    let footer_area = Rect {
        x: frame.area().x,
        y: frame.area().bottom().saturating_sub(4),
        width: frame.area().width,
        height: 4,
    };
    let transcript_height = frame.area().height.saturating_sub(footer_area.height);
    let transcript_area = Rect {
        x: frame.area().x,
        y: frame.area().y,
        width: frame.area().width,
        height: transcript_height,
    };

    render_transcript(frame, state, transcript_area);
    render_footer(frame, state, footer_area);

    if ui::palette_visible(&state.interaction.palette) {
        render_palette(frame, state, transcript_area, footer_area, &theme);
    }
}

fn render_transcript(frame: &mut Frame<'_>, state: &CliState, area: Rect) {
    let transcript = ui::transcript_lines(state, area.width.saturating_sub(2));
    let viewport_height = area.height.saturating_sub(1);
    let scroll = transcript_scroll_offset(
        transcript.lines.len(),
        viewport_height,
        state.interaction.scroll_anchor,
        state.interaction.follow_transcript_tail,
        transcript.selected_line_range,
        matches!(state.interaction.pane_focus, PaneFocus::Transcript),
    );
    frame.render_widget(
        Paragraph::new(
            transcript
                .lines
                .iter()
                .map(|line| ui::line_to_ratatui(line, state.shell.capabilities))
                .collect::<Vec<_>>(),
        )
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0)),
        area,
    );
}

fn render_footer(frame: &mut Frame<'_>, state: &CliState, area: Rect) {
    let theme = CodexTheme::new(state.shell.capabilities);
    let lines = ui::footer_lines(state, area.width.saturating_sub(2));
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

    frame.render_widget(
        Paragraph::new(theme.divider().repeat(usize::from(area.width)))
            .style(theme.line_style(crate::state::WrappedLineStyle::Divider)),
        layout[0],
    );
    frame.render_widget(
        Paragraph::new(vec![ui::line_to_ratatui(
            &lines[0],
            state.shell.capabilities,
        )]),
        layout[1],
    );
    frame.render_widget(
        Paragraph::new(theme.divider().repeat(usize::from(area.width)))
            .style(theme.line_style(crate::state::WrappedLineStyle::Divider)),
        layout[2],
    );
    frame.render_widget(
        Paragraph::new(vec![ui::line_to_ratatui(
            &lines[1],
            state.shell.capabilities,
        )]),
        layout[3],
    );
}

fn render_palette(
    frame: &mut Frame<'_>,
    state: &CliState,
    transcript_area: Rect,
    footer_area: Rect,
    theme: &CodexTheme,
) {
    let menu_lines = ui::palette_lines(
        &state.interaction.palette,
        usize::from(footer_area.width.saturating_sub(4)),
        theme,
    );
    let menu_height = menu_lines.len().clamp(2, 10) as u16;
    let menu_width = footer_area.width.saturating_sub(2).min(112).max(52);
    let menu_area = Rect {
        x: transcript_area.x.saturating_add(1),
        y: footer_area
            .y
            .saturating_sub(menu_height)
            .max(transcript_area.y),
        width: menu_width,
        height: menu_height,
    };
    frame.render_widget(Clear, menu_area);
    frame.render_widget(
        Paragraph::new(
            menu_lines
                .iter()
                .map(|line| ui::line_to_ratatui(line, state.shell.capabilities))
                .collect::<Vec<_>>(),
        )
        .style(theme.menu_block_style())
        .wrap(Wrap { trim: false }),
        menu_area,
    );
}

fn transcript_scroll_offset(
    total_lines: usize,
    viewport_height: u16,
    anchor_from_bottom: u16,
    follow_tail: bool,
    selected_line_range: Option<(usize, usize)>,
    selection_drives_scroll: bool,
) -> u16 {
    let max_scroll = total_lines.saturating_sub(usize::from(viewport_height));
    let mut top_offset = if follow_tail {
        max_scroll
    } else {
        max_scroll.saturating_sub(usize::from(anchor_from_bottom))
    };
    if selection_drives_scroll {
        if let Some((selected_start, selected_end)) = selected_line_range {
            let viewport_height = usize::from(viewport_height);
            if selected_start < top_offset {
                top_offset = selected_start;
            } else if selected_end >= top_offset.saturating_add(viewport_height) {
                top_offset = selected_end
                    .saturating_add(1)
                    .saturating_sub(viewport_height);
            }
        }
    }
    top_offset = top_offset.min(max_scroll);
    top_offset.try_into().unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use astrcode_client::{
        AstrcodeConversationSlashActionKindDto, AstrcodeConversationSlashCandidateDto,
    };
    use ratatui::{Terminal, backend::TestBackend};

    use super::render;
    use crate::{
        capability::{ColorLevel, GlyphMode, TerminalCapabilities},
        state::{CliState, PaneFocus, TranscriptCell, TranscriptCellKind, TranscriptCellStatus},
    };

    fn capabilities(glyphs: GlyphMode) -> TerminalCapabilities {
        TerminalCapabilities {
            color: ColorLevel::Ansi16,
            glyphs,
            alt_screen: false,
            mouse: false,
            bracketed_paste: false,
        }
    }

    #[test]
    fn renders_minimal_layout() {
        let backend = TestBackend::new(100, 28);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut state = CliState::new(
            "http://127.0.0.1:5529".to_string(),
            None,
            capabilities(GlyphMode::Unicode),
        );

        terminal
            .draw(|frame| render(frame, &mut state))
            .expect("draw");
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Astrcode"));
        assert!(text.contains("commands"));
        assert!(!text.contains("Navigation"));
    }

    #[test]
    fn renders_ascii_fallback_symbols() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut state = CliState::new(
            "http://127.0.0.1:5529".to_string(),
            None,
            capabilities(GlyphMode::Ascii),
        );

        terminal
            .draw(|frame| render(frame, &mut state))
            .expect("draw");
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains(">"));
        assert!(text.contains("-"));
    }

    #[test]
    fn renders_inline_slash_menu() {
        let backend = TestBackend::new(110, 28);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut state = CliState::new(
            "http://127.0.0.1:5529".to_string(),
            None,
            capabilities(GlyphMode::Unicode),
        );
        state.set_slash_query(
            "review",
            vec![AstrcodeConversationSlashCandidateDto {
                id: "review".to_string(),
                title: "Review current changes".to_string(),
                description: "对当前工作区变更运行 review".to_string(),
                keywords: vec!["review".to_string()],
                action_kind: AstrcodeConversationSlashActionKindDto::ExecuteCommand,
                action_value: "/review".to_string(),
            }],
        );
        terminal
            .draw(|frame| render(frame, &mut state))
            .expect("draw");
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("/ commands"));
        assert!(text.contains("Review current changes"));
    }

    #[test]
    fn renders_thinking_cell_with_preview() {
        let backend = TestBackend::new(100, 28);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut state = CliState::new(
            "http://127.0.0.1:5529".to_string(),
            None,
            capabilities(GlyphMode::Unicode),
        );
        state.conversation.transcript_cells.push(TranscriptCell {
            id: "thinking-1".to_string(),
            expanded: false,
            kind: TranscriptCellKind::Thinking {
                body: "".to_string(),
                status: TranscriptCellStatus::Streaming,
            },
        });
        state.interaction.set_focus(PaneFocus::Transcript);

        terminal
            .draw(|frame| render(frame, &mut state))
            .expect("draw");
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Thinking"));
    }

    #[test]
    fn transcript_scroll_offset_keeps_selected_range_visible() {
        assert_eq!(
            super::transcript_scroll_offset(64, 10, 0, false, Some((20, 24)), true),
            20
        );
        assert_eq!(
            super::transcript_scroll_offset(64, 10, 0, false, Some((3, 5)), true),
            3
        );
    }
}
