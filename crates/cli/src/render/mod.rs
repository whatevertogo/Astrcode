use ratatui::{
    Frame,
    layout::{Constraint, Direction, Flex, Layout, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
};

use crate::{
    capability::ColorLevel,
    state::{CliState, PaneFocus, WrappedLineStyle},
    ui,
};

pub fn render(frame: &mut Frame<'_>, state: &mut CliState) {
    state.set_viewport_size(frame.area().width, frame.area().height);
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(3),
        ])
        .split(frame.area());

    let header = Paragraph::new(format!("Astrcode Terminal | {}", state.connection_origin));
    frame.render_widget(header, layout[0]);

    let body = if state.child_summaries.is_empty() {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(1)])
            .split(layout[1])
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
            .split(layout[1])
    };

    let transcript_width = body[0].width.saturating_sub(2);
    let transcript_lines = if state.render.transcript_cache.width == transcript_width
        && state.render.transcript_cache.revision == state.render.transcript_revision
    {
        state.render.transcript_cache.lines.clone()
    } else {
        let lines = ui::transcript_lines(state, transcript_width);
        state.update_transcript_cache(transcript_width, lines.clone());
        lines
    };
    let transcript = Paragraph::new(
        transcript_lines
            .iter()
            .map(|line| ui::line_to_ratatui(line, state.capabilities))
            .collect::<Vec<_>>(),
    )
    .block(
        Block::default()
            .title(ui::pane_title(
                "Transcript",
                state.pane_focus,
                PaneFocus::Transcript,
            ))
            .borders(Borders::ALL)
            .border_style(focus_style(state, PaneFocus::Transcript)),
    )
    .wrap(Wrap { trim: false })
    .scroll((state.scroll_anchor, 0));
    frame.render_widget(transcript, body[0]);

    if body.len() > 1 {
        let child_items = ui::child_pane_lines(state)
            .into_iter()
            .map(|line| {
                ListItem::new(ui::line_to_ratatui(&line, state.capabilities)).style(
                    match line.style {
                        WrappedLineStyle::Error => Style::default().fg(Color::Red),
                        _ => Style::default(),
                    },
                )
            })
            .collect::<Vec<_>>();
        let child_pane = List::new(child_items).block(
            Block::default()
                .title(ui::pane_title(
                    "Children",
                    state.pane_focus,
                    PaneFocus::ChildPane,
                ))
                .borders(Borders::ALL)
                .border_style(focus_style(state, PaneFocus::ChildPane)),
        );
        frame.render_widget(child_pane, body[1]);
    }

    let composer = Paragraph::new(state.composer.input.as_str()).block(
        Block::default()
            .title(ui::pane_title(
                "Composer",
                state.pane_focus,
                PaneFocus::Composer,
            ))
            .borders(Borders::ALL)
            .border_style(focus_style(state, PaneFocus::Composer)),
    );
    frame.render_widget(composer, layout[2]);

    let status_style = if state.status.is_error {
        Style::default().fg(Color::Red)
    } else {
        Style::default()
    };
    let status_area = Rect {
        x: layout[2].x,
        y: layout[2].y.saturating_sub(1),
        width: layout[2].width,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(ui::status_line(state)).style(status_style),
        status_area,
    );

    if let Some(banner) = &state.banner {
        let banner_area = centered_rect(72, 5, frame.area());
        let banner_widget = Paragraph::new(banner.error.message.as_str())
            .block(
                Block::default()
                    .title("Connection")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Red)),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(Clear, banner_area);
        frame.render_widget(banner_widget, banner_area);
    }

    if let Some(title) = ui::overlay_title(state) {
        let overlay_area = centered_rect(75, 12, frame.area());
        let overlay = Paragraph::new(
            ui::overlay_lines(state)
                .iter()
                .map(|line| ui::line_to_ratatui(line, state.capabilities))
                .collect::<Vec<_>>(),
        )
        .block(Block::default().title(title).borders(Borders::ALL))
        .wrap(Wrap { trim: false });
        frame.render_widget(Clear, overlay_area);
        frame.render_widget(overlay, overlay_area);
    }
}

fn focus_style(state: &CliState, pane: PaneFocus) -> Style {
    if state.pane_focus != pane {
        return Style::default();
    }
    match state.capabilities.color {
        ColorLevel::None => Style::default(),
        ColorLevel::Ansi16 | ColorLevel::TrueColor => Style::default().fg(Color::Cyan),
    }
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
    fn renders_empty_state_and_composer() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut state = CliState::new(
            "http://127.0.0.1:5529".to_string(),
            None,
            capabilities(GlyphMode::Unicode),
        );

        terminal
            .draw(|frame| render(frame, &mut state))
            .expect("draw");
        let buffer = terminal.backend().buffer().clone();
        let text = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(text.contains("Transcript"));
        assert!(text.contains("Composer"));
        assert_eq!(state.render.transcript_cache.lines.len(), 1);
        assert!(
            state.render.transcript_cache.lines[0]
                .content
                .contains("暂无 transcript")
        );
    }

    #[test]
    fn renders_degrade_mode_in_ascii() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut state = CliState::new(
            "http://127.0.0.1:5529".to_string(),
            None,
            capabilities(GlyphMode::Ascii),
        );
        state.status.message = "ready".to_string();

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
        assert!(text.contains("ascii"));
    }

    #[test]
    fn renders_child_pane_title_when_children_exist() {
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut state = CliState::new(
            "http://127.0.0.1:5529".to_string(),
            None,
            capabilities(GlyphMode::Unicode),
        );
        state
            .child_summaries
            .push(astrcode_client::AstrcodeTerminalChildSummaryDto {
                child_session_id: "child-1".to_string(),
                child_agent_id: "agent-1".to_string(),
                title: "Repo inspector".to_string(),
                lifecycle: astrcode_client::AstrcodeConversationAgentLifecycleDto::Running,
                latest_output_summary: Some("checking repo".to_string()),
                child_ref: None,
            });
        state.pane_focus = PaneFocus::ChildPane;

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
        assert!(text.contains("Children"));
        assert!(text.contains("Repo inspector"));
    }
}
