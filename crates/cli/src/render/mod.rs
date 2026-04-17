use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    widgets::{Block, Clear, Paragraph, Wrap},
};

use crate::{
    state::{CliState, PaneFocus},
    ui::{self, CodexTheme, ThemePalette},
};

const FOOTER_HEIGHT: u16 = 5;

pub fn render(frame: &mut Frame<'_>, state: &mut CliState) {
    state.set_viewport_size(frame.area().width, frame.area().height);
    let theme = CodexTheme::new(state.shell.capabilities);
    frame.render_widget(Block::default().style(theme.app_background()), frame.area());

    let footer_area = Rect {
        x: frame.area().x,
        y: frame.area().bottom().saturating_sub(FOOTER_HEIGHT),
        width: frame.area().width,
        height: FOOTER_HEIGHT,
    };
    let transcript_height = frame.area().height.saturating_sub(footer_area.height);
    let transcript_area = Rect {
        x: frame.area().x,
        y: frame.area().y,
        width: frame.area().width,
        height: transcript_height,
    };

    refresh_caches(state, transcript_area, footer_area, &theme);
    render_transcript(frame, state, transcript_area, &theme);
    render_footer(frame, state, footer_area, &theme);

    if ui::palette_visible(&state.interaction.palette) {
        render_palette(frame, state, transcript_area, footer_area, &theme);
    }
}

fn render_transcript(frame: &mut Frame<'_>, state: &mut CliState, area: Rect, theme: &CodexTheme) {
    let transcript = &state.render.transcript_cache;
    let viewport_height = area.height;
    let scroll = transcript_scroll_offset(
        transcript.lines.len(),
        viewport_height,
        state.interaction.scroll_anchor,
        state.interaction.follow_transcript_tail,
        transcript.selected_line_range,
        matches!(state.interaction.pane_focus, PaneFocus::Transcript)
            && state.interaction.selection_drives_scroll,
    );
    frame.render_widget(
        Paragraph::new(
            transcript
                .lines
                .iter()
                .map(|line| ui::line_to_ratatui(line, theme))
                .collect::<Vec<_>>(),
        )
        .scroll((scroll, 0)),
        area,
    );
}

fn render_footer(frame: &mut Frame<'_>, state: &CliState, area: Rect, theme: &CodexTheme) {
    let footer = &state.render.footer_cache;
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

    frame.render_widget(
        Paragraph::new(vec![ui::line_to_ratatui(&footer.lines[0], theme)]),
        layout[0],
    );
    frame.render_widget(
        Paragraph::new(theme.divider().repeat(usize::from(area.width)))
            .style(theme.line_style(crate::state::WrappedLineStyle::Divider)),
        layout[1],
    );
    frame.render_widget(
        Paragraph::new(vec![ui::line_to_ratatui(&footer.lines[1], theme)]),
        layout[2],
    );
    frame.render_widget(
        Paragraph::new(theme.divider().repeat(usize::from(area.width)))
            .style(theme.line_style(crate::state::WrappedLineStyle::Divider)),
        layout[3],
    );
    frame.render_widget(
        Paragraph::new(vec![ui::line_to_ratatui(&footer.lines[2], theme)]),
        layout[4],
    );

    if matches!(
        state.interaction.pane_focus,
        PaneFocus::Composer | PaneFocus::Palette
    ) {
        frame.set_cursor_position((area.x.saturating_add(footer.cursor_col), layout[2].y));
    }
}

