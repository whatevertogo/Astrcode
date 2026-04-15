use ratatui::{
    Frame,
    layout::{Constraint, Direction, Flex, Layout, Rect},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::{
    state::CliState,
    ui::{self, BottomPaneView, CodexTheme, ComposerPane, ThemePalette},
};

pub fn render(frame: &mut Frame<'_>, state: &mut CliState) {
    state.set_viewport_size(frame.area().width, frame.area().height);

    let header_height = ui::header_lines(state, frame.area().width).len() as u16;
    let bottom_height = ComposerPane::new(state)
        .desired_height(frame.area().width)
        .clamp(
            5,
            frame.area().height.saturating_sub(header_height + 2).max(5),
        );

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_height),
            Constraint::Min(1),
            Constraint::Length(bottom_height),
        ])
        .split(frame.area());

    let theme = CodexTheme::new(state.shell.capabilities);

    frame.render_widget(
        Paragraph::new(
            ui::header_lines(state, layout[0].width)
                .iter()
                .map(|line| ui::line_to_ratatui(line, state.shell.capabilities))
                .collect::<Vec<_>>(),
        )
        .wrap(Wrap { trim: false }),
        layout[0],
    );

    let side_pane_visible =
        !state.conversation.child_summaries.is_empty() && layout[1].width >= 105;
    let body = if side_pane_visible {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(1),
                Constraint::Length(1),
                Constraint::Length(30),
            ])
            .split(layout[1])
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(1)])
            .split(layout[1])
    };

    render_transcript(frame, state, body[0]);
    if side_pane_visible {
        render_vertical_divider(frame, body[1], &theme);
        render_side_pane(frame, state, body[2]);
    }

    render_bottom_pane(frame, state, layout[2], &ComposerPane::new(state), &theme);

    if let Some(title) = ui::overlay_title(&state.interaction.overlay) {
        let overlay_lines =
            ui::centered_overlay_lines(state, frame.area().width.saturating_sub(20));
        let overlay_height = overlay_lines.len().saturating_add(2).clamp(6, 22) as u16;
        let overlay_area = centered_rect(72, overlay_height, frame.area());
        frame.render_widget(Clear, overlay_area);
        frame.render_widget(
            Paragraph::new(
                overlay_lines
                    .iter()
                    .map(|line| ui::line_to_ratatui(line, state.shell.capabilities))
                    .collect::<Vec<_>>(),
            )
            .block(
                Block::default()
                    .title(title)
                    .borders(Borders::ALL)
                    .border_style(theme.overlay_border_style()),
            )
            .wrap(Wrap { trim: false }),
            overlay_area,
        );
    }
}

fn render_transcript(frame: &mut Frame<'_>, state: &mut CliState, area: Rect) {
    let transcript_width = area.width;
    let transcript_lines = if state.render.transcript_cache.width == transcript_width
        && state.render.transcript_cache.revision == state.render.transcript_revision
    {
        state.render.transcript_cache.lines.clone()
    } else {
        let lines = ui::transcript_lines(state, transcript_width);
        state.update_transcript_cache(transcript_width, lines.clone());
        lines
    };

    let scroll = transcript_scroll_offset(
        transcript_lines.len(),
        area.height,
        state.interaction.scroll_anchor,
        state.interaction.follow_transcript_tail,
    );

    frame.render_widget(
        Paragraph::new(
            transcript_lines
                .iter()
                .map(|line| ui::line_to_ratatui(line, state.shell.capabilities))
                .collect::<Vec<_>>(),
        )
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0)),
        area,
    );
}

fn render_side_pane(frame: &mut Frame<'_>, state: &CliState, area: Rect) {
    frame.render_widget(
        Paragraph::new(
            ui::child_pane_lines(state, area.width)
                .iter()
                .map(|line| ui::line_to_ratatui(line, state.shell.capabilities))
                .collect::<Vec<_>>(),
        )
        .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_bottom_pane(
    frame: &mut Frame<'_>,
    state: &CliState,
    area: Rect,
    composer: &ComposerPane<'_>,
    theme: &CodexTheme,
) {
    frame.render_widget(
        Paragraph::new(
            composer
                .lines(area.width)
                .iter()
                .map(|line| ui::line_to_ratatui(line, state.shell.capabilities))
                .collect::<Vec<_>>(),
        )
        .wrap(Wrap { trim: false })
        .style(theme.muted_block_style()),
        area,
    );
}

fn render_vertical_divider(frame: &mut Frame<'_>, area: Rect, theme: &CodexTheme) {
    let line = theme
        .vertical_divider()
        .repeat(usize::from(area.height.max(1)));
    let divider = line
        .chars()
        .map(|glyph| glyph.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    frame.render_widget(
        Paragraph::new(divider).style(theme.line_style(crate::state::WrappedLineStyle::Border)),
        area,
    );
}

fn transcript_scroll_offset(
    total_lines: usize,
    viewport_height: u16,
    anchor_from_bottom: u16,
    follow_tail: bool,
) -> u16 {
    let max_scroll = total_lines.saturating_sub(usize::from(viewport_height));
    let top_offset = if follow_tail {
        max_scroll
    } else {
        max_scroll.saturating_sub(usize::from(anchor_from_bottom))
    };
    top_offset.try_into().unwrap_or(u16::MAX)
}

fn centered_rect(width_percent: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(area.height.saturating_sub(height) / 2),
            Constraint::Length(height),
            Constraint::Min(0),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(width_percent)])
        .flex(Flex::Center)
        .split(vertical[1])[0]
}

#[cfg(test)]
mod tests {
    use astrcode_client::{
        AstrcodeConversationAgentLifecycleDto, AstrcodeConversationSlashActionKindDto,
        AstrcodeConversationSlashCandidateDto,
    };
    use ratatui::{Terminal, backend::TestBackend};

    use super::render;
    use crate::{
        capability::{ColorLevel, GlyphMode, TerminalCapabilities},
        state::CliState,
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
    fn renders_workspace_scaffold() {
        let backend = TestBackend::new(100, 30);
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
        assert!(text.contains("Astrcode workspace"));
        assert!(text.contains("Find and fix a bug in @filename"));
    }

    #[test]
    fn renders_ascii_dividers_in_ascii_mode() {
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
        assert!(text.contains("-"));
        assert!(text.contains(">"));
    }

    #[test]
    fn renders_child_agents_side_pane() {
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut state = CliState::new(
            "http://127.0.0.1:5529".to_string(),
            None,
            capabilities(GlyphMode::Unicode),
        );
        state.conversation.child_summaries.push(
            astrcode_client::AstrcodeConversationChildSummaryDto {
                child_session_id: "child-1".to_string(),
                child_agent_id: "agent-1".to_string(),
                title: "Repo inspector".to_string(),
                lifecycle: AstrcodeConversationAgentLifecycleDto::Running,
                latest_output_summary: Some("checking repository layout".to_string()),
                child_ref: None,
            },
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
        assert!(text.contains("child sessions"));
        assert!(text.contains("Repo inspector"));
    }

    #[test]
    fn renders_embedded_command_palette_in_bottom_pane() {
        let backend = TestBackend::new(100, 30);
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
        assert!(text.contains("commands"));
        assert!(text.contains("Review current changes"));
    }

    #[test]
    fn transcript_scroll_offset_pins_tail_when_follow_is_enabled() {
        assert_eq!(super::transcript_scroll_offset(48, 10, 3, true), 38);
    }

    #[test]
    fn transcript_scroll_offset_uses_anchor_when_follow_is_disabled() {
        assert_eq!(super::transcript_scroll_offset(48, 10, 3, false), 35);
    }
}