fn render_palette(
    frame: &mut Frame<'_>,
    state: &CliState,
    transcript_area: Rect,
    footer_area: Rect,
    theme: &CodexTheme,
) {
    let menu_lines = &state.render.palette_cache.lines;
    let menu_height = menu_lines.len().clamp(1, 5) as u16;
    let menu_width = footer_area.width.saturating_sub(2);
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
                .map(|line| ui::line_to_ratatui(line, theme))
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

pub fn refresh_caches(
    state: &mut CliState,
    transcript_area: Rect,
    footer_area: Rect,
    theme: &CodexTheme,
) {
    let transcript_width = transcript_area.width.saturating_sub(2);
    let transcript_cache_valid = state.render.transcript_cache.width == transcript_width
        && state.render.transcript_cache.revision == state.render.transcript_revision
        && !state.render.transcript_cache.lines.is_empty();
    if state.render.dirty.transcript || !transcript_cache_valid {
        let transcript = ui::transcript_lines(state, transcript_width, theme);
        state.update_transcript_cache(
            transcript_width,
            transcript.lines,
            transcript.selected_line_range,
        );
    }

    let footer_width = footer_area.width.saturating_sub(2);
    let footer_cache_valid = state.render.footer_cache.width == footer_width
        && state.render.footer_cache.lines.len() == 3;
    if state.render.dirty.footer || !footer_cache_valid {
        let footer = ui::footer_lines(state, footer_width, theme);
        state
            .render
            .update_footer_cache(footer_width, footer.lines, footer.cursor_col);
    }

    let palette_width = footer_area.width.saturating_sub(2);
    let palette_should_show = ui::palette_visible(&state.interaction.palette);
    if !palette_should_show {
        if !state.render.palette_cache.lines.is_empty() {
            state.render.update_palette_cache(palette_width, Vec::new());
        }
        state.render.dirty.palette = false;
        return;
    }

    let palette_cache_valid = state.render.palette_cache.width == palette_width;
    if state.render.dirty.palette || !palette_cache_valid {
        let menu_lines = ui::palette_lines(
            &state.interaction.palette,
            usize::from(footer_area.width.saturating_sub(4)),
            theme,
        );
        state.render.update_palette_cache(palette_width, menu_lines);
    }
}

#[cfg(test)]
mod tests {
    use astrcode_client::{
        AstrcodeConversationControlStateDto, AstrcodeConversationCursorDto,
        AstrcodeConversationSlashActionKindDto, AstrcodeConversationSlashCandidateDto,
    };
    use ratatui::{Terminal, backend::TestBackend};

    use super::render;
    use crate::{
        capability::{ColorLevel, GlyphMode, TerminalCapabilities},
        state::{CliState, PaneFocus},
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
        state.conversation.control = Some(AstrcodeConversationControlStateDto {
            phase: astrcode_client::AstrcodePhaseDto::Thinking,
            can_submit_prompt: true,
            can_request_compact: true,
            compact_pending: false,
            compacting: false,
            active_turn_id: Some("turn-1".to_string()),
            last_compact_meta: None,
        });
        state.conversation.cursor = Some(AstrcodeConversationCursorDto("1.0".to_string()));
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
        assert!(text.contains("Ctrl+O"));
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

    #[test]
    fn transcript_render_uses_prewrapped_cache_without_extra_paragraph_wrapping() {
        use crate::state::{WrappedLine, WrappedLineStyle};

        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut state = CliState::new(
            "http://127.0.0.1:5529".to_string(),
            None,
            capabilities(GlyphMode::Unicode),
        );
        state.set_viewport_size(40, 10);
        state.update_transcript_cache(
            38,
            vec![
                WrappedLine {
                    style: WrappedLineStyle::Plain,
                    content: "这一行故意非常非常非常长，只有在 Paragraph 再次 wrap \
                              时才会额外占用多行。"
                        .to_string(),
                },
                WrappedLine {
                    style: WrappedLineStyle::Plain,
                    content: "第二行".to_string(),
                },
                WrappedLine {
                    style: WrappedLineStyle::Plain,
                    content: "第三行".to_string(),
                },
                WrappedLine {
                    style: WrappedLineStyle::Plain,
                    content: "第四行".to_string(),
                },
                WrappedLine {
                    style: WrappedLineStyle::Plain,
                    content: "目标尾行".to_string(),
                },
            ],
            None,
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
        let buffer = terminal.backend().buffer();
        let rendered_rows = (0..buffer.area.height)
            .map(|row| {
                let start = usize::from(row) * usize::from(buffer.area.width);
                let end = start + usize::from(buffer.area.width);
                buffer.content[start..end]
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect::<String>()
                    .replace(' ', "")
            })
            .collect::<Vec<_>>();

        assert!(
            rendered_rows
                .first()
                .is_some_and(|row| row.starts_with("这一行故意非常非常非常长")),
            "the first cached line should remain visible when the transcript exactly fits"
        );
        assert!(
            rendered_rows.iter().any(|row| row.contains("目标尾行")),
            "tail-follow transcript should keep the final cached line visible"
        );
        assert!(text.contains("第"));
    }
}
